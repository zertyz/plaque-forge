mod effects;
mod typography;

use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use image::{GrayImage, ImageBuffer, Luma, RgbaImage, imageops::FilterType};
use serde::{Deserialize, Serialize};

use crate::{
    analysis::{Analysis, CONTENT_MASK_FILE, OCCLUDER_DIR},
    analyze::extraction::transformed_rect,
    cli::ComposeArgs,
    image_io::load_luma,
    layers::{ForegroundReader, merge_mask},
    model::TypographyMetrics,
    progress::ProgressReporter,
    video::{self, Decoder, Encoder},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct RenderManifest {
    #[serde(default)]
    pub program_version: String,
    #[serde(default)]
    pub renderer_build: String,
    #[serde(default)]
    pub analyzer_build: String,
    pub typography: TypographyMetrics,
    pub frames: usize,
    pub used_occluder_masks: bool,
    pub refinement_foreground_layers: usize,
    pub source_sha256: String,
    pub canonical_text_mask: String,
    #[serde(default)]
    pub text_style: String,
    #[serde(default)]
    pub style_file: Option<String>,
    #[serde(default)]
    pub style_sha256: Option<String>,
    #[serde(default)]
    pub render_contact_sheet: Option<String>,
    #[serde(default)]
    pub title_text: String,
    #[serde(default)]
    pub font_file: String,
    #[serde(default)]
    pub font_sha256: String,
}

pub fn run(args: ComposeArgs) -> Result<()> {
    let mut progress = ProgressReporter::new(args.progress, args.progress_interval_ms);
    progress.start(1, 3, "Open analysis and validate source", None);
    let pack = Analysis::open(&args.analysis).with_context(|| {
        format!(
            "analysis cache is unavailable: {}\nhelp: create it explicitly with `plaque-forge analyze --input {}`",
            args.analysis.display(),
            args.input.display()
        )
    })?;
    pack.require_current_analyzer()?;
    if !pack.manifest.analysis_gate_passed {
        eprintln!("warning: this analysis was accepted below the confidence threshold");
    }
    let text = match (args.text, args.text_file) {
        (Some(text), None) => text,
        (None, Some(path)) => fs::read_to_string(&path)
            .with_context(|| format!("failed to read UTF-8 text file {}", path.display()))?,
        (None, None) => bail!("provide --text or --text-file"),
        (Some(_), Some(_)) => unreachable!(),
    };
    if !args.input.is_file() {
        bail!("input video does not exist: {}", args.input.display());
    }
    let current_sha = crate::digest::file_sha256(&args.input)?;
    if current_sha != pack.manifest.source.sha256 {
        bail!(
            "input video differs from the file used for analysis: {}\nhelp: rebuild the cache explicitly with `plaque-forge analyze --input {} --force`",
            args.input.display(),
            args.input.display()
        );
    }
    let current_refinement = crate::refinement::current_refinement_provenance(
        &args.input,
        args.refinement.as_deref(),
        args.plaque.as_deref(),
    )?;
    let refinements_match = match (&pack.manifest.refinements, &current_refinement) {
        (None, None) => true,
        (Some(cached), Some(current)) => cached.content_matches(current),
        _ => false,
    };
    if !refinements_match {
        bail!(
            "analysis cache does not include the current refinements\nhelp: rebuild it explicitly with `plaque-forge analyze --input {} --force`",
            args.input.display()
        );
    }
    let source = args.input;
    let info = video::probe(&args.ffprobe, &source)?;
    let mask = load_luma(
        &pack.require_asset(CONTENT_MASK_FILE)?,
        pack.manifest.canonical_width,
        pack.manifest.canonical_height,
    )?;
    progress.finish("source and analysis cache are valid");

    progress.start(2, 3, "Shape and fit typography", None);
    let style = effects::Style::load(
        args.style_file.as_deref(),
        effects::DirectStyleOptions {
            text_color: &args.text_color,
            stroke_color: &args.stroke_color,
            glow_color: &args.glow_color,
            glow_radius: args.glow_radius,
            stroke_width_ratio: args.stroke_width,
            shadow_offset_x_ratio: args.shadow_offset_x,
            shadow_offset_y_ratio: args.shadow_offset_y,
            shadow_blur_radius: args.shadow_blur_radius,
            shadow_color: &args.shadow_color,
        },
    )?;
    let text_style = style.describe();
    let style_file = args.style_file.as_ref().map(|path| {
        path.canonicalize()
            .unwrap_or_else(|_| path.clone())
            .to_string_lossy()
            .into_owned()
    });
    let style_sha256 = args
        .style_file
        .as_deref()
        .map(crate::digest::file_sha256)
        .transpose()?;
    let text_render = typography::render(typography::RenderRequest {
        width: pack.manifest.canonical_width,
        height: pack.manifest.canonical_height,
        mask: &mask,
        text: &text,
        font_path: &args.font,
        fit_mode: args.fit,
        requested_font_size: args.font_size,
        supersampling: args.supersampling,
        target_fill: args.target_fill,
        max_lines: args.max_lines,
        padding_ratio: args.padding,
        line_height_ratio: args.line_height,
        text_align: args.text_align,
        vertical_align: args.vertical_align,
        style: &style,
    })?;
    if text_render.metrics.missing_glyphs > 0 || text_render.metrics.fallback_glyphs > 0 {
        bail!(
            "font cannot render the requested title deterministically: {} missing glyphs, {} fallback glyphs",
            text_render.metrics.missing_glyphs,
            text_render.metrics.fallback_glyphs
        );
    }
    let canonical_text_mask_path = args.output.with_extension("text-mask.png");
    save_luma(
        text_render.layer.width(),
        text_render.layer.height(),
        &text_render.layer.alpha_mask(),
        &canonical_text_mask_path,
    )?;
    progress.finish(format!(
        "{:.2}px, {} lines, {:.1}% fill",
        text_render.metrics.font_size,
        text_render.metrics.lines,
        text_render.metrics.fill_ratio * 100.0
    ));

    let encoder_args = if args.encoder_args.is_empty() {
        vec![
            "-c:v".into(),
            "ffv1".into(),
            "-level".into(),
            "3".into(),
            "-coder".into(),
            "1".into(),
            "-context".into(),
            "1".into(),
            "-g".into(),
            "1".into(),
            "-slicecrc".into(),
            "1".into(),
            "-pix_fmt".into(),
            "bgr0".into(),
            "-c:a".into(),
            "copy".into(),
            "-shortest".into(),
        ]
    } else {
        args.encoder_args.clone()
    };
    let mut decoder = Decoder::spawn(&args.ffmpeg, &source, &info)?;
    let mut encoder = Encoder::spawn(&args.ffmpeg, &source, &args.output, &info, &encoder_args)?;
    let masks_dir = pack.root.join(OCCLUDER_DIR);
    let use_masks = pack.manifest.has_occluder && masks_dir.is_dir();
    let foregrounds = ForegroundReader::open(&pack)?;
    let refinement_foreground_layers = pack
        .manifest
        .layers
        .iter()
        .filter(|layer| layer.role == crate::refinement::LayerRole::Foreground)
        .count();
    let mut frame_index = 0usize;
    let diagnostic_indices = evenly_spaced(info.frames, 12);
    let mut diagnostic_frames = Vec::with_capacity(diagnostic_indices.len());
    progress.start(3, 3, "Composite and encode", Some(info.frames));
    while let Some(mut frame) = decoder.next_frame()? {
        if frame_index >= info.frames {
            bail!(
                "decoder produced more than the expected {} source frames",
                info.frames
            );
        }
        let original = frame.clone();
        let sample = pack
            .motion
            .get(frame_index)
            .with_context(|| format!("motion sample missing for frame {frame_index}"))?;

        // The source plaque is already text-free. Static typography is prepared once.
        // Frame-varying presentation (currently pulse/shine) is evaluated here without
        // repeating font discovery, shaping, or line breaking.
        let time_seconds = frame_index as f64 / info.fps.max(f64::EPSILON);
        let style_opacity = style.frame_opacity(time_seconds);
        let animated_overlay = style.frame_overlay(
            &text_render.glyph_mask,
            text_render.layer.width(),
            text_render.layer.height(),
            time_seconds,
        )?;
        let opacity = sample.plaque_visibility.clamp(0.0, 1.0) as f32 * style_opacity;
        if let Some(overlay) = animated_overlay {
            let mut presented = text_render.layer.clone();
            presented.blend_surface(&overlay, 0, 0, 1.0);
            frame.warp_blend(
                &presented,
                transformed_rect(pack.manifest.source_plaque_rect, sample.transform),
                opacity,
            )?;
        } else {
            frame.warp_blend(
                &text_render.layer,
                transformed_rect(pack.manifest.source_plaque_rect, sample.transform),
                opacity,
            )?;
        }
        let mut restore = foregrounds
            .frame_mask(frame_index, sample.transform)?
            .unwrap_or_default();
        if use_masks {
            let path = masks_dir.join(format!("{frame_index:06}.png"));
            if path.exists() {
                merge_mask(
                    &mut restore,
                    &load_full_luma(&path, info.width, info.height)?,
                );
            }
        }
        if !restore.is_empty() {
            frame.restore_from_mask(&original, &restore)?;
        }
        if diagnostic_indices
            .get(diagnostic_frames.len())
            .is_some_and(|&index| index == frame_index)
        {
            diagnostic_frames.push(frame.clone());
        }
        encoder
            .write_frame(&frame)
            .with_context(|| format!("failed to encode frame {frame_index}"))?;
        frame_index += 1;
        progress.update(frame_index, "");
    }
    encoder.finish()?;
    decoder.finish()?;
    if frame_index != info.frames {
        bail!(
            "decoder produced {frame_index} frames, expected {} from the source probe",
            info.frames
        );
    }
    progress.finish(format!("{} frames", frame_index));

    let render_contact_sheet = if let Some(diagnostics) = &args.diagnostics {
        fs::create_dir_all(diagnostics).with_context(|| {
            format!(
                "failed to create render diagnostics directory {}",
                diagnostics.display()
            )
        })?;
        let path = diagnostics.join("render-contact-sheet.png");
        write_contact_sheet(&diagnostic_frames, &path)?;
        Some(
            path.canonicalize()
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned(),
        )
    } else {
        None
    };

    let font_file = args
        .font
        .canonicalize()
        .unwrap_or_else(|_| args.font.clone())
        .to_string_lossy()
        .into_owned();
    let font_sha256 = crate::digest::file_sha256(&args.font)?;

    let manifest = RenderManifest {
        program_version: env!("CARGO_PKG_VERSION").to_string(),
        renderer_build: crate::build_info::RENDERER_BUILD_VERSION.to_string(),
        analyzer_build: pack.manifest.analyzer_build.clone(),
        typography: text_render.metrics,
        frames: frame_index,
        used_occluder_masks: use_masks || !foregrounds.is_empty(),
        refinement_foreground_layers,
        source_sha256: pack.manifest.source.sha256.clone(),
        canonical_text_mask: canonical_text_mask_path
            .canonicalize()
            .unwrap_or(canonical_text_mask_path.clone())
            .to_string_lossy()
            .into_owned(),
        text_style,
        style_file,
        style_sha256,
        render_contact_sheet,
        title_text: text,
        font_file,
        font_sha256,
    };
    let report_path = args.output.with_extension("render-manifest.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&manifest)?)
        .with_context(|| format!("failed to write render manifest {}", report_path.display()))?;
    println!("rendered {frame_index} frames -> {}", args.output.display());
    println!("render manifest -> {}", report_path.display());
    Ok(())
}

fn load_full_luma(path: &Path, width: u32, height: u32) -> Result<Vec<u8>> {
    let image = image::open(path)
        .with_context(|| format!("failed to load occluder mask {}", path.display()))?
        .to_luma8();
    anyhow::ensure!(
        image.width() == width && image.height() == height,
        "occluder mask dimensions differ from video"
    );
    Ok(image.into_raw())
}

fn save_luma(width: u32, height: u32, data: &[u8], path: &Path) -> Result<()> {
    let image: GrayImage = ImageBuffer::<Luma<u8>, _>::from_raw(width, height, data.to_vec())
        .context("invalid canonical text mask")?;
    image
        .save(path)
        .with_context(|| format!("failed to save canonical text mask {}", path.display()))?;
    Ok(())
}

fn evenly_spaced(frames: usize, count: usize) -> Vec<usize> {
    if frames == 0 || count == 0 {
        return Vec::new();
    }
    let count = count.min(frames);
    if count == 1 {
        return vec![0];
    }
    (0..count)
        .map(|index| index * (frames - 1) / (count - 1))
        .collect()
}

fn write_contact_sheet(frames: &[crate::surface::Surface], path: &Path) -> Result<()> {
    let Some(first) = frames.first() else {
        bail!("cannot create a render contact sheet without frames");
    };
    let tile_width = 240_u32;
    let tile_height =
        ((first.height() as f64 * tile_width as f64 / first.width() as f64).round() as u32).max(1);
    let columns = 3_u32;
    let rows = (frames.len() as u32).div_ceil(columns);
    let mut sheet = RgbaImage::new(columns * tile_width, rows * tile_height);
    for (index, frame) in frames.iter().enumerate() {
        let image = RgbaImage::from_raw(frame.width(), frame.height(), frame.pixels().to_vec())
            .context("invalid render diagnostic frame")?;
        let tile = image::imageops::resize(&image, tile_width, tile_height, FilterType::Lanczos3);
        let x = index as u32 % columns * tile_width;
        let y = index as u32 / columns * tile_height;
        image::imageops::replace(&mut sheet, &tile, i64::from(x), i64::from(y));
    }
    sheet
        .save(path)
        .with_context(|| format!("failed to save render contact sheet {}", path.display()))?;
    Ok(())
}
