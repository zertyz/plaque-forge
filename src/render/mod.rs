pub mod effects;
mod font_system;
pub mod typography;

use std::{collections::HashMap, fs, path::Path};

use anyhow::{Context, Result, bail};
use image::{RgbaImage, imageops::FilterType};
use serde::{Deserialize, Serialize};

use crate::{
    analysis::{Analysis, CONTENT_MASK_FILE, LAYERS_DIR, OCCLUDER_DIR},
    analyze::extraction::transformed_rect,
    application::{RenderRequest, TitleSource},
    image_io::load_luma,
    layers::{ForegroundReader, apply_matte_policy, merge_mask},
    model::TypographyMetrics,
    portable_path::PortablePath,
    progress::ProgressReporter,
    surface::Surface,
    video::{self, Decoder, Encoder},
};

pub const RENDER_MANIFEST_SCHEMA_VERSION: u32 = 4;
pub const DECISION_TRACE_SCHEMA_VERSION: u32 = 2;

/// The artifact identities a render manifest certifies, recomputed from the
/// exact current bytes. Consumers compare these against their own acceptance
/// policies; this loader only establishes what the bytes currently are.
pub struct RenderProvenance {
    pub manifest_path: std::path::PathBuf,
    pub manifest: RenderManifest,
    pub rendered_sha256: String,
    pub analysis_manifest_sha256: String,
    pub analysis_inputs_sha256: String,
    pub render_manifest_sha256: String,
}

/// Load the render manifest published beside a rendered video, gate its schema
/// version, and recompute every digest it certifies.
pub fn load_render_provenance(rendered: &Path, pack: &Analysis) -> Result<RenderProvenance> {
    let manifest_path = rendered.with_extension("render-manifest.json");
    let manifest_bytes = fs::read(&manifest_path)
        .with_context(|| format!("failed to read render manifest {}", manifest_path.display()))?;
    let manifest: RenderManifest = serde_json::from_slice(&manifest_bytes).with_context(|| {
        format!(
            "failed to parse render manifest {}",
            manifest_path.display()
        )
    })?;
    anyhow::ensure!(
        manifest.schema_version == RENDER_MANIFEST_SCHEMA_VERSION,
        "unsupported render manifest schema {}; expected {}",
        manifest.schema_version,
        RENDER_MANIFEST_SCHEMA_VERSION
    );
    let analysis_inputs_sha256 =
        pack.render_inputs_sha256(manifest.used_analysis_occluder_masks)?;
    Ok(RenderProvenance {
        analysis_manifest_sha256: crate::digest::file_sha256(
            &pack.root.join(crate::analysis::MANIFEST_FILE),
        )?,
        rendered_sha256: crate::digest::file_sha256(rendered)?,
        analysis_inputs_sha256,
        render_manifest_sha256: crate::digest::bytes_sha256(&manifest_bytes),
        manifest_path,
        manifest,
    })
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderManifest {
    pub schema_version: u32,
    #[serde(default)]
    pub program_version: String,
    #[serde(default)]
    pub renderer_build: String,
    pub renderer_source_sha256: String,
    #[serde(default)]
    pub analyzer_build: String,
    pub typography: TypographyMetrics,
    pub frames: usize,
    pub used_occluder_masks: bool,
    pub used_analysis_occluder_masks: bool,
    pub scene_foreground_layers: usize,
    #[serde(default)]
    pub used_injected_surface: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub injected_surface_sha256: Option<String>,
    pub source_sha256: String,
    pub analysis_manifest_sha256: String,
    pub analysis_inputs_sha256: String,
    pub rendered_sha256: String,
    pub canonical_text_mask: PortablePath,
    pub canonical_text_mask_sha256: String,
    #[serde(default)]
    pub text_style: String,
    #[serde(default)]
    pub style_file: Option<String>,
    #[serde(default)]
    pub style_sha256: Option<String>,
    #[serde(default)]
    pub render_contact_sheet: Option<PortablePath>,
    #[serde(default)]
    pub render_contact_sheet_sha256: Option<String>,
    #[serde(default)]
    pub title_text: String,
    #[serde(default)]
    pub font_file: String,
    #[serde(default)]
    pub font_sha256: String,
    #[serde(default)]
    pub encoder_args: Vec<String>,
    pub decision_trace: PortablePath,
    pub decision_trace_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderDecisionTrace {
    pub schema_version: u32,
    pub source_sha256: String,
    pub analysis_manifest_sha256: String,
    pub analysis_inputs_sha256: String,
    pub renderer_source_sha256: String,
    pub rendered_sha256: String,
    pub surface: SurfaceDecision,
    pub tracking: TrackingDecision,
    pub typography: TypographyMetrics,
    pub compositing_layers: Vec<CompositingLayerDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceDecision {
    pub id: Option<String>,
    pub selection_reason: String,
    pub reference_frame: usize,
    pub source_plaque_rect: crate::model::RectF,
    pub surface_space: crate::scene::SurfaceSpace,
    pub canonical_width: u32,
    pub canonical_height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackingDecision {
    pub trajectory_model: String,
    pub locked_keyframes: usize,
    pub guide_keyframes: usize,
    pub foreground_layers_excluded_from_tracking: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositingLayerDecision {
    pub id: String,
    pub role: crate::scene::LayerRole,
    pub affects_layout: bool,
    pub affects_tracking: bool,
    pub matte: crate::scene::LayerMatte,
}

pub fn run(
    mut args: RenderRequest,
    commands: &dyn crate::infrastructure::CommandExecutor,
) -> Result<()> {
    let final_output = args.output.clone();
    let output_name = final_output
        .file_name()
        .context("render output has no file name")?
        .to_owned();
    let final_mask = final_output.with_extension("text-mask.png");
    let final_manifest = final_output.with_extension("render-manifest.json");
    let final_trace = final_output.with_extension("decision-trace.json");
    let final_contact_sheet = args.diagnostics.as_ref().map(|directory| {
        directory.join(Path::new(&output_name).with_extension("render-contact-sheet.png"))
    });
    let stage = crate::staged_output::create(&final_output)?;
    args.output = stage.path().join(&output_name);
    args.diagnostics = final_contact_sheet
        .as_ref()
        .map(|_| stage.path().join("diagnostics"));

    let frame_count = render_to(
        args,
        &final_manifest,
        final_contact_sheet.as_deref(),
        commands,
    )?;
    let staged_output = stage.path().join(&output_name);
    let staged_mask = staged_output.with_extension("text-mask.png");
    let staged_manifest = staged_output.with_extension("render-manifest.json");
    let staged_trace = staged_output.with_extension("decision-trace.json");
    let mut members = vec![
        (staged_mask, final_mask),
        (staged_output, final_output.clone()),
        (staged_trace, final_trace.clone()),
    ];
    if let Some(final_contact_sheet) = final_contact_sheet {
        members.push((
            stage.path().join("diagnostics/render-contact-sheet.png"),
            final_contact_sheet,
        ));
    }
    // The manifest is the commit marker and must be published last.
    members.push((staged_manifest, final_manifest.clone()));
    stage.commit_files(&members, true)?;
    println!(
        "rendered {frame_count} frames -> {}",
        final_output.display()
    );
    println!("render manifest -> {}", final_manifest.display());
    println!("decision trace -> {}", final_trace.display());
    Ok(())
}

fn render_to(
    args: RenderRequest,
    manifest_reference_path: &Path,
    contact_sheet_reference_path: Option<&Path>,
    commands: &dyn crate::infrastructure::CommandExecutor,
) -> Result<usize> {
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
    let text = match args.title {
        TitleSource::Text(text) => text,
        TitleSource::File(path) => fs::read_to_string(&path)
            .with_context(|| format!("failed to read UTF-8 text file {}", path.display()))?,
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
    // Generated prompted layers belong to the selected analysis bundle. Recomputing
    // scene provenance from the scene path would accidentally consult the canonical
    // assets/analysis cache and reject a valid explicit --analysis bundle.
    let generated_layer_root = args.analysis.join(LAYERS_DIR);
    let current_scene = crate::scene::current_scene_provenance_with_generated_layer_root(
        &args.input,
        args.scene.as_deref(),
        args.surface.as_deref(),
        Some(&generated_layer_root),
    )?;
    let scenes_match = match (&pack.manifest.scenes, &current_scene) {
        (None, None) => true,
        (Some(cached), Some(current)) => cached.content_matches(current),
        _ => false,
    };
    if !scenes_match {
        bail!(
            "analysis cache does not include the current scenes\nhelp: rebuild it explicitly with `plaque-forge analyze --input {} --force`",
            args.input.display()
        );
    }
    let source = args.input;
    let info = video::probe_with(commands, &args.ffprobe, &source)?;
    info.ensure_supported_compositing_color()?;
    let mask = load_luma(
        &pack.require_asset(CONTENT_MASK_FILE)?,
        pack.manifest.canonical_width,
        pack.manifest.canonical_height,
    )?;
    let injected_surface = pack
        .manifest
        .injected_surface
        .as_ref()
        .map(|asset| {
            let path = pack.require_asset_path(asset.path.as_path())?;
            let image = image::open(&path)
                .with_context(|| format!("failed to load injected plaque {}", path.display()))?
                .to_rgba8();
            anyhow::ensure!(
                image.width() == pack.manifest.canonical_width
                    && image.height() == pack.manifest.canonical_height,
                "injected plaque dimensions do not match canonical analysis"
            );
            Surface::from_rgba(image.width(), image.height(), image.into_raw())
        })
        .transpose()?;
    progress.finish("source and analysis cache are valid");

    progress.start(2, 3, "Shape and fit typography", None);
    let style = effects::Style::load(
        args.style_file.as_deref(),
        effects::DirectStyleOptions {
            font_weight: args.font_weight,
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
    let style_file = args.style_file.as_ref().and_then(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
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
    crate::image_io::save_luma_png(
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
    validate_portable_encoder_args(&encoder_args)?;
    let mut decoder = Decoder::spawn(&args.ffmpeg, &source, &info)?;
    let mut encoder = Encoder::spawn(&args.ffmpeg, &source, &args.output, &info, &encoder_args)?;
    let masks_dir = pack.root.join(OCCLUDER_DIR);
    let use_masks = should_use_analysis_occluders(&pack) && masks_dir.is_dir();
    let analysis_inputs_sha256 = pack.render_inputs_sha256(use_masks)?;
    let authored_occluder_matte = authored_occluder_matte(&pack);
    // Opaque source masks carry semantic identity, not material alpha. When the
    // analyzer has produced a frame-local fused matte, restoring the semantic mask
    // too would turn prompt boxes/blobs into solid foreground and close porous gaps.
    let foregrounds = ForegroundReader::open(&pack, use_masks)?;
    let scene_foreground_layers = pack
        .manifest
        .layers
        .iter()
        .filter(|layer| layer.role == crate::scene::LayerRole::Foreground)
        .count();
    let mut frame_index = 0usize;
    let diagnostic_indices = crate::stats::evenly_spaced(info.frames, 12);
    let mut diagnostic_frames = Vec::with_capacity(diagnostic_indices.len());
    let static_presented = (!style.has_frame_variation()).then(|| text_render.layer.clone());
    let dynamic_target = text_render.metrics.resolved_text.clone();
    let mut dynamic_text_cache = HashMap::<String, typography::TextRender>::new();
    let needs_original_frame = use_masks || !foregrounds.is_empty();
    progress.start(3, 3, "Composite and encode", Some(info.frames));
    while let Some(mut frame) = decoder.next_frame()? {
        if frame_index >= info.frames {
            bail!(
                "decoder produced more than the expected {} source frames",
                info.frames
            );
        }
        let original = needs_original_frame.then(|| frame.clone());
        let sample = pack
            .motion
            .get(frame_index)
            .with_context(|| format!("motion sample missing for frame {frame_index}"))?;

        let plaque_quad = transformed_rect(pack.manifest.source_plaque_rect, sample.transform);
        if let Some(plaque_layer) = &injected_surface {
            frame.warp_blend(
                plaque_layer,
                plaque_quad,
                sample.plaque_visibility.clamp(0.0, 1.0) as f32,
            )?;
        }

        // Static shaping/fitting is reused. Scramble and split-flap intentionally render
        // discrete character states, cached by state string, without recomputing scene analysis.
        let time_seconds = frame_index as f64 / info.fps.max(f64::EPSILON);
        let dynamic_key = style.dynamic_text(&dynamic_target, time_seconds);
        if let Some(ref key) = dynamic_key
            && key != &dynamic_target
            && !dynamic_text_cache.contains_key(key)
        {
            let rendered = typography::render(typography::RenderRequest {
                width: pack.manifest.canonical_width,
                height: pack.manifest.canonical_height,
                mask: &mask,
                text: key,
                font_path: &args.font,
                fit_mode: crate::application::FitMode::Fixed,
                requested_font_size: Some(text_render.metrics.font_size * 0.97),
                supersampling: args.supersampling,
                target_fill: args.target_fill,
                max_lines: args.max_lines,
                padding_ratio: args.padding,
                line_height_ratio: args.line_height,
                text_align: args.text_align,
                vertical_align: args.vertical_align,
                style: &style,
            });
            if let Ok(rendered) = rendered {
                dynamic_text_cache.insert(key.clone(), rendered);
            }
        }
        let using_dynamic = dynamic_key
            .as_ref()
            .is_some_and(|key| key != &dynamic_target && dynamic_text_cache.contains_key(key));
        let frame_text = dynamic_key
            .as_ref()
            .and_then(|key| dynamic_text_cache.get(key))
            .filter(|_| using_dynamic)
            .unwrap_or(&text_render);

        let opacity =
            sample.plaque_visibility.clamp(0.0, 1.0) as f32 * style.frame_opacity(time_seconds);
        let animated_presented = if static_presented.is_none() || using_dynamic {
            let mut layer = frame_text.layer.clone();
            if let Some(overlay) = style.frame_overlay(
                &frame_text.glyph_mask,
                frame_text.layer.width(),
                frame_text.layer.height(),
                time_seconds,
            )? {
                layer.blend_surface(&overlay, 0, 0, 1.0);
            }
            Some(style.frame_transform(&layer, time_seconds)?)
        } else {
            None
        };
        let presented = if using_dynamic {
            animated_presented.as_ref()
        } else {
            static_presented.as_ref().or(animated_presented.as_ref())
        }
        .context("title presentation was not created")?;

        if style.has_surface_effects() {
            let canonical_plaque = Surface::extract_quad(
                &frame,
                plaque_quad,
                pack.manifest.canonical_width,
                pack.manifest.canonical_height,
            )?;
            let transformed_mask = style.frame_transform_mask(
                &frame_text.glyph_mask,
                frame_text.layer.width(),
                frame_text.layer.height(),
                time_seconds,
            )?;
            if let Some(surface_layer) =
                style.surface_overlay(&canonical_plaque, &transformed_mask)?
            {
                frame.warp_blend(&surface_layer, plaque_quad, opacity)?;
            }
        }

        frame.warp_blend(presented, plaque_quad, opacity)?;
        let mut restore = foregrounds
            .frame_mask(frame_index, sample.transform)?
            .unwrap_or_default();
        if use_masks {
            let path = masks_dir.join(format!("{frame_index:06}.png"));
            if path.exists() {
                let mut detail = load_full_luma(&path, info.width, info.height)?;
                if let Some(matte) = authored_occluder_matte {
                    apply_matte_policy(&mut detail, matte);
                }
                merge_mask(&mut restore, &detail);
            }
        }
        if !restore.is_empty() {
            frame.restore_from_mask(
                original
                    .as_ref()
                    .context("foreground restoration source is unavailable")?,
                &restore,
            )?;
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

    let report_path = args.output.with_extension("render-manifest.json");
    let render_contact_sheet = if let Some(diagnostics) = &args.diagnostics {
        fs::create_dir_all(diagnostics).with_context(|| {
            format!(
                "failed to create render diagnostics directory {}",
                diagnostics.display()
            )
        })?;
        let path = diagnostics.join("render-contact-sheet.png");
        write_contact_sheet(&diagnostic_frames, &path)?;
        Some(crate::portable_path::relative_reference(
            manifest_reference_path,
            contact_sheet_reference_path.context("render contact sheet has no publication path")?,
        )?)
    } else {
        None
    };
    let render_contact_sheet_sha256 = args
        .diagnostics
        .as_ref()
        .map(|directory| crate::digest::file_sha256(&directory.join("render-contact-sheet.png")))
        .transpose()?;

    let font_file = args
        .font
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "<unnamed-font>".to_string());
    let font_sha256 = crate::digest::file_sha256(&args.font)?;
    let rendered_sha256 = crate::digest::file_sha256(&args.output)?;
    let canonical_text_mask_sha256 = crate::digest::file_sha256(&canonical_text_mask_path)?;
    let analysis_manifest_sha256 =
        crate::digest::file_sha256(&pack.root.join(crate::analysis::MANIFEST_FILE))?;

    let scene_surface_id = pack
        .manifest
        .scenes
        .as_ref()
        .and_then(|scene| scene.surface_id.clone());
    let selected_surface_id = args.surface.clone().or(scene_surface_id.clone());
    let selection_reason = if args.surface.is_some() {
        "explicit-surface-request"
    } else if scene_surface_id.is_some() {
        "scene-default-surface"
    } else {
        "analysis-selected-surface"
    };
    let scene_provenance = pack.manifest.scenes.as_ref();
    let decision_trace = RenderDecisionTrace {
        schema_version: DECISION_TRACE_SCHEMA_VERSION,
        source_sha256: pack.manifest.source.sha256.clone(),
        analysis_manifest_sha256: analysis_manifest_sha256.clone(),
        analysis_inputs_sha256: analysis_inputs_sha256.clone(),
        renderer_source_sha256: crate::build_info::RENDERER_SOURCE_SHA256.to_string(),
        rendered_sha256: rendered_sha256.clone(),
        surface: SurfaceDecision {
            id: selected_surface_id,
            selection_reason: selection_reason.to_string(),
            reference_frame: pack.manifest.reference_frame,
            source_plaque_rect: pack.manifest.source_plaque_rect,
            surface_space: pack.manifest.surface_space,
            canonical_width: pack.manifest.canonical_width,
            canonical_height: pack.manifest.canonical_height,
        },
        tracking: TrackingDecision {
            trajectory_model: pack.manifest.trajectory_model.clone(),
            locked_keyframes: scene_provenance.map_or(0, |scene| scene.locked_keyframes),
            guide_keyframes: scene_provenance.map_or(0, |scene| scene.guide_keyframes),
            foreground_layers_excluded_from_tracking: pack
                .manifest
                .layers
                .iter()
                .filter(|layer| {
                    layer.role == crate::scene::LayerRole::Foreground && !layer.affects_tracking
                })
                .map(|layer| layer.id.clone())
                .collect(),
        },
        typography: text_render.metrics.clone(),
        compositing_layers: pack
            .manifest
            .layers
            .iter()
            .map(|layer| CompositingLayerDecision {
                id: layer.id.clone(),
                role: layer.role,
                affects_layout: layer.affects_layout,
                affects_tracking: layer.affects_tracking,
                matte: layer.matte,
            })
            .collect(),
    };
    let decision_trace_path = args.output.with_extension("decision-trace.json");
    fs::write(
        &decision_trace_path,
        serde_json::to_vec_pretty(&decision_trace)?,
    )
    .with_context(|| {
        format!(
            "failed to write render decision trace {}",
            decision_trace_path.display()
        )
    })?;
    let decision_trace_sha256 = crate::digest::file_sha256(&decision_trace_path)?;

    let manifest = RenderManifest {
        schema_version: RENDER_MANIFEST_SCHEMA_VERSION,
        program_version: env!("CARGO_PKG_VERSION").to_string(),
        renderer_build: crate::build_info::RENDERER_BUILD_VERSION.to_string(),
        renderer_source_sha256: crate::build_info::RENDERER_SOURCE_SHA256.to_string(),
        analyzer_build: pack.manifest.analyzer_build.clone(),
        typography: text_render.metrics,
        frames: frame_index,
        used_occluder_masks: use_masks || !foregrounds.is_empty(),
        used_analysis_occluder_masks: use_masks,
        scene_foreground_layers,
        used_injected_surface: injected_surface.is_some(),
        injected_surface_sha256: pack
            .manifest
            .injected_surface
            .as_ref()
            .map(|surface| surface.source_sha256.clone()),
        source_sha256: pack.manifest.source.sha256.clone(),
        analysis_manifest_sha256,
        analysis_inputs_sha256,
        rendered_sha256,
        canonical_text_mask: PortablePath::bundle(
            canonical_text_mask_path
                .file_name()
                .context("canonical text mask has no file name")?,
        )?,
        canonical_text_mask_sha256,
        text_style,
        style_file,
        style_sha256,
        render_contact_sheet,
        render_contact_sheet_sha256,
        title_text: text,
        font_file,
        font_sha256,
        encoder_args,
        decision_trace: PortablePath::bundle(
            decision_trace_path
                .file_name()
                .context("decision trace has no file name")?,
        )?,
        decision_trace_sha256,
    };
    fs::write(&report_path, serde_json::to_vec_pretty(&manifest)?)
        .with_context(|| format!("failed to write render manifest {}", report_path.display()))?;
    Ok(frame_index)
}

pub(crate) fn should_use_analysis_occluders(pack: &Analysis) -> bool {
    let authored_opaque_source_foreground = pack.manifest.layers.iter().any(|layer| {
        layer.role == crate::scene::LayerRole::Foreground
            && layer.coordinates == crate::scene::LayerCoordinates::SourcePixels
            && layer.matte.mode == crate::scene::LayerMatteMode::Opaque
    });
    analysis_occluders_are_renderable(
        pack.manifest.occlusion_mode,
        pack.manifest.has_occluder,
        authored_opaque_source_foreground,
    )
}

fn authored_occluder_matte(pack: &Analysis) -> Option<crate::scene::LayerMatte> {
    (pack.manifest.occlusion_mode == crate::scene::DepthMode::DeclaredOnly)
        .then(|| {
            pack.manifest.layers.iter().find_map(|layer| {
                (layer.role == crate::scene::LayerRole::Foreground
                    && layer.coordinates == crate::scene::LayerCoordinates::SourcePixels
                    && layer.matte.mode == crate::scene::LayerMatteMode::Opaque)
                    .then_some(layer.matte)
            })
        })
        .flatten()
}

fn analysis_occluders_are_renderable(
    depth: crate::scene::DepthMode,
    has_occluder: bool,
    authored_opaque_source_foreground: bool,
) -> bool {
    has_occluder
        && (depth == crate::scene::DepthMode::Automatic || authored_opaque_source_foreground)
}

pub(crate) fn load_decision_trace(
    manifest_path: &Path,
    manifest: &RenderManifest,
) -> Result<RenderDecisionTrace> {
    anyhow::ensure!(
        manifest.schema_version == RENDER_MANIFEST_SCHEMA_VERSION,
        "unsupported render manifest schema {}; expected {}",
        manifest.schema_version,
        RENDER_MANIFEST_SCHEMA_VERSION
    );
    let path = manifest.decision_trace.resolve_from(manifest_path);
    let bytes = fs::read(&path)
        .with_context(|| format!("failed to read render decision trace {}", path.display()))?;
    ensure_trace_hash(&bytes, &manifest.decision_trace_sha256, &path)?;
    let trace: RenderDecisionTrace = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse render decision trace {}", path.display()))?;
    anyhow::ensure!(
        trace.schema_version == DECISION_TRACE_SCHEMA_VERSION,
        "unsupported render decision trace schema {}; expected {}",
        trace.schema_version,
        DECISION_TRACE_SCHEMA_VERSION
    );
    anyhow::ensure!(
        trace.source_sha256 == manifest.source_sha256
            && trace.analysis_manifest_sha256 == manifest.analysis_manifest_sha256
            && trace.analysis_inputs_sha256 == manifest.analysis_inputs_sha256
            && trace.renderer_source_sha256 == manifest.renderer_source_sha256
            && trace.rendered_sha256 == manifest.rendered_sha256
            && trace.typography == manifest.typography,
        "render decision trace provenance or typography differs from its render manifest"
    );
    Ok(trace)
}

fn ensure_trace_hash(bytes: &[u8], expected: &str, path: &Path) -> Result<()> {
    let actual = crate::digest::bytes_sha256(bytes);
    anyhow::ensure!(
        actual == expected,
        "render decision trace identity changed: {}",
        path.display()
    );
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

fn validate_portable_encoder_args(args: &[String]) -> Result<()> {
    for argument in args {
        let has_absolute_fragment = argument
            .split(|character: char| {
                character.is_ascii_whitespace() || matches!(character, '=' | ',' | ';')
            })
            .any(|fragment| {
                Path::new(fragment).is_absolute()
                    || fragment.starts_with("file://")
                    || fragment
                        .as_bytes()
                        .get(1)
                        .is_some_and(|character| *character == b':')
            });
        if has_absolute_fragment || argument.contains('\\') {
            bail!(
                "encoder argument contains a non-portable path: {argument:?}; use only self-contained encoder settings"
            );
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::{analysis_occluders_are_renderable, validate_portable_encoder_args};
    use crate::scene::DepthMode;
    use crate::stats::evenly_spaced;

    #[test]
    fn diagnostic_indices_are_bounded_and_include_endpoints() {
        assert_eq!(evenly_spaced(0, 12), Vec::<usize>::new());
        assert_eq!(evenly_spaced(1, 12), vec![0]);
        assert_eq!(evenly_spaced(5, 3), vec![0, 2, 4]);
    }

    #[test]
    fn persisted_encoder_arguments_reject_workstation_paths() {
        assert!(validate_portable_encoder_args(&["-c:v".into(), "ffv1".into()]).is_ok());
        assert!(validate_portable_encoder_args(&["/home/user/lut.cube".into()]).is_err());
        assert!(
            validate_portable_encoder_args(&["lut3d=file=/opt/color/look.cube".into()]).is_err()
        );
        assert!(validate_portable_encoder_args(&[r"C:\looks\grade.cube".into()]).is_err());
    }

    #[test]
    fn declared_only_analysis_can_refine_an_authored_opaque_source_foreground() {
        assert!(analysis_occluders_are_renderable(
            DepthMode::DeclaredOnly,
            true,
            true
        ));
        assert!(!analysis_occluders_are_renderable(
            DepthMode::DeclaredOnly,
            true,
            false
        ));
        assert!(analysis_occluders_are_renderable(
            DepthMode::Automatic,
            true,
            false
        ));
        assert!(!analysis_occluders_are_renderable(
            DepthMode::Automatic,
            false,
            true
        ));
    }

    // ---- Per-style golden-mask regression tests ----
    //
    // For each shipped .toml style, renders a canonical string on a small synthetic
    // surface and compares the glyph mask against a reviewed reference. This catches
    // regressions in any effect when shared infrastructure changes.

    use std::{fs, path::Path};

    use super::{effects, font_system, typography};
    use crate::application::{FitMode, TextAlign, VerticalAlign};

    use font_system::pinned_test_font;

    fn mask_correlation(actual: &[u8], expected: &[u8]) -> f64 {
        if actual.len() != expected.len() || actual.is_empty() {
            return 0.0;
        }
        let mean_a = actual.iter().map(|&v| v as f64).sum::<f64>() / actual.len() as f64;
        let mean_e = expected.iter().map(|&v| v as f64).sum::<f64>() / expected.len() as f64;
        let mut cov = 0.0;
        let mut var_a = 0.0;
        let mut var_e = 0.0;
        for (&a, &e) in actual.iter().zip(expected.iter()) {
            let da = a as f64 - mean_a;
            let de = e as f64 - mean_e;
            cov += da * de;
            var_a += da * da;
            var_e += de * de;
        }
        let denom = (var_a * var_e).sqrt();
        if denom == 0.0 {
            if (var_a == 0.0) && (var_e == 0.0) {
                1.0
            } else {
                0.0
            }
        } else {
            cov / denom
        }
    }

    #[test]
    fn every_shipped_style_produces_a_stable_text_mask() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let styles_dir = root.join("styles");
        let fixtures_dir = root.join("tests/fixtures/style_masks");
        let font = pinned_test_font();
        let width = 64_u32;
        let height = 48_u32;
        let mask = vec![255u8; width as usize * height as usize];
        let text = "PLAQUE FORGE";
        let blessing = std::env::var("PLAQUE_FORGE_BLESS").as_deref() == Ok("1");

        let mut tested = 0;
        let mut failures = Vec::new();

        for entry in fs::read_dir(&styles_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
                continue;
            }
            let style_name = path.file_stem().unwrap().to_string_lossy().into_owned();

            let style = match effects::Style::load(
                Some(&path),
                effects::DirectStyleOptions {
                    font_weight: 600,
                    text_color: "#FFFFFFFF",
                    stroke_color: "#000000FF",
                    glow_color: "#00000000",
                    glow_radius: 0,
                    stroke_width_ratio: 0.0,
                    shadow_offset_x_ratio: 0.0,
                    shadow_offset_y_ratio: 0.0,
                    shadow_blur_radius: 0,
                    shadow_color: "#00000000",
                },
            ) {
                Ok(style) => style,
                Err(e) => {
                    failures.push(format!("{style_name}: failed to load style: {e:#}"));
                    continue;
                }
            };

            let result = match typography::render(typography::RenderRequest {
                width,
                height,
                mask: &mask,
                text,
                font_path: &font,
                fit_mode: FitMode::Artistic,
                requested_font_size: None,
                supersampling: 1,
                target_fill: 0.80,
                max_lines: 3,
                padding_ratio: 0.03,
                line_height_ratio: 1.08,
                text_align: TextAlign::Center,
                vertical_align: VerticalAlign::Center,
                style: &style,
            }) {
                Ok(result) => result,
                Err(e) => {
                    failures.push(format!("{style_name}: render failed: {e:#}"));
                    continue;
                }
            };

            let actual_mask = result.layer.alpha_mask();
            let bounds = result.layer.alpha_bounds();
            if bounds.is_none() {
                failures.push(format!(
                    "{style_name}: rendered alpha mask is completely empty"
                ));
                continue;
            }

            let reference_path = fixtures_dir.join(format!("{style_name}.expected.png"));

            if blessing {
                fs::create_dir_all(&fixtures_dir).unwrap();
                let image = image::ImageBuffer::<image::Luma<u8>, _>::from_raw(
                    width,
                    height,
                    actual_mask.clone(),
                )
                .expect("valid mask dimensions");
                image.save(&reference_path).unwrap_or_else(|e| {
                    failures.push(format!("{style_name}: failed to save reference: {e}"));
                });
            } else if reference_path.is_file() {
                let reference = image::open(&reference_path)
                    .expect("reference mask should be a valid image")
                    .to_luma8()
                    .into_raw();
                if actual_mask.len() != reference.len() {
                    failures.push(format!(
                        "{style_name}: mask size mismatch ({} vs {})",
                        actual_mask.len(),
                        reference.len()
                    ));
                } else {
                    let correlation = mask_correlation(&actual_mask, &reference);
                    if correlation < 0.65 {
                        failures.push(format!(
                            "{style_name}: structural mask correlation too low ({correlation:.3} < 0.65) \
                             [font: {}, resolved: {:?}, size: {:.2}]",
                            font.display(),
                            result.metrics.resolved_text,
                            result.metrics.font_size,
                        ));
                    }
                }
            } else {
                failures.push(format!(
                    "{style_name}: golden reference missing at {}; \
                     run with PLAQUE_FORGE_BLESS=1 to create it",
                    reference_path.display()
                ));
            }

            tested += 1;
        }

        assert!(
            tested >= 25,
            "expected at least 25 style files to be tested, found {tested}"
        );
        assert!(
            failures.is_empty(),
            "style regression failures:\n{}",
            failures.join("\n")
        );
    }
}
