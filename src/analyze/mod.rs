mod candidate;
pub(crate) mod extraction;
mod occlusion;
mod tracking;

use std::{fs, path::PathBuf};

use anyhow::{Context, Result, bail};

use crate::{
    cli::AnalyzeArgs,
    model::AnalysisConfidence,
    progress::ProgressReporter,
    titlepack::{
        CURRENT_FORMAT_VERSION, MOTION_FILE, PackStatus, SourceInfo, TitlePack, TitlePackManifest,
    },
    video,
};

pub fn run(args: AnalyzeArgs) -> Result<()> {
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
    progress.finish(format!(
        "{}x{}, {:.3} fps, {} frames",
        info.width, info.height, info.fps, info.frames
    ));

    let output = args.output.clone();
    if output.exists() {
        if !args.force {
            bail!(
                "analysis output already exists: {}\nhelp: use --force for analyze or --reanalyze for replace",
                output.display()
            );
        }
        if output.is_dir() {
            fs::remove_dir_all(&output)
                .with_context(|| format!("failed to remove old title-pack {}", output.display()))?;
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
        .with_context(|| format!("failed to create partial title-pack {}", partial.display()))?;

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

    let result = (|| -> Result<TitlePack> {
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

        let mut track = if args.track_csv.is_some() {
            tracking::load_supervised(&args, &info, candidate.rect, &diagnostics, &mut progress)
                .context("supervised plaque tracking failed")?
        } else {
            tracking::track(
                &args,
                &info,
                candidate.rect,
                candidate.frame_index,
                &diagnostics,
                &mut progress,
            )
            .context("adaptive scene and plaque tracking failed")?
        };

        let extraction = extraction::recover(
            &args.ffmpeg,
            &args.input,
            &info,
            candidate.rect,
            &mut track.samples,
            &partial,
            &diagnostics,
            args.extraction_samples,
            args.local_refinement_radius,
            args.track_csv.is_none(),
            track.reference_frame,
            args.tracking_inertia,
            track.loop_closed,
            &mut progress,
        )
        .context("canonical plaque structure analysis failed")?;

        progress.start(7, 7, "Analyze foreground occlusion", Some(info.frames));
        let occlusion = if args.disable_occlusion {
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
                &mut progress,
            )
            .context("foreground occlusion extraction failed")?
        };
        progress.finish(format!(
            "occluder {}, confidence {:.3}",
            if occlusion.has_occluder {
                "detected"
            } else {
                "not detected"
            },
            occlusion.confidence
        ));

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
                "format_version": CURRENT_FORMAT_VERSION,
                "source_contract": "text-free-plaque",
                "candidate_confidence": candidate.confidence,
                "motion_confidence": track.confidence,
                "structural_confidence": extraction.confidence,
                "content_cavity_area": extraction.cavity_area,
                "structural_area": extraction.structural_area,
                "occlusion_confidence": occlusion.confidence,
                "occluder_mean_coverage": occlusion.mean_coverage,
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

        let manifest = TitlePackManifest {
            format_version: CURRENT_FORMAT_VERSION,
            status: PackStatus::Complete,
            source_is_text_free: true,
            analyzer_build: crate::build_info::SOURCE_FINGERPRINT.to_string(),
            source: SourceInfo {
                path: args.input.canonicalize().unwrap_or(args.input.clone()),
                sha256: video::sha256(&args.input)
                    .with_context(|| format!("failed to hash source {}", args.input.display()))?,
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
            analysis_gate_passed,
            confidence: AnalysisConfidence {
                plaque_detection: candidate.confidence,
                motion: track.confidence,
                extraction: extraction.confidence,
                occlusion: occlusion.confidence,
                overall,
            },
        };

        TitlePack::create(&partial, manifest, track.samples)
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
            "analysis succeeded but title-pack could not be committed from {} to {}",
            partial.display(),
            output.display()
        )
    })?;
    let pack = TitlePack::open(&output)?;
    println!("title-pack: {}", pack.root.display());
    println!(
        "overall analysis confidence: {:.3}",
        pack.manifest.confidence.overall
    );
    Ok(())
}

fn partial_path(output: &std::path::Path) -> PathBuf {
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("analysis.titlepack");
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
                "--plaque-hint selected {:.0},{:.0},{:.0},{:.0}, which contains {:.3}% stable structure and is not a usable plaque; remove the hint for automatic selection or correct it to the full plaque bounds",
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                structural_area * 100.0
            )
        } else {
            format!(
                "automatic selection chose {:.0},{:.0},{:.0},{:.0}, which contains {:.3}% stable structure and is not a usable plaque; inspect candidate.png and rerun with --plaque-hint x,y,width,height around the full plaque",
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
            "automatic detection selected {:.0},{:.0},{:.0},{:.0} with confidence {candidate:.3}; inspect candidate.png and pass --plaque-hint x,y,width,height only if the yellow rectangle is wrong",
            rect.x, rect.y, rect.width, rect.height
        ));
    }
    if motion < 0.70 {
        output.push(format!(
            "automatic motion confidence is {motion:.3}; all frames were measured. --anchor-interval {} only controls feature-reference refreshes; use verification to distinguish spatial misregistration from temporal jitter before changing controls",
            args.anchor_interval
        ));
    }
    if structure < 0.75 {
        output.push(format!(
            "canonical extraction confidence is {structure:.3}; inspect canonical-reference.png and temporal-mad.png"
        ));
    }
    if occlusion < 0.80 {
        output.push(format!(
            "occlusion confidence is {occlusion:.3}; inspect occlusion-summary.json, then adjust --occlusion-sensitivity or use --disable-occlusion only when nothing crosses the plaque"
        ));
    }
    output
}
