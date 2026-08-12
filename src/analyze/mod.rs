pub(crate) mod candidate;
pub(crate) mod extraction;
mod occlusion;
mod tracking;

use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use image::{GrayImage, ImageBuffer, Luma};

use crate::{
    analysis::{
        ANALYSIS_SCHEMA_VERSION, Analysis, AnalysisManifest, AnalysisStatus, INJECTED_SURFACE_FILE,
        InjectedSurfaceAsset, MOTION_FILE, OCCLUDER_DIR, SegmentationConfig, SourceInfo,
    },
    cli::AnalyzeArgs,
    layers::{self, LayerInput},
    model::AnalysisConfidence,
    progress::ProgressReporter,
    refinement::{
        InjectedMotion, MotionRefinement, OcclusionMode, PlaqueSurface, RefinementProvenance,
        find_refinement, layer_artifact_path, layer_artifact_provenance, provenance,
        resolve_relative, selected_layer_artifacts, semantic_provenance,
    },
    video, workspace,
};

struct InjectedSurfaceInput {
    path: std::path::PathBuf,
    motion: InjectedMotion,
    inset: [f64; 4],
}

struct AnalysisRefinements {
    motion_track: Option<MotionRefinement>,
    layers: Vec<LayerInput>,
    provenance: Option<RefinementProvenance>,
    injected_surface: Option<InjectedSurfaceInput>,
    source_motion: Option<InjectedMotion>,
    occlusion_mode: OcclusionMode,
}

impl AnalysisRefinements {
    fn has_dense_locked_track(&self, frame_count: usize) -> bool {
        self.motion_track
            .as_ref()
            .is_some_and(|track| track.is_dense_locked(frame_count))
    }
}

pub fn run(mut args: AnalyzeArgs) -> Result<()> {
    if !args.source_is_text_free {
        bail!(
            "analysis requires an explicit --source-is-text-free assertion; Plaque Forge does not remove or inpaint existing titles"
        );
    }
    if args.force_ml && args.segmentation_worker.is_none() {
        bail!(
            "--force-ml requires --segmentation-worker; use the high-level analyze script for the configured ML runtime"
        );
    }
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
    info.ensure_supported_compositing_color()?;
    if !info.constant_frame_rate {
        bail!(
            "variable-frame-rate input is unsupported; transcode it to a constant frame rate before analysis"
        );
    }
    let source_sha256 = crate::digest::file_sha256(&args.input)
        .with_context(|| format!("failed to hash source {}", args.input.display()))?;
    if let Some(worker) = args.segmentation_worker.as_deref() {
        eprintln!(
            "[ml] segmentation enabled: worker={}, backend={}, model={}, device={}",
            worker.display(),
            args.segmentation_backend,
            args.segmentation_model,
            args.segmentation_device
        );
        let generated = crate::segmentation::ensure_prompted_layers(
            crate::segmentation::PromptedLayersRequest {
                input: &args.input,
                explicit_refinement: args.refinement.as_deref(),
                plaque_id: args.plaque.as_deref(),
                worker,
                backend: &args.segmentation_backend,
                model: &args.segmentation_model,
                device: &args.segmentation_device,
                force: args.force_ml,
                ffprobe: &args.ffprobe,
                info: &info,
                source_sha256: &source_sha256,
            },
        )
        .context("failed to materialize prompted refinement layers")?;
        if generated > 0 {
            eprintln!("[ml] generated {generated} prompted refinement layer artifact(s)");
        }
    } else {
        eprintln!("[ml] segmentation disabled: no worker configured (high-level --no-ml mode)");
    }
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
    if output.exists() && args.if_needed && !args.force {
        if analysis_cache_is_current(
            &output,
            &source_sha256,
            refinements.provenance.as_ref(),
            segmentation_config(&args),
        ) {
            println!("analysis cache is current: {}", output.display());
            return Ok(());
        }
        eprintln!("analysis cache is stale; rebuilding: {}", output.display());
        args.force = true;
    }
    if output.exists() && !args.force {
        bail!(
            "analysis output already exists: {}\nhelp: use --force to delete and replace it after a successful rebuild",
            output.display()
        );
    }
    if output.exists() {
        eprintln!(
            "replacing analysis after successful rebuild: {}",
            output.display()
        );
    }
    let staged = crate::staged_output::create(&output)?;
    let partial = staged.path().to_path_buf();

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
        progress.start(
            2,
            7,
            "Resolve writing surface",
            Some(args.candidate_samples),
        );
        let candidate = candidate::detect(&args, &info, &diagnostics)
            .context("writing-surface proposal failed")?;
        progress.finish(format!(
            "frame {}, confidence {:.3}, rect {:.0},{:.0},{:.0},{:.0}",
            candidate.frame_index,
            candidate.confidence,
            candidate.rect.x,
            candidate.rect.y,
            candidate.rect.width,
            candidate.rect.height
        ));

        let injected_motion = refinements
            .injected_surface
            .as_ref()
            .map(|surface| surface.motion);
        let surface_motion = refinements.source_motion.or(injected_motion);
        let candidate_area_ratio =
            candidate.rect.width * candidate.rect.height / (info.width as f64 * info.height as f64);
        // Smooth clouds, dark circular canvases, holographic fields, and similar title
        // surfaces can be excellent writing regions while containing almost no stable
        // feature texture. Treat broad, strongly-detected canvases as a first-class
        // structureless surface instead of requiring plaque-like interior detail.
        let broad_canvas = candidate_area_ratio >= 0.34
            && candidate.confidence >= 0.78
            && (candidate.edge_completeness >= 0.18
                || candidate.screen_stationarity >= 0.45
                || candidate.temporal_support >= 0.62);
        let automatic_structureless_surface = args.plaque_hint.is_none()
            && refinements.motion_track.is_none()
            && ((candidate_area_ratio >= 0.12
                && candidate.screen_stationarity >= 0.64
                && candidate.edge_completeness >= 0.18)
                || broad_canvas);
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
        } else if matches!(surface_motion, Some(InjectedMotion::Screen)) {
            eprintln!("surface motion: screen-fixed (tracking skipped by human intent)");
            tracking::screen_fixed(
                info.frames,
                candidate.frame_index,
                0.98,
                "declared-screen-fixed",
            )
        } else {
            match tracking::track(
                &args,
                &info,
                candidate.rect,
                candidate.frame_index,
                &diagnostics,
                &mut progress,
            ) {
                Ok(track) => track,
                Err(error) if matches!(injected_motion, Some(InjectedMotion::Auto)) => {
                    eprintln!(
                        "warning: scene anchoring for injected surface failed; falling back to screen-fixed placement: {error:#}"
                    );
                    tracking::screen_fixed(
                        info.frames,
                        candidate.frame_index,
                        0.72,
                        "injected-auto-screen-fallback",
                    )
                }
                Err(error) if automatic_structureless_surface => {
                    let stationary_confidence =
                        (0.58 + 0.34 * candidate.screen_stationarity).min(0.92);
                    eprintln!(
                        "warning: feature tracking failed for a broad screen-stationary writing surface; using screen-fixed motion instead: {error:#}"
                    );
                    tracking::screen_fixed(
                        info.frames,
                        candidate.frame_index,
                        stationary_confidence,
                        "automatic-screen-fixed-structureless-surface",
                    )
                }
                Err(error) => {
                    return Err(error.context("adaptive scene and plaque tracking failed"));
                }
            }
        };

        if matches!(injected_motion, Some(InjectedMotion::Auto)) && track.confidence < 0.50 {
            eprintln!(
                "warning: scene anchoring for injected surface had confidence {:.3}; falling back to screen-fixed placement",
                track.confidence
            );
            track = tracking::screen_fixed(
                info.frames,
                candidate.frame_index,
                0.72,
                "injected-auto-screen-fallback",
            );
        }

        if automatic_structureless_surface && track.confidence < 0.58 {
            let stationary_confidence = (0.58 + 0.34 * candidate.screen_stationarity).min(0.92);
            eprintln!(
                "automatic surface is broad and screen-stationary (area {:.1}%, stationarity {:.3}); using screen-fixed motion instead of rejecting a structureless surface",
                candidate_area_ratio * 100.0,
                candidate.screen_stationarity
            );
            track = tracking::screen_fixed(
                info.frames,
                candidate.frame_index,
                stationary_confidence,
                "automatic-screen-fixed-structureless-surface",
            );
        }

        // The declared surface motion chooses the base coordinate system; sparse
        // anchors remain valid corrections within that system. In particular, a
        // mostly screen-fixed plaque may still have a short authored entrance or
        // exit animation. Apply the anchors after any automatic fallback so they
        // cannot be silently discarded by the fallback decision.
        if let Some(refinement_track) = &refinements.motion_track
            && !refinement_track.is_dense_locked(info.frames)
        {
            tracking::apply_motion_refinement(&mut track, refinement_track, candidate.rect)
                .context("failed to apply motion refinement constraints")?;
        }

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
            !refinements.has_dense_locked_track(info.frames)
                && !track.model_name.contains("screen-fixed"),
            refinements.motion_track.as_ref(),
            track.reference_frame,
            args.tracking_inertia,
            track.loop_closed,
            &mut progress,
        )
        .context("canonical plaque/background structure analysis failed")?;
        let surface_intent = SurfaceIntentContext {
            args: &args,
            refinements: &refinements,
            tracking_rect: candidate.rect,
            width: candidate.canonical_width,
            height: candidate.canonical_height,
            output_root: &partial,
            diagnostics: &diagnostics,
        };
        let mut injected_surface_asset = apply_surface_intent(&surface_intent, &mut extraction)?;

        progress.start(7, 7, "Analyze foreground occlusion", Some(info.frames));
        let automatic_occlusion =
            !args.disable_occlusion && refinements.occlusion_mode == OcclusionMode::Automatic;
        let mut occlusion = if !automatic_occlusion {
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
        let authored_foreground = layers::has_authored_foreground(&refinements.layers);
        let mut automatic_ml_foreground = false;
        if automatic_occlusion && occlusion.has_occluder {
            if let Some(worker) = args.segmentation_worker.as_deref() {
                match crate::segmentation::refine_automatic_foreground(
                    crate::segmentation::AutomaticForegroundRequest {
                        input: &args.input,
                        worker,
                        backend: &args.segmentation_backend,
                        model: &args.segmentation_model,
                        device: &args.segmentation_device,
                        info: &info,
                        plaque: candidate.rect,
                        seed_masks: &partial.join(OCCLUDER_DIR),
                        analysis_root: &partial,
                        force: args.force_ml,
                        reuse_root: output.is_dir().then_some(output.as_path()),
                    },
                ) {
                    Ok(true) => {
                        automatic_ml_foreground = true;
                        occlusion.has_occluder = true;
                    }
                    Ok(false) => {}
                    Err(error) => {
                        // Automatic ML is a refinement of the already-available Rust
                        // occlusion path. A model/runtime failure must not destroy an
                        // otherwise useful analysis; authored ML layers remain strict.
                        eprintln!(
                            "warning: automatic ML foreground refinement failed; keeping Rust occlusion masks: {error:#}"
                        );
                    }
                }
            } else {
                eprintln!(
                    "[ml] automatic foreground opportunity detected, but no worker is configured"
                );
            }
        } else if automatic_occlusion {
            eprintln!(
                "[ml] automatic foreground skipped: Rust found no persistent foreground crossing"
            );
        }

        let automatic_exclusions = partial.join(OCCLUDER_DIR);
        let combined_exclusions = partial.join("tracking-exclusions");
        let empty_exclusions = partial.join("no-automatic-exclusions");
        let automatic_tracking_exclusions = if automatic_occlusion {
            &automatic_exclusions
        } else {
            &empty_exclusions
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
        } else if !automatic_occlusion && authored_foreground {
            &empty_exclusions
        } else {
            &automatic_exclusions
        };
        let stabilized_frames = if automatic_occlusion
            && !refinements.has_dense_locked_track(info.frames)
            && !track.model_name.contains("screen-fixed")
            && (has_refinement_exclusions || occlusion.has_occluder)
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
            .context("failed to rebuild the plaque/background model after masked tracking")?;
            injected_surface_asset = apply_surface_intent(&surface_intent, &mut extraction)?;
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
            if automatic_ml_foreground {
                crate::segmentation::install_automatic_foreground_masks(&partial, info.frames)
                    .context("failed to restore ML foreground masks after masked retracking")?;
                occlusion.has_occluder = true;
            }
            repaired
        } else {
            0
        };
        if !automatic_occlusion && authored_foreground {
            occlusion.has_occluder = false;
            if automatic_exclusions.is_dir() {
                crate::staged_output::remove_child(&partial, &automatic_exclusions)?;
            }
        }
        if has_refinement_exclusions {
            crate::staged_output::remove_child(&partial, &combined_exclusions)?;
        }
        if !automatic_occlusion && let Some(refinement_track) = &refinements.motion_track {
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
        let declared_writable_region = args.writable_region_hint.is_some();
        let injected_surface = refinements.injected_surface.is_some();
        let structureless_surface =
            declared_writable_region || automatic_structureless_surface || injected_surface;
        let extraction_floor = if structureless_surface { 0.45 } else { 0.65 };
        let structure_gate = structureless_surface || extraction.structural_area >= 0.001;
        let motion_floor = if structureless_surface { 0.44 } else { 0.50 };
        let component_gate_passed = candidate.confidence >= 0.60
            && track.confidence >= motion_floor
            && extraction.confidence >= extraction_floor
            && structure_gate
            && occlusion.confidence >= 0.55;
        let analysis_gate_passed =
            overall >= args.minimum_analysis_confidence && component_gate_passed;
        let recovery = remedies(&AnalysisQuality {
            args: &args,
            rect: candidate.rect,
            candidate: candidate.confidence,
            motion: track.confidence,
            structure: extraction.confidence,
            structural_area: extraction.structural_area,
            occlusion: occlusion.confidence,
            structure_optional: structureless_surface,
        });

        fs::write(
            partial.join("analysis-summary.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version": ANALYSIS_SCHEMA_VERSION,
                "source_contract": "text-free-writing-surface",
                "declared_writable_region": args.writable_region_hint.as_ref().map(|region| region.kind()),
                "automatic_structureless_surface": automatic_structureless_surface,
                "surface_source": if injected_surface { "injected" } else { "source" },
                "candidate_screen_stationarity": candidate.screen_stationarity,
                "candidate_area_ratio": candidate_area_ratio,
                "broad_canvas": broad_canvas,
                "candidate_confidence": candidate.confidence,
                "motion_confidence": track.confidence,
                "structural_confidence": extraction.confidence,
                "content_cavity_area": extraction.cavity_area,
                "structural_area": extraction.structural_area,
                "occlusion_confidence": occlusion.confidence,
                "has_occluder": occlusion.has_occluder,
                "occluder_mean_coverage": occlusion.mean_coverage,
                "occlusion_stabilized_frames": stabilized_frames,
                "automatic_ml_foreground": automatic_ml_foreground,
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
                "analysis quality gate failed: writing surface {:.0},{:.0},{:.0},{:.0}; overall {overall:.3} (minimum {:.3}), components candidate {:.3}, tracking {:.3}, extraction {:.3} (structural area {:.3}%), occlusion {:.3}. {}. Use --allow-low-confidence only to retain diagnostic output",
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
            analyzer_build: crate::build_info::ANALYZER_CACHE_VERSION.to_string(),
            migrated_from_analyzer: None,
            source: SourceInfo {
                path: crate::portable_path::relative_reference(
                    &output.join("manifest.toml"),
                    &args.input,
                )?,
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
            occlusion_mode: refinements.occlusion_mode,
            segmentation: segmentation_config(&args),
            automatic_ml_foreground,
            injected_surface: injected_surface_asset,
            layers: packed_layers,
            refinements: refinements
                .provenance
                .as_ref()
                .map(|provenance| provenance.portable_for(&output.join("manifest.toml")))
                .transpose()?,
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
            let disposition = match crate::staged_output::retain_failure(&partial, &output) {
                Ok(Some(path)) => {
                    format!("compact diagnostics were retained in {}", path.display())
                }
                Ok(None) => "temporary work was cleaned up".to_string(),
                Err(retention_error) => {
                    eprintln!(
                        "warning: compact failure diagnostics could not be retained: {retention_error:#}"
                    );
                    "temporary work was cleaned up; diagnostic retention also failed".to_string()
                }
            };
            return Err(error.context(format!("analysis failed; {disposition}")));
        }
    };

    drop(pack);
    staged.commit(args.force)?;
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
    let mut embedded_track = None;
    let mut layer_inputs = Vec::new();
    let mut injected_surface = None;
    let mut source_motion = None;
    let mut occlusion_mode = OcclusionMode::Automatic;

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
        occlusion_mode = selected.occlusion;
        selected_id = Some(selected.id.clone());
        identity.manifest = Some(semantic_provenance(&loaded.path, &loaded.document)?);
        identity.plaque_id = Some(selected.id.clone());
        match &selected.surface {
            Some(PlaqueSurface::Source { motion }) => {
                source_motion = Some(*motion);
            }
            Some(PlaqueSurface::Injected {
                image,
                motion,
                inset,
            }) => {
                let path = resolve_relative(&loaded.path, image);
                if !path.is_file() {
                    bail!("injected plaque image does not exist: {}", path.display());
                }
                image::open(&path).with_context(|| {
                    format!("failed to decode injected plaque image {}", path.display())
                })?;
                identity.surface_asset = Some(provenance(&path)?);
                injected_surface = Some(InjectedSurfaceInput {
                    path,
                    motion: *motion,
                    inset: *inset,
                });
            }
            None => {}
        }

        if args.writable_region_hint.is_none()
            && let Some(region) = &selected.writable_region
        {
            args.writable_region_hint = Some(region.resolve(&loaded.path));
        }
        if args.plaque_hint.is_none()
            && let Some(bounds) = selected.tracking_bounds()
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
            if !layer.prompts.is_empty() {
                let artifact = layer_artifact_path(&loaded.path, layer)
                    .expect("prompted layers always have an inferred artifact path");
                if !artifact.is_file() {
                    bail!(
                        "layer {:?} has segmentation prompts but its artifact is missing at {}; run the high-level analyze script after ./scripts/setup_segmentation.sh",
                        layer.id,
                        artifact.display()
                    );
                }
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
        embedded_track = selected.sparse_motion_track(info.width, info.height, source_sha256)?;
        if let Some(track) = &embedded_track {
            identity.locked_keyframes = track.locked_keyframes();
            identity.guide_keyframes = track.guide_keyframes();
        }
    } else if let Some(id) = &args.plaque {
        bail!("--plaque {id:?} requires a refinement manifest");
    }

    let (motion_track, motion_track_path) = if let Some(track) = embedded_track {
        (Some(track), None)
    } else if let Some(path) = referenced_track {
        (Some(MotionRefinement::load(&path)?), Some(path))
    } else {
        (None, None)
    };
    let motion_track = if let Some(track) = motion_track {
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
        if let Some(path) = motion_track_path {
            identity.motion_track = Some(semantic_provenance(&path, &track)?);
        }
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
        injected_surface,
        source_motion,
        occlusion_mode,
    })
}

fn analysis_cache_is_current(
    output: &Path,
    source_sha256: &str,
    refinements: Option<&RefinementProvenance>,
    segmentation: Option<SegmentationConfig>,
) -> bool {
    let Ok(pack) = Analysis::open(output) else {
        return false;
    };
    if pack.manifest.analyzer_build != crate::build_info::ANALYZER_CACHE_VERSION
        || !pack
            .manifest
            .source
            .sha256
            .eq_ignore_ascii_case(source_sha256)
    {
        return false;
    }
    if pack.manifest.segmentation != segmentation {
        return false;
    }
    match (pack.manifest.refinements.as_ref(), refinements) {
        (None, None) => true,
        (Some(cached), Some(current)) => cached.content_matches(current),
        _ => false,
    }
}

fn segmentation_config(args: &AnalyzeArgs) -> Option<SegmentationConfig> {
    args.segmentation_worker
        .as_ref()
        .map(|_| SegmentationConfig {
            backend: args.segmentation_backend.clone(),
            model: args.segmentation_model.clone(),
            device: args.segmentation_device.clone(),
        })
}

struct SurfaceIntentContext<'a> {
    args: &'a AnalyzeArgs,
    refinements: &'a AnalysisRefinements,
    tracking_rect: crate::model::RectF,
    width: u32,
    height: u32,
    output_root: &'a Path,
    diagnostics: &'a Path,
}

fn apply_surface_intent(
    context: &SurfaceIntentContext<'_>,
    extraction: &mut extraction::ExtractionResult,
) -> Result<Option<InjectedSurfaceAsset>> {
    let SurfaceIntentContext {
        args,
        refinements,
        tracking_rect,
        width,
        height,
        output_root,
        diagnostics,
    } = *context;
    let tracking_bounds = [
        tracking_rect.x,
        tracking_rect.y,
        tracking_rect.width,
        tracking_rect.height,
    ];

    let Some(injected) = refinements.injected_surface.as_ref() else {
        if let Some(region) = args.writable_region_hint.as_ref() {
            apply_declared_writable_region(
                region,
                tracking_bounds,
                width,
                height,
                output_root,
                diagnostics,
                extraction,
            )?;
        }
        return Ok(None);
    };

    let source_image = image::open(&injected.path)
        .with_context(|| format!("failed to load injected plaque {}", injected.path.display()))?
        .to_rgba8();
    let canonical = if source_image.width() == width && source_image.height() == height {
        source_image
    } else {
        image::imageops::resize(
            &source_image,
            width,
            height,
            image::imageops::FilterType::Lanczos3,
        )
    };
    let canonical_path = output_root.join(INJECTED_SURFACE_FILE);
    canonical.save(&canonical_path).with_context(|| {
        format!(
            "failed to save injected plaque {}",
            canonical_path.display()
        )
    })?;

    // Plaque appearance and writability are intentionally orthogonal. In particular,
    // a glass/holographic plaque may have low alpha throughout its writing cavity:
    // multiplying by artwork alpha would make valid title pixels look clipped and
    // would incorrectly attenuate the title a second time. The explicit region or
    // inset owns layout geometry; the PNG alpha is used only when compositing the
    // injected plaque itself.
    let writable = if let Some(region) = args.writable_region_hint.as_ref() {
        region.canonical_mask_in(tracking_bounds, width, height)?
    } else {
        let [left, top, right, bottom] = injected.inset;
        let x0 = (left * width as f64).round().clamp(0.0, width as f64) as u32;
        let y0 = (top * height as f64).round().clamp(0.0, height as f64) as u32;
        let x1 = ((1.0 - right) * width as f64)
            .round()
            .clamp(0.0, width as f64) as u32;
        let y1 = ((1.0 - bottom) * height as f64)
            .round()
            .clamp(0.0, height as f64) as u32;
        let mut mask = vec![0_u8; width as usize * height as usize];
        for y in y0.min(height)..y1.min(height) {
            for x in x0.min(width)..x1.min(width) {
                mask[(y * width + x) as usize] = 255;
            }
        }
        mask
    };
    let area =
        writable.iter().filter(|&&value| value > 127).count() as f64 / writable.len().max(1) as f64;
    if area < 0.01 {
        bail!(
            "injected plaque writable region covers only {:.3}% of its canonical surface",
            area * 100.0
        );
    }
    extraction.content_mask = writable.clone();
    extraction.cavity_area = area;
    save_luma_mask(
        width,
        height,
        &writable,
        &output_root.join("content-mask.png"),
    )?;
    save_luma_mask(
        width,
        height,
        &writable,
        &diagnostics.join("injected-writable-mask.png"),
    )?;

    Ok(Some(InjectedSurfaceAsset {
        path: crate::portable_path::PortablePath::bundle(INJECTED_SURFACE_FILE)?,
        source_sha256: crate::digest::file_sha256(&injected.path)?,
        motion: injected.motion,
    }))
}

fn apply_declared_writable_region(
    region: &crate::writable_region::ResolvedWritableRegion,
    tracking_bounds: [f64; 4],
    width: u32,
    height: u32,
    output_root: &Path,
    diagnostics: &Path,
    extraction: &mut extraction::ExtractionResult,
) -> Result<()> {
    let mask = region.canonical_mask_in(tracking_bounds, width, height)?;
    let area = mask.iter().filter(|&&value| value > 127).count() as f64 / mask.len().max(1) as f64;
    if area < 0.01 {
        bail!(
            "declared {} writable region covers only {:.3}% of its canonical bounds",
            region.kind(),
            area * 100.0
        );
    }
    extraction.content_mask = mask.clone();
    extraction.cavity_area = area;
    save_luma_mask(width, height, &mask, &output_root.join("content-mask.png"))?;
    save_luma_mask(
        width,
        height,
        &mask,
        &diagnostics.join("declared-writable-mask.png"),
    )?;
    Ok(())
}

fn save_luma_mask(width: u32, height: u32, mask: &[u8], path: &Path) -> Result<()> {
    let image: GrayImage = ImageBuffer::<Luma<u8>, _>::from_raw(width, height, mask.to_vec())
        .context("invalid writable-region mask dimensions")?;
    image
        .save(path)
        .with_context(|| format!("failed to save writable-region mask {}", path.display()))
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

fn geometric_mean(values: &[f64]) -> f64 {
    let product = values
        .iter()
        .map(|value| value.clamp(1.0e-9, 1.0))
        .product::<f64>();
    product.powf(1.0 / values.len() as f64)
}

struct AnalysisQuality<'a> {
    args: &'a AnalyzeArgs,
    rect: crate::model::RectF,
    candidate: f64,
    motion: f64,
    structure: f64,
    structural_area: f64,
    occlusion: f64,
    structure_optional: bool,
}

fn remedies(quality: &AnalysisQuality<'_>) -> Vec<String> {
    let AnalysisQuality {
        args,
        rect,
        candidate,
        motion,
        structure,
        structural_area,
        occlusion,
        structure_optional,
    } = *quality;
    let mut output = Vec::new();
    if structural_area < 0.001 && !structure_optional && args.writable_region_hint.is_none() {
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
                "automatic selection chose {:.0},{:.0},{:.0},{:.0}, which contains {:.3}% stable structure; inspect candidate.png and add the smallest writable-region refinement needed to identify the intended surface",
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
            "automatic detection selected {:.0},{:.0},{:.0},{:.0} with confidence {candidate:.3}; inspect candidate.png and add a writable-region refinement if the selected enclosure is wrong",
            rect.x, rect.y, rect.width, rect.height
        ));
    }
    if motion < 0.70 {
        output.push(format!(
            "automatic motion confidence is {motion:.3}; export the motion and lock only incorrect frames before reanalysis"
        ));
    }
    if structure < 0.75 && !structure_optional {
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
