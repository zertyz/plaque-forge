pub(crate) mod candidate;
pub(crate) mod extraction;
mod occlusion;
mod tracking;

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::{
    analysis::{
        ANALYSIS_SCHEMA_VERSION, Analysis, AnalysisManifest, AnalysisStatus, MOTION_FILE,
        OCCLUDER_DIR, SourceInfo,
    },
    cli::AnalyzeArgs,
    layers::{self, LayerInput},
    model::AnalysisConfidence,
    progress::ProgressReporter,
    refinement::{
        MotionRefinement, RefinementProvenance, find_refinement, layer_artifact_provenance,
        relative_reference, resolve_relative, selected_layer_artifacts, semantic_provenance,
    },
    video, workspace,
};

struct AnalysisRefinements {
    motion_track: Option<MotionRefinement>,
    layers: Vec<LayerInput>,
    provenance: Option<RefinementProvenance>,
}

impl AnalysisRefinements {
    fn has_dense_locked_track(&self, frame_count: usize) -> bool {
        self.motion_track
            .as_ref()
            .is_some_and(|track| track.is_dense_locked(frame_count))
    }
}

pub fn run(mut args: AnalyzeArgs) -> Result<()> {
    if !(0.0..=1.0).contains(&args.minimum_analysis_confidence) {
        bail!("--minimum-analysis-confidence must be between 0 and 1");
    }
    let mut progress = ProgressReporter::new(args.progress, args.progress_interval_ms);
    progress.start(1, 7, "Probe and validate source", None);
    if !args.input.is_file() {
        bail!(
            "input video does not exist or is not a file: {}",
            args.input.display()
        );
    }
    let info = video::probe(&args.ffprobe, &args.input)
        .with_context(|| format!("failed to probe input video {}", args.input.display()))?;
    if !info.constant_frame_rate {
        bail!(
            "variable-frame-rate input is outside the 0.3 source contract; transcode it to a constant frame rate before analysis"
        );
    }
    let source_sha256 = video::sha256(&args.input)
        .with_context(|| format!("failed to hash source {}", args.input.display()))?;
    let refinements = resolve_refinements(&mut args, &info, &source_sha256)?;
    progress.finish(format!(
        "{}x{}, {:.3} fps, {} frames",
        info.width, info.height, info.fps, info.frames
    ));

    let output = args
        .output
        .clone()
        .map(Ok)
        .unwrap_or_else(|| workspace::analysis_path(&args.input))?;
    if output.exists() {
        if !args.force {
            bail!(
                "analysis output already exists: {}\nhelp: use --force or render --reanalyze",
                output.display()
            );
        }
        if output.is_dir() {
            fs::remove_dir_all(&output)
                .with_context(|| format!("failed to remove old analysis {}", output.display()))?;
        } else {
            fs::remove_file(&output)
                .with_context(|| format!("failed to remove old output {}", output.display()))?;
        }
    }

    let partial = partial_path(&output);
    if partial.exists() {
        fs::remove_dir_all(&partial).with_context(|| {
            format!(
                "failed to remove stale partial analysis {}",
                partial.display()
            )
        })?;
    }
    fs::create_dir_all(&partial)
        .with_context(|| format!("failed to create partial analysis {}", partial.display()))?;

    let diagnostics = match &args.diagnostics {
        Some(path) => path.clone(),
        None => partial.join("diagnostics"),
    };
    fs::create_dir_all(&diagnostics).with_context(|| {
        format!(
            "failed to create diagnostics directory {}",
            diagnostics.display()
        )
    })?;

    let result = (|| -> Result<Analysis> {
        progress.start(2, 7, "Detect plaque", Some(args.candidate_samples));
        let candidate = candidate::detect(&args, &info, &diagnostics)
            .context("plaque candidate detection failed")?;
        progress.finish(format!(
            "frame {}, confidence {:.3}, rect {:.0},{:.0},{:.0},{:.0}",
            candidate.frame_index,
            candidate.confidence,
            candidate.rect.x,
            candidate.rect.y,
            candidate.rect.width,
            candidate.rect.height
        ));

        let mut track = if let Some(refinement_track) = &refinements.motion_track
            && refinement_track.is_dense_locked(info.frames)
        {
            tracking::load_dense_refinement(
                &args,
                &info,
                candidate.rect,
                refinement_track,
                &diagnostics,
                &mut progress,
            )
            .context("failed to load authoritative refined plaque track")?
        } else {
            let mut track = tracking::track(
                &args,
                &info,
                candidate.rect,
                candidate.frame_index,
                &diagnostics,
                &mut progress,
            )
            .context("adaptive scene and plaque tracking failed")?;
            if let Some(refinement_track) = &refinements.motion_track {
                tracking::apply_motion_refinement(&mut track, refinement_track, candidate.rect)
                    .context("failed to apply motion refinement constraints")?;
            }
            track
        };

        let mut extraction = extraction::recover(
            &args.ffmpeg,
            &args.input,
            &info,
            candidate.rect,
            &mut track.samples,
            &partial,
            &diagnostics,
            args.extraction_samples,
            args.local_refinement_radius,
            !refinements.has_dense_locked_track(info.frames),
            refinements.motion_track.as_ref(),
            track.reference_frame,
            args.tracking_inertia,
            track.loop_closed,
            &mut progress,
        )
        .context("canonical plaque structure analysis failed")?;

        progress.start(7, 7, "Analyze foreground occlusion", Some(info.frames));
        let mut occlusion = if args.disable_occlusion {
            occlusion::OcclusionResult {
                has_occluder: false,
                confidence: 0.80,
                mean_coverage: 0.0,
            }
        } else {
            occlusion::extract(
                &args.ffmpeg,
                &args.input,
                &info,
                candidate.rect,
                &mut track.samples,
                &extraction,
                &partial,
                &diagnostics,
                args.occlusion_sensitivity,
                track.loop_closed,
                refinements.motion_track.as_ref(),
                &mut progress,
            )
            .context("foreground occlusion extraction failed")?
        };
        let automatic_exclusions = partial.join(OCCLUDER_DIR);
        let combined_exclusions = partial.join("tracking-exclusions");
        let authoritative_foreground = layers::has_authored_foreground(&refinements.layers);
        let empty_exclusions = partial.join("no-automatic-exclusions");
        let automatic_tracking_exclusions = if authoritative_foreground {
            &empty_exclusions
        } else {
            &automatic_exclusions
        };
        let has_refinement_exclusions = layers::build_tracking_exclusions(
            &refinements.layers,
            automatic_tracking_exclusions,
            &combined_exclusions,
            info.width,
            info.height,
            info.frames,
        )
        .context("failed to combine refinement and automatic tracking exclusions")?;
        let tracking_exclusions = if has_refinement_exclusions {
            &combined_exclusions
        } else if authoritative_foreground {
            &empty_exclusions
        } else {
            &automatic_exclusions
        };
        let stabilized_frames = if !args.disable_occlusion
            && !refinements.has_dense_locked_track(info.frames)
            && (has_refinement_exclusions || (!authoritative_foreground && occlusion.has_occluder))
        {
            let mut refined = tracking::retrack_masked(
                &args,
                &info,
                candidate.rect,
                track.reference_frame,
                &diagnostics,
                &mut progress,
                tracking_exclusions,
            )
            .context("failed to retrack the plaque with foreground masks")?;
            if let Some(refinement_track) = &refinements.motion_track {
                tracking::apply_motion_refinement(&mut refined, refinement_track, candidate.rect)?;
            }
            for (sample, previous) in refined.samples.iter_mut().zip(&track.samples) {
                sample.occluder_coverage = previous.occluder_coverage;
            }
            track = tracking::select_masked_refinement(track, refined);
            let repaired = tracking::stabilize_occluded_intervals(
                &mut track.samples,
                candidate.rect,
                track.reference_frame,
            );
            if let Some(refinement_track) = &refinements.motion_track {
                tracking::reapply_locked_refinements(
                    &mut track.samples,
                    refinement_track,
                    candidate.rect,
                )?;
            }
            track
                .model_name
                .push_str(&format!("-occlusion-bridge-{repaired}"));
            extraction = extraction::recover(
                &args.ffmpeg,
                &args.input,
                &info,
                candidate.rect,
                &mut track.samples,
                &partial,
                &diagnostics,
                args.extraction_samples,
                args.local_refinement_radius,
                false,
                refinements.motion_track.as_ref(),
                track.reference_frame,
                args.tracking_inertia,
                track.loop_closed,
                &mut progress,
            )
            .context("failed to rebuild the plaque model after masked tracking")?;
            progress.start(7, 7, "Rebuild foreground occlusion", Some(info.frames));
            occlusion = occlusion::extract(
                &args.ffmpeg,
                &args.input,
                &info,
                candidate.rect,
                &mut track.samples,
                &extraction,
                &partial,
                &diagnostics,
                args.occlusion_sensitivity,
                track.loop_closed,
                refinements.motion_track.as_ref(),
                &mut progress,
            )
            .context("failed to rebuild foreground masks after masked tracking")?;
            repaired
        } else {
            0
        };
        if authoritative_foreground {
            occlusion.has_occluder = false;
            if automatic_exclusions.is_dir() {
                fs::remove_dir_all(&automatic_exclusions)?;
            }
        }
        if has_refinement_exclusions {
            fs::remove_dir_all(&combined_exclusions)?;
        }
        if args.disable_occlusion
            && let Some(refinement_track) = &refinements.motion_track
        {
            tracking::apply_visibility_refinements(&mut track.samples, refinement_track)
                .context("failed to apply refined plaque visibility constraints")?;
        }
        progress.finish(format!(
            "occluder {}, confidence {:.3}",
            if occlusion.has_occluder {
                "detected"
            } else {
                "not detected"
            },
            occlusion.confidence
        ));

        let packed_layers = layers::package(
            &refinements.layers,
            &partial,
            candidate.canonical_width,
            candidate.canonical_height,
            info.width,
            info.height,
            candidate.rect,
            &track.samples,
            &mut extraction.content_mask,
        )
        .context("failed to import refinement layer artifacts")?;

        let overall = geometric_mean(&[
            candidate.confidence,
            track.confidence,
            extraction.confidence,
            occlusion.confidence,
        ]);
        let component_gate_passed = candidate.confidence >= 0.60
            && track.confidence >= 0.50
            && extraction.confidence >= 0.65
            && extraction.structural_area >= 0.001
            && occlusion.confidence >= 0.55;
        let analysis_gate_passed =
            overall >= args.minimum_analysis_confidence && component_gate_passed;
        let recovery = remedies(
            &args,
            candidate.rect,
            candidate.confidence,
            track.confidence,
            extraction.confidence,
            extraction.structural_area,
            occlusion.confidence,
        );

        fs::write(
            partial.join("analysis-summary.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version": ANALYSIS_SCHEMA_VERSION,
                "source_contract": "text-free-plaque",
                "candidate_confidence": candidate.confidence,
                "motion_confidence": track.confidence,
                "structural_confidence": extraction.confidence,
                "content_cavity_area": extraction.cavity_area,
                "structural_area": extraction.structural_area,
                "occlusion_confidence": occlusion.confidence,
                "occluder_mean_coverage": occlusion.mean_coverage,
                "occlusion_stabilized_frames": stabilized_frames,
                "overall": overall,
                "minimum_analysis_confidence": args.minimum_analysis_confidence,
                "component_gate_passed": component_gate_passed,
                "analysis_gate_passed": analysis_gate_passed,
                "low_confidence_explicitly_accepted": !analysis_gate_passed && args.allow_low_confidence,
                "remedies": recovery
            }))?,
        )
        .with_context(|| format!("failed to write analysis summary in {}", partial.display()))?;
        fs::write(
            partial.join(MOTION_FILE),
            serde_json::to_vec_pretty(&track.samples)?,
        )
        .with_context(|| format!("failed to write diagnostic motion in {}", partial.display()))?;

        if !analysis_gate_passed && !args.allow_low_confidence {
            bail!(
                "analysis quality gate failed: detected plaque {:.0},{:.0},{:.0},{:.0}; overall {overall:.3} (minimum {:.3}), components candidate {:.3}, tracking {:.3}, extraction {:.3} (structural area {:.3}%), occlusion {:.3}. {}. Use --allow-low-confidence only to retain diagnostic output",
                candidate.rect.x,
                candidate.rect.y,
                candidate.rect.width,
                candidate.rect.height,
                args.minimum_analysis_confidence,
                candidate.confidence,
                track.confidence,
                extraction.confidence,
                extraction.structural_area * 100.0,
                occlusion.confidence,
                recovery.join(". ")
            );
        }

        let manifest = AnalysisManifest {
            schema_version: ANALYSIS_SCHEMA_VERSION,
            status: AnalysisStatus::Complete,
            source_is_text_free: true,
            analyzer_build: crate::build_info::SOURCE_FINGERPRINT.to_string(),
            source: SourceInfo {
                path: relative_reference(&output.join("manifest.toml"), &args.input)?,
                sha256: source_sha256.clone(),
                width: info.width,
                height: info.height,
                fps: info.fps,
                frames: info.frames,
                duration_seconds: info.duration_seconds,
            },
            reference_frame: track.reference_frame,
            canonical_width: candidate.canonical_width,
            canonical_height: candidate.canonical_height,
            source_plaque_rect: candidate.rect,
            motion_model: track.model_name,
            loop_closed: track.loop_closed,
            has_occluder: occlusion.has_occluder,
            layers: packed_layers,
            refinements: refinements.provenance.clone(),
            analysis_gate_passed,
            confidence: AnalysisConfidence {
                plaque_detection: candidate.confidence,
                motion: track.confidence,
                extraction: extraction.confidence,
                occlusion: occlusion.confidence,
                overall,
            },
        };

        Analysis::create(&partial, manifest, track.samples)
    })();

    let pack = match result {
        Ok(pack) => pack,
        Err(error) => {
            return Err(error.context(format!(
                "analysis failed; partial diagnostics were retained in {}",
                partial.display()
            )));
        }
    };

    drop(pack);
    fs::rename(&partial, &output).with_context(|| {
        format!(
            "analysis succeeded but could not be committed from {} to {}",
            partial.display(),
            output.display()
        )
    })?;
    let pack = Analysis::open(&output)?;
    println!("analysis: {}", pack.root.display());
    println!(
        "overall analysis confidence: {:.3}",
        pack.manifest.confidence.overall
    );
    Ok(())
}

fn resolve_refinements(
    args: &mut AnalyzeArgs,
    info: &video::VideoInfo,
    source_sha256: &str,
) -> Result<AnalysisRefinements> {
    let loaded = find_refinement(&args.input, args.refinement.as_deref())?;
    let mut identity = RefinementProvenance::default();
    let mut selected_id = None;
    let mut referenced_track = None;
    let mut layer_inputs = Vec::new();

    if let Some(loaded) = &loaded {
        let declared_source = resolve_relative(&loaded.path, &loaded.document.source);
        same_file(&declared_source, &args.input).with_context(|| {
            format!(
                "refinement {} declares source {}, not input {}",
                loaded.path.display(),
                declared_source.display(),
                args.input.display()
            )
        })?;
        let selected = loaded.document.select_plaque(args.plaque.as_deref())?;
        selected_id = Some(selected.id.clone());
        identity.manifest = Some(semantic_provenance(&loaded.path, &loaded.document)?);
        identity.plaque_id = Some(selected.id.clone());

        if args.plaque_hint.is_none()
            && let Some(bounds) = selected.bounds
        {
            args.plaque_hint = Some(bounds);
            args.plaque_frame = selected.reference_frame;
        }
        if let Some(frame) = selected.reference_frame
            && frame >= info.frames
        {
            bail!(
                "plaque {:?} reference_frame {} is outside the {}-frame source",
                selected.id,
                frame,
                info.frames
            );
        }
        for prompt in &selected.prompts {
            if prompt.frame >= info.frames {
                bail!(
                    "plaque {:?} prompt frame {} is outside the {}-frame source",
                    selected.id,
                    prompt.frame,
                    info.frames
                );
            }
        }
        for layer in loaded
            .document
            .layers
            .iter()
            .filter(|layer| layer.plaque == selected.id)
        {
            for prompt in &layer.prompts {
                if prompt.frame >= info.frames {
                    bail!(
                        "layer {:?} prompt frame {} is outside the {}-frame source",
                        layer.id,
                        prompt.frame,
                        info.frames
                    );
                }
            }
            if layer.artifact.is_none() && !layer.prompts.is_empty() {
                bail!(
                    "layer {:?} has segmentation prompts but no artifact; run segment first",
                    layer.id
                );
            }
        }
        for (refinement, path, artifact) in selected_layer_artifacts(loaded, &selected.id)? {
            identity
                .layer_artifacts
                .push(layer_artifact_provenance(&path, &artifact)?);
            layer_inputs.push(LayerInput {
                refinement,
                artifact_path: path,
                artifact,
            });
        }
        referenced_track = selected
            .motion_track
            .as_ref()
            .map(|path| resolve_relative(&loaded.path, path));
    } else if let Some(id) = &args.plaque {
        bail!("--plaque {id:?} requires a refinement manifest");
    }

    let motion_track = if let Some(path) = referenced_track {
        let track = MotionRefinement::load(&path)?;
        if let Some(selected_id) = &selected_id
            && track.plaque != *selected_id
        {
            bail!(
                "motion track describes plaque {:?}, but refinement selected {:?}",
                track.plaque,
                selected_id
            );
        }
        if let Some(expected) = &track.source_sha256
            && !expected.eq_ignore_ascii_case(source_sha256)
        {
            bail!(
                "motion track source hash does not match {}; export or review the track against this source",
                args.input.display()
            );
        }
        for keyframe in &track.keyframes {
            if keyframe.frame >= info.frames {
                bail!(
                    "motion keyframe {} is outside the {}-frame source",
                    keyframe.frame,
                    info.frames
                );
            }
        }
        if args.plaque_hint.is_none() {
            let first = track.sorted_keyframes()[0];
            args.plaque_hint = Some(quad_bounds(first.quad));
            args.plaque_frame = Some(first.frame);
        }
        identity.plaque_id = Some(track.plaque.clone());
        identity.motion_track = Some(semantic_provenance(&path, &track)?);
        identity.locked_keyframes = track.locked_keyframes();
        identity.guide_keyframes = track.guide_keyframes();
        Some(track)
    } else {
        None
    };

    if let Some([x, y, width, height]) = args.plaque_hint {
        if ![x, y, width, height].iter().all(|value| value.is_finite())
            || width <= 0.0
            || height <= 0.0
        {
            bail!("plaque bounds must be finite with positive width and height");
        }
        let frame = args.plaque_frame.unwrap_or(0);
        if frame >= info.frames {
            bail!(
                "plaque bounds reference frame {} is outside the {}-frame source",
                frame,
                info.frames
            );
        }
    }

    let provenance = if identity == RefinementProvenance::default() {
        None
    } else {
        Some(identity)
    };
    Ok(AnalysisRefinements {
        motion_track,
        layers: layer_inputs,
        provenance,
    })
}

fn same_file(a: &Path, b: &Path) -> Result<()> {
    let a = a
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", a.display()))?;
    let b = b
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", b.display()))?;
    if a != b {
        bail!("resolved paths differ: {} != {}", a.display(), b.display());
    }
    Ok(())
}

fn quad_bounds(quad: [[f64; 2]; 4]) -> [f64; 4] {
    let min_x = quad
        .iter()
        .map(|point| point[0])
        .fold(f64::INFINITY, f64::min);
    let min_y = quad
        .iter()
        .map(|point| point[1])
        .fold(f64::INFINITY, f64::min);
    let max_x = quad
        .iter()
        .map(|point| point[0])
        .fold(f64::NEG_INFINITY, f64::max);
    let max_y = quad
        .iter()
        .map(|point| point[1])
        .fold(f64::NEG_INFINITY, f64::max);
    [min_x, min_y, max_x - min_x, max_y - min_y]
}

fn partial_path(output: &std::path::Path) -> PathBuf {
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("analysis.analysis");
    output.with_file_name(format!("{name}.partial-{}", std::process::id()))
}

fn geometric_mean(values: &[f64]) -> f64 {
    let product = values
        .iter()
        .map(|value| value.clamp(1.0e-9, 1.0))
        .product::<f64>();
    product.powf(1.0 / values.len() as f64)
}

fn remedies(
    args: &AnalyzeArgs,
    rect: crate::model::RectF,
    candidate: f64,
    motion: f64,
    structure: f64,
    structural_area: f64,
    occlusion: f64,
) -> Vec<String> {
    let mut output = Vec::new();
    if structural_area < 0.001 {
        output.push(if args.plaque_hint.is_some() {
            format!(
                "the refinement selected {:.0},{:.0},{:.0},{:.0}, which contains {:.3}% stable structure; correct it to the full plaque bounds",
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                structural_area * 100.0
            )
        } else {
            format!(
                "automatic selection chose {:.0},{:.0},{:.0},{:.0}, which contains {:.3}% stable structure; inspect candidate.png, run refine, and correct the plaque bounds",
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                structural_area * 100.0
            )
        });
        return output;
    }
    if candidate < 0.75 {
        output.push(format!(
            "automatic detection selected {:.0},{:.0},{:.0},{:.0} with confidence {candidate:.3}; inspect candidate.png and create a refinement if the rectangle is wrong",
            rect.x, rect.y, rect.width, rect.height
        ));
    }
    if motion < 0.70 {
        output.push(format!(
            "automatic motion confidence is {motion:.3}; export the motion and lock only incorrect frames before reanalysis"
        ));
    }
    if structure < 0.75 {
        output.push(format!(
            "canonical extraction confidence is {structure:.3}; inspect canonical-reference.png and temporal-mad.png"
        ));
    }
    if occlusion < 0.80 {
        output.push(format!(
            "occlusion confidence is {occlusion:.3}; inspect occlusion-summary.json and add a foreground refinement when automatic separation is wrong"
        ));
    }
    output
}
