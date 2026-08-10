use std::fs;

use anyhow::{Context, Result, bail};

use crate::{
    analysis::Analysis,
    analyze::{candidate, extraction::transformed_rect},
    cli::{ExportMotionArgs, RefineArgs},
    refinement::{PlaqueProposal, motion_track_document, refinement_document, write_refinement},
    video, workspace,
};

pub fn refine(args: RefineArgs) -> Result<()> {
    if !args.input.is_file() {
        bail!(
            "input video does not exist or is not a file: {}",
            args.input.display()
        );
    }
    let output = args
        .output
        .clone()
        .map(Ok)
        .unwrap_or_else(|| workspace::refinement_path(&args.input))?;
    if output.exists() && !args.force {
        bail!(
            "refusing to overwrite refinement {}; use --force to replace it",
            output.display()
        );
    }
    let info = video::probe(&args.ffprobe, &args.input)
        .with_context(|| format!("failed to probe input video {}", args.input.display()))?;
    if !info.constant_frame_rate {
        bail!("variable-frame-rate input is unsupported; transcode it to a constant frame rate");
    }
    if let Some(diagnostics) = &args.diagnostics {
        fs::create_dir_all(diagnostics).with_context(|| {
            format!(
                "failed to create diagnostics directory {}",
                diagnostics.display()
            )
        })?;
    }
    let report = candidate::detect_proposals(&args.input, 24, &info, args.diagnostics.as_deref())
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
    let contents = refinement_document(&args.input, &output, "ensemble", proposal, &alternatives)?;
    write_refinement(&output, &contents, args.force)?;
    let Some(report) = report else {
        bail!(
            "automatic plaque detection found no plausible candidate; refinement written to {}",
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
    println!("refinement: {}", output.display());
    Ok(())
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

pub fn export_motion(args: ExportMotionArgs) -> Result<()> {
    let pack = Analysis::open(&args.analysis)?;
    let source = pack.source_path();
    let output = args
        .output
        .clone()
        .map(Ok)
        .unwrap_or_else(|| workspace::motion_path(&source))?;
    let analyzed_plaque = pack
        .manifest
        .refinements
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
    write_refinement(&output, &contents, args.force)?;
    let authority = if args.locked { "locked" } else { "guided" };
    println!(
        "motion refinement: {} ({} {authority} frames)",
        output.display(),
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
