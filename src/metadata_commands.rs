use std::fs;

use anyhow::{Context, Result, bail};

use crate::{
    analyze::{candidate, extraction::transformed_rect},
    cli::{CandidateDetector, ExportTrackArgs, InitArgs},
    metadata::{
        PlaqueProposal, default_sidecar_path, motion_track_document, sidecar_document,
        write_human_file,
    },
    titlepack::TitlePack,
    video,
};

pub fn init(args: InitArgs) -> Result<()> {
    if !args.input.is_file() {
        bail!(
            "input video does not exist or is not a file: {}",
            args.input.display()
        );
    }
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| default_sidecar_path(&args.input));
    if output.exists() && !args.force {
        bail!(
            "refusing to overwrite human-owned file {}; use --force only after reviewing it",
            output.display()
        );
    }
    let info = video::probe(&args.ffprobe, &args.input)
        .with_context(|| format!("failed to probe input video {}", args.input.display()))?;
    if !info.constant_frame_rate {
        bail!(
            "variable-frame-rate input is outside the 0.3 source contract; transcode it to a constant frame rate before initialization"
        );
    }
    if let Some(diagnostics) = &args.diagnostics {
        fs::create_dir_all(diagnostics).with_context(|| {
            format!(
                "failed to create diagnostics directory {}",
                diagnostics.display()
            )
        })?;
    }
    let detector = detector_name(args.detector);
    let report = candidate::detect_proposals(
        &args.input,
        args.detector,
        args.candidate_samples,
        &info,
        args.diagnostics.as_deref(),
    )
    .context("automatic plaque proposal failed")?;
    let proposal = report.as_ref().map(|report| to_proposal(&report.selected));
    let alternatives = report
        .as_ref()
        .map(|report| {
            report
                .alternatives
                .iter()
                .map(to_proposal)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let contents = sidecar_document(&args.input, &output, detector, proposal, &alternatives)?;
    write_human_file(&output, &contents, args.force)?;
    let Some(report) = report else {
        bail!(
            "automatic plaque detection found no plausible candidate; unresolved metadata was written to {}",
            output.display()
        );
    };
    println!(
        "plaque proposal: frame {}, confidence {:.3}, bounds {:.0},{:.0},{:.0},{:.0}",
        report.selected.frame_index,
        report.selected.confidence,
        report.selected.rect.x,
        report.selected.rect.y,
        report.selected.rect.width,
        report.selected.rect.height,
    );
    println!("source metadata: {}", output.display());
    Ok(())
}

fn detector_name(detector: CandidateDetector) -> &'static str {
    match detector {
        CandidateDetector::Ensemble => "ensemble",
        CandidateDetector::Geometry => "geometry",
        CandidateDetector::Color => "color",
        CandidateDetector::Text => "text",
    }
}

fn to_proposal(candidate: &candidate::Candidate) -> PlaqueProposal {
    PlaqueProposal {
        reference_frame: candidate.frame_index,
        bounds: [
            candidate.rect.x,
            candidate.rect.y,
            candidate.rect.width,
            candidate.rect.height,
        ],
        confidence: candidate.confidence,
    }
}

pub fn export_track(args: ExportTrackArgs) -> Result<()> {
    let pack = TitlePack::open(&args.analysis)?;
    let analyzed_plaque = pack
        .manifest
        .human_inputs
        .as_ref()
        .and_then(|inputs| inputs.plaque_id.as_deref());
    let plaque = export_plaque_id(args.plaque.as_deref(), analyzed_plaque);
    let frames = pack
        .motion
        .iter()
        .map(|sample| {
            let quad = transformed_rect(pack.manifest.source_plaque_rect, sample.transform);
            let points = quad.points();
            (
                sample.frame,
                points.map(|point| [point.x, point.y]),
                sample.plaque_visibility,
            )
        })
        .collect::<Vec<_>>();
    let contents =
        motion_track_document(&plaque, &pack.manifest.source.sha256, &frames, args.locked)?;
    write_human_file(&args.output, &contents, args.force)?;
    let authority = if args.locked { "locked" } else { "guided" };
    println!(
        "human motion track: {} ({} {authority} frames)",
        args.output.display(),
        frames.len()
    );
    Ok(())
}

fn export_plaque_id(requested: Option<&str>, analyzed: Option<&str>) -> String {
    requested.or(analyzed).unwrap_or("main").to_string()
}

#[cfg(test)]
mod tests {
    use super::export_plaque_id;

    #[test]
    fn export_uses_the_analyzed_plaque_unless_overridden() {
        assert_eq!(export_plaque_id(None, Some("left")), "left");
        assert_eq!(export_plaque_id(Some("right"), Some("left")), "right");
        assert_eq!(export_plaque_id(None, None), "main");
    }
}
