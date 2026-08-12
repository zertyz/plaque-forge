use std::{collections::BTreeSet, fs, path::Path};

use anyhow::{Context, Result, bail};

use crate::{
    analysis::Analysis,
    analyze::{candidate, extraction::transformed_rect},
    cli::{ExportMotionArgs, PlacePlaqueArgs, PlacementMotion, RefineArgs},
    refinement::{
        PlaqueProposal, REFINEMENT_SCHEMA_VERSION, Refinement, motion_track_document,
        refinement_document, relative_reference, write_refinement,
    },
    surface::Surface,
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
    info.ensure_supported_compositing_color()?;
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

pub fn place_plaque(args: PlacePlaqueArgs) -> Result<()> {
    if !args.input.is_file() {
        bail!(
            "input video does not exist or is not a file: {}",
            args.input.display()
        );
    }
    if !args.image.is_file() {
        bail!(
            "plaque image does not exist or is not a file: {}",
            args.image.display()
        );
    }
    let output = args
        .output
        .clone()
        .map(Ok)
        .unwrap_or_else(|| workspace::refinement_path(&args.input))?;
    if output.exists() && !args.force {
        bail!(
            "refusing to overwrite refinement {}; use --force only when replacing the whole plaque-placement refinement is intentional",
            output.display()
        );
    }

    let info = video::probe(&args.ffprobe, &args.input)
        .with_context(|| format!("failed to probe input video {}", args.input.display()))?;
    info.ensure_supported_compositing_color()?;
    if !info.constant_frame_rate {
        bail!("variable-frame-rate input is unsupported; transcode it to a constant frame rate");
    }

    let source_image = image::open(&args.image)
        .with_context(|| format!("failed to decode plaque image {}", args.image.display()))?
        .to_rgba8();
    let bounds = if args.bounds.is_empty() {
        let samples = placement_samples(&args.ffmpeg, &args.input, &info)?;
        let proposal = propose_quiet_placement(
            &samples,
            info.width,
            info.height,
            source_image.width(),
            source_image.height(),
        )?;
        println!(
            "injected plaque placement proposal: {:.0},{:.0},{:.0},{:.0} (quiet-region score {:.3})",
            proposal[0], proposal[1], proposal[2], proposal[3], proposal[4]
        );
        [proposal[0], proposal[1], proposal[2], proposal[3]]
    } else {
        four_values(&args.bounds, "--bounds")?
    };
    validate_placement(bounds, info.width, info.height)?;

    let inset = if args.inset.is_empty() {
        [0.08, 0.12, 0.08, 0.12]
    } else {
        four_values(&args.inset, "--inset")?
    };
    validate_inset(inset)?;

    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    let normalized_image = parent.join("injected-plaque.png");
    let preview = (!args.no_preview).then(|| parent.join("placement-preview.png"));
    if !args.force {
        for target in [&normalized_image, &output]
            .into_iter()
            .chain(preview.as_ref())
        {
            if target.exists() {
                bail!(
                    "refusing to replace plaque-placement bundle member {}; use --force only when replacing the whole placement is intentional",
                    target.display()
                );
            }
        }
    }
    let staged = crate::staged_output::create(&output)?;
    let staged_image = staged.path().join("injected-plaque.png");
    source_image
        .save_with_format(&staged_image, image::ImageFormat::Png)
        .with_context(|| format!("failed to stage injected plaque {}", staged_image.display()))?;

    let source = relative_reference(&output, &args.input)?;
    let motion = match args.motion {
        PlacementMotion::Auto => "auto",
        PlacementMotion::Screen => "screen",
        PlacementMotion::Scene => "scene",
    };
    let text = format!(
        "# Human-readable injected-plaque intent. Dense motion/occlusion state belongs in the analysis cache.\n\
         schema_version = {REFINEMENT_SCHEMA_VERSION}\n\
         source = {}\n\
         default_plaque = \"main\"\n\n\
         [[plaques]]\n\
         id = \"main\"\n\
         reference_frame = 0\n\
         bounds = [{:.1}, {:.1}, {:.1}, {:.1}]\n\n\
         [plaques.surface]\n\
         type = \"injected\"\n\
         image = \"injected-plaque.png\"\n\
         motion = \"{}\"\n\
         inset = [{:.4}, {:.4}, {:.4}, {:.4}]\n",
        toml::Value::String(source.to_string_lossy().into_owned()),
        bounds[0],
        bounds[1],
        bounds[2],
        bounds[3],
        motion,
        inset[0],
        inset[1],
        inset[2],
        inset[3],
    );
    let parsed: Refinement =
        toml::from_str(&text).context("generated injected-plaque refinement is not valid TOML")?;
    parsed
        .validate()
        .context("generated injected-plaque refinement is invalid")?;
    let output_name = output
        .file_name()
        .context("refinement output has no file name")?;
    let staged_manifest = staged.path().join(output_name);
    fs::write(&staged_manifest, &text).with_context(|| {
        format!(
            "failed to stage plaque refinement {}",
            staged_manifest.display()
        )
    })?;

    let staged_preview = if let Some(preview) = &preview {
        let staged_preview = staged.path().join("placement-preview.png");
        write_placement_preview(
            &args.ffmpeg,
            &args.input,
            &info,
            &staged_image,
            bounds,
            &staged_preview,
        )?;
        Some((staged_preview, preview.clone()))
    } else {
        None
    };
    let mut members = vec![(staged_image, normalized_image)];
    if let Some(preview) = staged_preview {
        members.push(preview);
    }
    // The refinement manifest is the bundle's commit marker.
    members.push((staged_manifest, output.clone()));
    staged.commit_files(&members, args.force)?;

    if let Some(preview) = preview {
        println!("placement preview: {}", preview.display());
    }
    println!("refinement: {}", output.display());
    println!(
        "next: analyze this asset; automatic plaque selection/source-plaque appearance recovery are skipped, while scene motion and foreground crossings are still analyzed"
    );
    Ok(())
}

fn four_values(values: &[f64], option: &str) -> Result<[f64; 4]> {
    if values.len() != 4 {
        bail!("{option} requires exactly four values");
    }
    Ok([values[0], values[1], values[2], values[3]])
}

fn validate_placement(bounds: [f64; 4], width: u32, height: u32) -> Result<()> {
    if bounds.iter().any(|value| !value.is_finite())
        || bounds[0] < 0.0
        || bounds[1] < 0.0
        || bounds[2] <= 1.0
        || bounds[3] <= 1.0
        || bounds[0] + bounds[2] > width as f64
        || bounds[1] + bounds[3] > height as f64
    {
        bail!(
            "plaque placement must be a finite positive rectangle inside the {}x{} video",
            width,
            height
        );
    }
    Ok(())
}

fn validate_inset(inset: [f64; 4]) -> Result<()> {
    if inset
        .iter()
        .any(|value| !value.is_finite() || !(0.0..=0.45).contains(value))
        || inset[0] + inset[2] >= 0.95
        || inset[1] + inset[3] >= 0.95
    {
        bail!(
            "--inset values must be fractions in [0,0.45] and must leave a positive inner writing region"
        );
    }
    Ok(())
}

fn placement_samples(ffmpeg: &Path, input: &Path, info: &video::VideoInfo) -> Result<Vec<Surface>> {
    let wanted_count = 9usize;
    let last = info.frames.saturating_sub(1);
    let targets = (0..wanted_count)
        .map(|index| {
            if wanted_count == 1 {
                0
            } else {
                index * last / (wanted_count - 1)
            }
        })
        .collect::<BTreeSet<_>>();
    let mut decoder = video::Decoder::spawn(ffmpeg, input, info)?;
    let mut frames = Vec::new();
    let mut index = 0usize;
    while let Some(frame) = decoder.next_frame()? {
        if targets.contains(&index) {
            frames.push(frame);
        }
        index += 1;
    }
    decoder.finish()?;
    if frames.is_empty() {
        bail!("could not decode any frames while proposing injected-plaque placement");
    }
    Ok(frames)
}

/// Returns [x,y,width,height,score]. Lower score means a quieter, less visually busy region.
fn propose_quiet_placement(
    frames: &[Surface],
    frame_width: u32,
    frame_height: u32,
    plaque_width: u32,
    plaque_height: u32,
) -> Result<[f64; 5]> {
    if plaque_width == 0 || plaque_height == 0 {
        bail!("plaque image has zero width or height");
    }
    let portrait = frame_height > frame_width;
    let max_width = frame_width as f64 * if portrait { 0.78 } else { 0.68 };
    let max_height = frame_height as f64 * if portrait { 0.25 } else { 0.30 };
    let scale = (max_width / plaque_width as f64)
        .min(max_height / plaque_height as f64)
        .max(1.0 / plaque_width.max(plaque_height) as f64);
    let width = (plaque_width as f64 * scale).clamp(8.0, frame_width as f64);
    let height = (plaque_height as f64 * scale).clamp(8.0, frame_height as f64);
    let max_x = (frame_width as f64 - width).max(0.0);
    let max_y = (frame_height as f64 - height).max(0.0);

    let mut best = [0.0, 0.0, width, height, f64::INFINITY];
    for yi in 0..7 {
        let y = max_y * yi as f64 / 6.0;
        for xi in 0..7 {
            let x = max_x * xi as f64 / 6.0;
            let activity = region_activity(frames, x, y, width, height);
            let cx = (x + width * 0.5) / frame_width as f64;
            let cy = (y + height * 0.5) / frame_height as f64;
            // A small presentation prior breaks ties toward upper/central title locations;
            // image evidence remains dominant.
            let prior = (cx - 0.5).abs() * 0.025 + (cy - 0.20).abs() * 0.050;
            let score = activity + prior;
            if score < best[4] {
                best = [x.round(), y.round(), width.round(), height.round(), score];
            }
        }
    }
    Ok(best)
}

fn region_activity(frames: &[Surface], x: f64, y: f64, width: f64, height: f64) -> f64 {
    let columns = 24usize;
    let rows = 14usize;
    let mut temporal = 0.0;
    let mut edges = 0.0;
    let mut points = 0usize;
    let middle = &frames[frames.len() / 2];
    for row in 0..rows {
        for column in 0..columns {
            let px = (x + width * (column as f64 + 0.5) / columns as f64)
                .round()
                .clamp(0.0, middle.width().saturating_sub(1) as f64) as u32;
            let py = (y + height * (row as f64 + 0.5) / rows as f64)
                .round()
                .clamp(0.0, middle.height().saturating_sub(1) as f64) as u32;
            let mut sum = 0.0;
            let mut sum_sq = 0.0;
            for frame in frames {
                let value = luma(frame.pixel(px, py));
                sum += value;
                sum_sq += value * value;
            }
            let n = frames.len() as f64;
            let mean = sum / n;
            temporal += (sum_sq / n - mean * mean).max(0.0) / (255.0 * 255.0);

            let nx = (px + (width / columns as f64).round().max(1.0) as u32)
                .min(middle.width().saturating_sub(1));
            let ny = (py + (height / rows as f64).round().max(1.0) as u32)
                .min(middle.height().saturating_sub(1));
            let center = luma(middle.pixel(px, py));
            edges += ((center - luma(middle.pixel(nx, py))).abs()
                + (center - luma(middle.pixel(px, ny))).abs())
                / (2.0 * 255.0);
            points += 1;
        }
    }
    if points == 0 {
        return 1.0;
    }
    temporal / points as f64 * 0.65 + edges / points as f64 * 0.35
}

fn luma(pixel: crate::color::Rgba) -> f64 {
    pixel.r as f64 * 0.2126 + pixel.g as f64 * 0.7152 + pixel.b as f64 * 0.0722
}

fn write_placement_preview(
    ffmpeg: &Path,
    input: &Path,
    info: &video::VideoInfo,
    plaque: &Path,
    bounds: [f64; 4],
    output: &Path,
) -> Result<()> {
    let mut decoder = video::Decoder::spawn(ffmpeg, input, info)?;
    let mut frame = decoder
        .next_frame()?
        .context("could not decode frame 0 for placement preview")?;
    // Drain the decoder so FFmpeg exits normally rather than seeing a broken output pipe.
    while decoder.next_frame()?.is_some() {}
    decoder.finish()?;

    let plaque = image::open(plaque)?.to_rgba8();
    let resized = image::imageops::resize(
        &plaque,
        bounds[2].round().max(1.0) as u32,
        bounds[3].round().max(1.0) as u32,
        image::imageops::FilterType::Lanczos3,
    );
    let surface = Surface::from_rgba(resized.width(), resized.height(), resized.into_raw())?;
    frame.blend_surface(
        &surface,
        bounds[0].round() as i32,
        bounds[1].round() as i32,
        1.0,
    );
    image::save_buffer(
        output,
        frame.pixels(),
        frame.width(),
        frame.height(),
        image::ColorType::Rgba8,
    )
    .with_context(|| format!("failed to save placement preview {}", output.display()))?;
    Ok(())
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
