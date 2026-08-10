use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
    analyze::extraction::{StructuralMatcher, StructuralRegistration, rectify, transformed_rect},
    cli::VerifyArgs,
    color::Rgba,
    image_io::{load_luma, load_rgba},
    model::{MotionSample, RectF},
    progress::ProgressReporter,
    render::RenderMetadata,
    surface::Surface,
    titlepack::{OCCLUDER_DIR, STRUCTURAL_MASK_FILE, STRUCTURAL_TEMPLATE_FILE, TitlePack},
    video::{self, Decoder},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct VerificationThresholds {
    pub overall: f64,
    pub tracking_lock: f64,
    pub scene_integrity: f64,
    pub typography_fit: f64,
    pub typography_validity: f64,
    pub temporal_stability: f64,
    pub occlusion_restore: f64,
    pub loop_seam: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VerificationReport {
    pub passed: bool,
    pub overall: f64,
    pub tracking_lock: f64,
    pub tracking_lock_basis: String,
    pub scene_integrity: f64,
    pub typography_fit: f64,
    pub typography_validity: f64,
    pub temporal_stability: f64,
    pub temporal_stability_basis: String,
    pub occlusion_restore: f64,
    pub loop_seam: f64,
    pub loop_seam_basis: String,
    pub loop_seam_mean_error: f64,
    pub title_effect_frame_mean_error: f64,
    pub structural_edge_alignment: f64,
    pub tracking_measurement_valid: bool,
    pub maximum_tracking_correction_pixels: Option<f64>,
    pub maximum_trajectory_residual_pixels: f64,
    pub worst_trajectory_frame: usize,
    pub loop_trajectory_residual_pixels: f64,
    pub untouched_region_mean_error: f64,
    pub structural_mean_error: f64,
    pub worst_tracking_frame: usize,
    pub worst_tracking_diagnostic: Option<String>,
    pub worst_scene_frame: usize,
    pub thresholds: VerificationThresholds,
    pub failures: Vec<String>,
    pub remedies: Vec<String>,
}

pub fn run(args: VerifyArgs) -> Result<()> {
    let mut progress = ProgressReporter::new(args.progress, args.progress_interval_ms);
    progress.start(1, 2, "Open verification inputs", None);
    let pack = TitlePack::open(&args.analysis)?;
    let original = args
        .original
        .clone()
        .unwrap_or_else(|| pack.manifest.source.path.clone());
    let original_info = video::probe(&args.ffprobe, &original)?;
    let rendered_info = video::probe(&args.ffprobe, &args.rendered)?;
    if !original_info.constant_frame_rate || !rendered_info.constant_frame_rate {
        bail!("verification requires constant-frame-rate source and rendered video");
    }
    if original_info.width != rendered_info.width || original_info.height != rendered_info.height {
        bail!(
            "render dimensions {}x{} differ from source {}x{}",
            rendered_info.width,
            rendered_info.height,
            original_info.width,
            original_info.height
        );
    }
    if original_info.frames != rendered_info.frames
        || (original_info.fps - rendered_info.fps).abs() > 1.0e-6
        || (original_info.start_time_seconds - rendered_info.start_time_seconds).abs() > 0.001
    {
        bail!(
            "render timing differs from source: source {} frames at {:.6} fps from {:.6}s, render {} frames at {:.6} fps from {:.6}s",
            original_info.frames,
            original_info.fps,
            original_info.start_time_seconds,
            rendered_info.frames,
            rendered_info.fps,
            rendered_info.start_time_seconds
        );
    }
    let metadata_path = args.rendered.with_extension("render-metadata.json");
    let metadata: RenderMetadata =
        serde_json::from_slice(&fs::read(&metadata_path).with_context(|| {
            format!("failed to read render metadata {}", metadata_path.display())
        })?)?;
    let text_mask_image = image::open(&metadata.canonical_text_mask)
        .with_context(|| {
            format!(
                "failed to load canonical text mask {}",
                metadata.canonical_text_mask
            )
        })?
        .to_luma8();
    anyhow::ensure!(
        text_mask_image.width() == pack.manifest.canonical_width
            && text_mask_image.height() == pack.manifest.canonical_height,
        "canonical text mask dimensions do not match title-pack"
    );
    let canonical_text_mask = text_mask_image.into_raw();
    let canonical_text_surface = Surface::from_alpha_mask(
        pack.manifest.canonical_width,
        pack.manifest.canonical_height,
        &canonical_text_mask,
        Rgba::new(255, 255, 255, 255),
    )?;
    let structural_mask = load_luma(
        &pack.require_asset(STRUCTURAL_MASK_FILE)?,
        pack.manifest.canonical_width,
        pack.manifest.canonical_height,
    )?;
    let structural_template = load_rgba(&pack.require_asset(STRUCTURAL_TEMPLATE_FILE)?)?;
    let structural_matcher = StructuralMatcher::new(&structural_template, &structural_mask);
    progress.finish("dimensions, timing, metadata and masks are valid");

    progress.start(2, 2, "Verify every frame", Some(original_info.frames));
    let mut original_decoder = Decoder::spawn(&args.ffmpeg, &original, &original_info)?;
    let mut rendered_decoder = Decoder::spawn(&args.ffmpeg, &args.rendered, &rendered_info)?;
    let mut outside_error = 0_u64;
    let mut outside_count = 0_u64;
    let mut structural_error = 0_u64;
    let mut structural_count = 0_u64;
    let mut structural_alignment_sum = 0.0_f64;
    let mut structural_alignment_count = 0_u64;
    let mut tracking_score_sum = 0.0_f64;
    let mut maximum_tracking_correction = 0.0_f64;
    let mut occlusion_error = 0_u64;
    let mut occlusion_count = 0_u64;
    let mut temporal_error = 0_u64;
    let mut temporal_count = 0_u64;
    let mut previous_delta: Option<Vec<i16>> = None;
    let mut previous_occluder: Option<Vec<u8>> = None;
    let mut first_delta: Option<Vec<i16>> = None;
    let mut last_delta = Vec::new();
    let mut first_seam_occluder: Option<Vec<u8>> = None;
    let mut last_seam_occluder = Vec::new();
    let mut worst_tracking = (0usize, 1.0f64);
    let mut worst_tracking_preview = None;
    let mut worst_scene = (0usize, 0.0f64);
    let mut frame_index = 0usize;

    loop {
        let Some(original_frame) = original_decoder.next_frame()? else {
            break;
        };
        let rendered_frame = rendered_decoder
            .next_frame()?
            .context("render ended before source")?;
        let sample = pack
            .motion
            .get(frame_index)
            .with_context(|| format!("motion sample missing for frame {frame_index}"))?;

        let mut allowed = Surface::new(original_info.width, original_info.height);
        allowed.warp_blend(
            &canonical_text_surface,
            transformed_rect(pack.manifest.source_plaque_rect, sample.transform),
            1.0,
        )?;
        let allowed_mask = allowed.alpha_mask();
        let mut frame_outside_error = 0u64;
        let mut frame_outside_count = 0u64;
        for (&allowed_alpha, (source, rendered)) in allowed_mask.iter().zip(
            original_frame
                .pixels()
                .chunks_exact(4)
                .zip(rendered_frame.pixels().chunks_exact(4)),
        ) {
            if allowed_alpha == 0 {
                let difference = (0..3)
                    .map(|channel| source[channel].abs_diff(rendered[channel]) as u64)
                    .sum::<u64>();
                outside_error += difference;
                outside_count += 3;
                frame_outside_error += difference;
                frame_outside_count += 3;
            }
        }
        let frame_scene_mean = frame_outside_error as f64 / frame_outside_count.max(1) as f64;
        if frame_scene_mean > worst_scene.1 {
            worst_scene = (frame_index, frame_scene_mean);
        }

        let original_canonical = rectify(
            &original_frame,
            pack.manifest.source_plaque_rect,
            sample.transform,
        )?;
        for (&mask, (observed, template)) in structural_mask.iter().zip(
            original_canonical
                .pixels()
                .chunks_exact(4)
                .zip(structural_template.pixels().chunks_exact(4)),
        ) {
            if mask > 64 {
                let difference = (0..3)
                    .map(|channel| observed[channel].abs_diff(template[channel]) as u64)
                    .sum::<u64>();
                structural_error += difference;
                structural_count += 3;
            }
        }
        let frame_tracking_score = if sample.plaque_visibility >= 0.5 {
            let alignment = structural_edge_alignment(
                &original_canonical,
                &structural_template,
                &structural_mask,
            );
            structural_alignment_sum += alignment;
            structural_alignment_count += 1;
            let correction = structural_matcher
                .as_ref()
                .and_then(|matcher| matcher.measure(&original_canonical, 4))
                .map(|registration| {
                    if registration.after + 0.25 < registration.before {
                        registration_correction_pixels(
                            &registration,
                            original_canonical.width(),
                            original_canonical.height(),
                        )
                    } else {
                        0.0
                    }
                })
                .unwrap_or(f64::INFINITY);
            maximum_tracking_correction = maximum_tracking_correction.max(correction);
            let score = tracking_lock_score(correction, alignment);
            tracking_score_sum += score;
            if score < worst_tracking.1 {
                worst_tracking = (frame_index, score);
                worst_tracking_preview = Some((
                    original_frame.clone(),
                    transformed_rect(pack.manifest.source_plaque_rect, sample.transform),
                ));
            }
            score
        } else {
            1.0
        };

        let canonical_occluder = if pack.manifest.has_occluder {
            let path = pack
                .root
                .join(OCCLUDER_DIR)
                .join(format!("{frame_index:06}.png"));
            if path.exists() {
                let mask = image::open(&path)?.to_luma8();
                anyhow::ensure!(
                    mask.width() == original_info.width && mask.height() == original_info.height,
                    "occluder mask dimensions differ from source at frame {frame_index}"
                );
                for ((&alpha, source), rendered) in mask
                    .as_raw()
                    .iter()
                    .zip(original_frame.pixels().chunks_exact(4))
                    .zip(rendered_frame.pixels().chunks_exact(4))
                {
                    if alpha >= 250 {
                        occlusion_error += (0..3)
                            .map(|channel| source[channel].abs_diff(rendered[channel]) as u64)
                            .sum::<u64>();
                        occlusion_count += 3;
                    }
                }
                let full_mask = Surface::from_alpha_mask(
                    original_info.width,
                    original_info.height,
                    mask.as_raw(),
                    Rgba::new(255, 255, 255, 255),
                )?;
                Some(
                    rectify(
                        &full_mask,
                        pack.manifest.source_plaque_rect,
                        sample.transform,
                    )?
                    .alpha_mask(),
                )
            } else {
                None
            }
        } else {
            None
        };

        let rendered_canonical = rectify(
            &rendered_frame,
            pack.manifest.source_plaque_rect,
            sample.transform,
        )?;
        let delta: Vec<i16> = rendered_canonical
            .pixels()
            .chunks_exact(4)
            .zip(original_canonical.pixels().chunks_exact(4))
            .flat_map(|(rendered, source)| {
                (0..3).map(move |channel| rendered[channel] as i16 - source[channel] as i16)
            })
            .collect();
        if let Some(previous) = &previous_delta {
            for (pixel_index, (&text_alpha, (current, prior))) in canonical_text_mask
                .iter()
                .zip(delta.chunks_exact(3).zip(previous.chunks_exact(3)))
                .enumerate()
            {
                let current_occluded = canonical_occluder
                    .as_ref()
                    .is_some_and(|mask| mask[pixel_index] >= 250);
                let previous_occluded = previous_occluder
                    .as_ref()
                    .is_some_and(|mask| mask[pixel_index] >= 250);
                if text_alpha > 32 && !current_occluded && !previous_occluded {
                    temporal_error += (0..3)
                        .map(|channel| current[channel].abs_diff(prior[channel]) as u64)
                        .sum::<u64>();
                    temporal_count += 3;
                }
            }
        }
        if first_delta.is_none() {
            first_delta = Some(delta.clone());
            first_seam_occluder = canonical_occluder.clone();
        }
        last_delta.clone_from(&delta);
        last_seam_occluder = canonical_occluder.clone().unwrap_or_default();
        previous_delta = Some(delta);
        previous_occluder = canonical_occluder;
        frame_index += 1;
        progress.update(
            frame_index,
            format!(
                "structural lock {:.2}, outside error {:.4}",
                frame_tracking_score, frame_scene_mean
            ),
        );
    }
    original_decoder.finish()?;
    if rendered_decoder.next_frame()?.is_some() {
        bail!("render contains frames after the source ended");
    }
    rendered_decoder.finish()?;
    progress.finish(format!("{} frames", frame_index));

    let untouched_error = outside_error as f64 / outside_count.max(1) as f64;
    let structure_mean = structural_error as f64 / structural_count.max(1) as f64;
    let temporal_mean = temporal_error as f64 / temporal_count.max(1) as f64;
    let structural_edge_alignment = if structural_alignment_count == 0 {
        0.0
    } else {
        (structural_alignment_sum / structural_alignment_count as f64).clamp(0.0, 1.0)
    };
    let measured_tracking_lock = if structural_alignment_count == 0 {
        0.0
    } else {
        (tracking_score_sum / structural_alignment_count as f64).clamp(0.0, 1.0)
    };
    let supervised_track = pack
        .manifest
        .motion_model
        .starts_with("supervised-quad-csv-")
        || pack
            .manifest
            .motion_model
            .starts_with("authoritative-human-quad-track-");
    let tracking_measurement_valid = maximum_tracking_correction.is_finite();
    let tracking_lock = if supervised_track {
        1.0
    } else {
        measured_tracking_lock
    };
    let tracking_lock_basis = if pack
        .manifest
        .motion_model
        .starts_with("authoritative-human-quad-track-")
    {
        "authoritative-human-quad-track"
    } else if supervised_track {
        "authoritative-supervised-quad-track"
    } else {
        "automatic-structural-registration-and-edge-alignment"
    };
    let scene_integrity = (-untouched_error / 1.5).exp().clamp(0.0, 1.0);
    let trajectory = trajectory_quality(
        &pack.motion,
        pack.manifest.source_plaque_rect,
        pack.manifest.loop_closed,
    );
    let temporal_stability = trajectory.temporal_score;
    let occlusion_restore = if occlusion_count == 0 {
        if pack.manifest.has_occluder {
            0.40
        } else {
            1.0
        }
    } else {
        (-(occlusion_error as f64 / occlusion_count as f64) / 1.5)
            .exp()
            .clamp(0.0, 1.0)
    };
    let seam_error = if pack.manifest.loop_closed {
        canonical_seam_error(
            first_delta.as_deref(),
            &last_delta,
            &canonical_text_mask,
            first_seam_occluder.as_deref(),
            &last_seam_occluder,
        )
    } else {
        0.0
    };
    let loop_seam = if pack.manifest.loop_closed {
        trajectory.loop_score
    } else {
        1.0
    };
    let (typography_fit, typography_validity) = typography_scores(&metadata);

    let overall = weighted_geometric_mean(&[
        (tracking_lock, 0.24),
        (scene_integrity, 0.22),
        (typography_fit, 0.14),
        (typography_validity, 0.10),
        (temporal_stability, 0.12),
        (occlusion_restore, 0.10),
        (loop_seam, 0.08),
    ]);
    let thresholds = VerificationThresholds {
        overall: args.minimum_score,
        tracking_lock: 0.95,
        scene_integrity: 0.995,
        typography_fit: 0.98,
        typography_validity: 1.0,
        temporal_stability: 0.95,
        occlusion_restore: 0.95,
        loop_seam: 0.98,
    };
    let mut failures = Vec::new();
    let mut remedies = Vec::new();
    let worst_tracking_diagnostic = if supervised_track {
        None
    } else if let (Some(directory), Some((mut frame, quad))) =
        (&args.diagnostics, worst_tracking_preview)
    {
        fs::create_dir_all(directory).with_context(|| {
            format!(
                "failed to create verification diagnostics {}",
                directory.display()
            )
        })?;
        draw_quad(&mut frame, quad, Rgba::new(255, 220, 0, 255));
        let path = directory.join("verification-worst-tracking-frame.png");
        save_surface(&frame, &path)?;
        Some(path.display().to_string())
    } else {
        None
    };
    let rect = pack.manifest.source_plaque_rect;
    let frame_seconds = worst_tracking.0 as f64 / original_info.fps;
    let tracking_remedy = if tracking_measurement_valid {
        format!(
            "automatic registration needs up to {:.2}px correction; worst frame {} ({frame_seconds:.3}s){}. The analyzed rectangle is {:.0},{:.0},{:.0},{:.0}. If it does not follow the plaque edges, rerun with a corrected --plaque-hint x,y,width,height; if it does, use a reviewed --track-csv because temporal smoothing cannot fix spatial misregistration",
            maximum_tracking_correction,
            worst_tracking.0,
            worst_tracking_diagnostic
                .as_ref()
                .map(|path| format!(", saved as {path}"))
                .unwrap_or_default(),
            rect.x,
            rect.y,
            rect.width,
            rect.height
        )
    } else {
        format!(
            "tracking could not be measured because the analyzed rectangle {:.0},{:.0},{:.0},{:.0} has no usable structural template; inspect canonical-reference.png and candidate.png when present, then rerun with --plaque-hint x,y,width,height around the full plaque",
            rect.x, rect.y, rect.width, rect.height
        )
    };
    check_score(
        "tracking_lock",
        tracking_lock,
        thresholds.tracking_lock,
        &mut failures,
        &mut remedies,
        tracking_remedy,
    );
    check_score(
        "scene_integrity",
        scene_integrity,
        thresholds.scene_integrity,
        &mut failures,
        &mut remedies,
        "use the default lossless FFV1 output and confirm the source plaque is text-free".into(),
    );
    check_score(
        "typography_fit",
        typography_fit,
        thresholds.typography_fit,
        &mut failures,
        &mut remedies,
        "use --fit maximize; if it still fails, reduce --padding, increase --max-lines, or choose a narrower font".into(),
    );
    check_score(
        "typography_validity",
        typography_validity,
        thresholds.typography_validity,
        &mut failures,
        &mut remedies,
        "choose a font containing every requested glyph; no fallback font is allowed".into(),
    );
    check_score(
        "temporal_stability",
        temporal_stability,
        thresholds.temporal_stability,
        &mut failures,
        &mut remedies,
        format!(
            "trajectory changes abruptly at frame {} ({:.3}s); current --tracking-inertia is {:.2}. Raise it toward 0.50 for more smoothing, or lower it toward 0.20 only when the title visibly lags genuine plaque motion; every frame is measured",
            trajectory.worst_frame,
            trajectory.worst_frame as f64 / original_info.fps,
            tracking_inertia(&pack.manifest.motion_model).unwrap_or(0.35)
        ),
    );
    check_score(
        "occlusion_restore",
        occlusion_restore,
        thresholds.occlusion_restore,
        &mut failures,
        &mut remedies,
        "re-run analyze with a different --occlusion-sensitivity, or --disable-occlusion if nothing crosses the plaque".into(),
    );
    check_score(
        "loop_seam",
        loop_seam,
        thresholds.loop_seam,
        &mut failures,
        &mut remedies,
        "re-run analyze with --loop-closure on for a known loop".into(),
    );
    if overall < thresholds.overall {
        failures.push(format!(
            "overall score {overall:.4} is below {:.4}",
            thresholds.overall
        ));
    }
    let passed = failures.is_empty();
    let report = VerificationReport {
        passed,
        overall,
        tracking_lock,
        tracking_lock_basis: tracking_lock_basis.to_string(),
        scene_integrity,
        typography_fit,
        typography_validity,
        temporal_stability,
        temporal_stability_basis: "quad-and-visibility-trajectory-curvature".to_string(),
        occlusion_restore,
        loop_seam,
        loop_seam_basis: "circular-trajectory-curvature".to_string(),
        loop_seam_mean_error: seam_error,
        title_effect_frame_mean_error: temporal_mean,
        structural_edge_alignment,
        tracking_measurement_valid,
        maximum_tracking_correction_pixels: tracking_measurement_valid
            .then_some(maximum_tracking_correction),
        maximum_trajectory_residual_pixels: trajectory.maximum_residual,
        worst_trajectory_frame: trajectory.worst_frame,
        loop_trajectory_residual_pixels: trajectory.loop_residual,
        untouched_region_mean_error: untouched_error,
        structural_mean_error: structure_mean,
        worst_tracking_frame: if supervised_track {
            0
        } else {
            worst_tracking.0
        },
        worst_tracking_diagnostic,
        worst_scene_frame: worst_scene.0,
        thresholds,
        failures,
        remedies,
    };
    let json = serde_json::to_string_pretty(&report)?;
    if let Some(path) = args.report {
        fs::write(&path, &json)
            .with_context(|| format!("failed to write verification report {}", path.display()))?;
    }
    println!("{json}");
    if !report.passed {
        bail!(
            "verification failed with score {:.3}; follow the remedies in the verification report",
            report.overall
        );
    }
    Ok(())
}

fn typography_scores(metadata: &RenderMetadata) -> (f64, f64) {
    let metrics = &metadata.typography;
    let fit = if metrics.fit_mode == "maximize" {
        (metrics.font_size as f64 / metrics.maximum_safe_font_size.max(0.001) as f64)
            .clamp(0.0, 1.0)
    } else {
        1.0
    };
    let validity = if metrics.missing_glyphs == 0
        && metrics.fallback_glyphs == 0
        && metrics.clipped_pixels == 0
    {
        1.0
    } else {
        0.0
    };
    (fit, validity)
}

fn tracking_inertia(model: &str) -> Option<f64> {
    model.rsplit_once("inertia-")?.1.parse().ok()
}

fn save_surface(surface: &Surface, path: &Path) -> Result<()> {
    let image =
        image::RgbaImage::from_raw(surface.width(), surface.height(), surface.pixels().to_vec())
            .context("invalid verification diagnostic image")?;
    image
        .save(path)
        .with_context(|| format!("failed to save verification diagnostic {}", path.display()))?;
    Ok(())
}

fn draw_quad(surface: &mut Surface, quad: crate::geometry::Quad, color: Rgba) {
    let points = quad.points();
    for index in 0..4 {
        draw_line(surface, points[index], points[(index + 1) % 4], color);
    }
}

fn draw_line(
    surface: &mut Surface,
    start: crate::geometry::Point,
    end: crate::geometry::Point,
    color: Rgba,
) {
    let steps = (end.x - start.x)
        .abs()
        .max((end.y - start.y).abs())
        .ceil()
        .max(1.0) as usize;
    for step in 0..=steps {
        let t = step as f64 / steps as f64;
        let x = (start.x + (end.x - start.x) * t).round() as i32;
        let y = (start.y + (end.y - start.y) * t).round() as i32;
        for dy in -2..=2 {
            for dx in -2..=2 {
                if x + dx >= 0 && y + dy >= 0 {
                    surface.set_pixel((x + dx) as u32, (y + dy) as u32, color);
                }
            }
        }
    }
}

fn structural_edge_alignment(observed: &Surface, template: &Surface, mask: &[u8]) -> f64 {
    let width = observed.width() as usize;
    let height = observed.height() as usize;
    if width != template.width() as usize
        || height != template.height() as usize
        || mask.len() != width * height
        || width < 3
        || height < 3
    {
        return 0.0;
    }

    let luma = |surface: &Surface, index: usize| -> f64 {
        let pixel = &surface.pixels()[index * 4..index * 4 + 3];
        (f64::from(pixel[0]) * 54.0 + f64::from(pixel[1]) * 183.0 + f64::from(pixel[2]) * 19.0)
            / 256.0
    };
    let mut weighted_score = 0.0;
    let mut total_weight = 0.0;
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let index = y * width + x;
            if mask[index] <= 64 {
                continue;
            }
            let template_x = luma(template, index + 1) - luma(template, index - 1);
            let template_y = luma(template, index + width) - luma(template, index - width);
            let template_magnitude = template_x.hypot(template_y);
            if template_magnitude < 8.0 {
                continue;
            }
            let observed_x = luma(observed, index + 1) - luma(observed, index - 1);
            let observed_y = luma(observed, index + width) - luma(observed, index - width);
            let observed_magnitude = observed_x.hypot(observed_y);
            let direction = ((template_x * observed_x + template_y * observed_y)
                / (template_magnitude * observed_magnitude).max(1.0))
            .clamp(0.0, 1.0);
            let presence = (observed_magnitude / 8.0).clamp(0.0, 1.0);
            let weight = f64::from(mask[index]) / 255.0 * template_magnitude.min(255.0);
            weighted_score += direction * presence * weight;
            total_weight += weight;
        }
    }
    if total_weight <= f64::EPSILON {
        0.0
    } else {
        (weighted_score / total_weight).clamp(0.0, 1.0)
    }
}

fn registration_correction_pixels(
    registration: &StructuralRegistration,
    width: u32,
    height: u32,
) -> f64 {
    if registration.after + 0.25 >= registration.before {
        return 0.0;
    }
    let center_x = f64::from(width.saturating_sub(1)) * 0.5;
    let center_y = f64::from(height.saturating_sub(1)) * 0.5;
    let corners = [
        (0.0, 0.0),
        (f64::from(width.saturating_sub(1)), 0.0),
        (
            f64::from(width.saturating_sub(1)),
            f64::from(height.saturating_sub(1)),
        ),
        (0.0, f64::from(height.saturating_sub(1))),
    ];
    corners
        .iter()
        .map(|&(x, y)| {
            let corrected_x = center_x + (x - center_x) * registration.scale + registration.dx;
            let corrected_y = center_y + (y - center_y) * registration.scale + registration.dy;
            (corrected_x - x).hypot(corrected_y - y)
        })
        .sum::<f64>()
        / corners.len() as f64
}

fn registration_lock_score(correction_pixels: f64) -> f64 {
    if !correction_pixels.is_finite() {
        return 0.0;
    }
    let excess = (correction_pixels - 0.75).max(0.0);
    (-(excess / 2.0).powi(2)).exp().clamp(0.0, 1.0)
}

fn tracking_lock_score(correction_pixels: f64, edge_alignment: f64) -> f64 {
    registration_lock_score(correction_pixels).max(edge_alignment.clamp(0.0, 1.0).sqrt())
}

fn canonical_seam_error(
    first: Option<&[i16]>,
    last: &[i16],
    text_mask: &[u8],
    first_occluder: Option<&[u8]>,
    last_occluder: &[u8],
) -> f64 {
    let Some(first) = first else {
        return 255.0;
    };
    if first.len() != last.len() || first.len() != text_mask.len() * 3 {
        return 255.0;
    }
    let mut error = 0_u64;
    let mut count = 0_u64;
    for (pixel, &text_alpha) in text_mask.iter().enumerate() {
        let first_hidden = first_occluder.is_some_and(|mask| mask[pixel] >= 250);
        let last_hidden = last_occluder.get(pixel).is_some_and(|&alpha| alpha >= 250);
        if text_alpha <= 32 || first_hidden || last_hidden {
            continue;
        }
        for channel in 0..3 {
            let index = pixel * 3 + channel;
            error += (first[index] - last[index]).unsigned_abs() as u64;
            count += 1;
        }
    }
    if count == 0 {
        255.0
    } else {
        error as f64 / count as f64
    }
}

struct TrajectoryQuality {
    temporal_score: f64,
    loop_score: f64,
    maximum_residual: f64,
    worst_frame: usize,
    loop_residual: f64,
}

fn trajectory_quality(
    samples: &[MotionSample],
    rect: RectF,
    loop_closed: bool,
) -> TrajectoryQuality {
    if samples.len() < 3 {
        return TrajectoryQuality {
            temporal_score: 0.0,
            loop_score: if loop_closed { 0.0 } else { 1.0 },
            maximum_residual: f64::INFINITY,
            worst_frame: 0,
            loop_residual: if loop_closed { f64::INFINITY } else { 0.0 },
        };
    }

    let quads: Vec<_> = samples
        .iter()
        .map(|sample| transformed_rect(rect, sample.transform))
        .collect();
    let mut residuals = Vec::with_capacity(samples.len());
    let mut maximum_residual = 0.0_f64;
    let mut worst_frame = 0usize;
    let first = if loop_closed { 0 } else { 1 };
    let end = if loop_closed {
        samples.len()
    } else {
        samples.len() - 1
    };
    for index in first..end {
        let previous = if index == 0 {
            samples.len() - 1
        } else {
            index - 1
        };
        let next = if index + 1 == samples.len() {
            0
        } else {
            index + 1
        };
        let mut residual = 0.0;
        for ((point, before), after) in quads[index]
            .points()
            .into_iter()
            .zip(quads[previous].points())
            .zip(quads[next].points())
        {
            let expected_x = (before.x + after.x) * 0.5;
            let expected_y = (before.y + after.y) * 0.5;
            residual += (point.x - expected_x).hypot(point.y - expected_y);
        }
        residual /= 4.0;

        // Visibility is part of the rendered title trajectory. Convert abrupt
        // opacity curvature to a small pixel-equivalent penalty without
        // penalizing a smooth intentional fade.
        let expected_visibility =
            (samples[previous].plaque_visibility + samples[next].plaque_visibility) * 0.5;
        residual =
            residual.hypot((samples[index].plaque_visibility - expected_visibility).abs() * 8.0);
        if residual > maximum_residual {
            maximum_residual = residual;
            worst_frame = index;
        }
        residuals.push((index, residual));
    }

    let score_for = |residual: f64| {
        let excess = (residual - 0.35).max(0.0);
        (-(excess / 1.5).powi(2)).exp().clamp(0.0, 1.0)
    };
    let temporal_score = residuals
        .iter()
        .map(|(_, residual)| score_for(*residual))
        .sum::<f64>()
        / residuals.len().max(1) as f64;
    let loop_residual = if loop_closed {
        residuals
            .iter()
            .filter(|(index, _)| *index == 0 || *index + 1 == samples.len())
            .map(|(_, residual)| *residual)
            .fold(0.0, f64::max)
    } else {
        0.0
    };

    TrajectoryQuality {
        temporal_score,
        loop_score: if loop_closed {
            score_for(loop_residual)
        } else {
            1.0
        },
        maximum_residual,
        worst_frame,
        loop_residual,
    }
}

fn check_score(
    name: &str,
    value: f64,
    threshold: f64,
    failures: &mut Vec<String>,
    remedies: &mut Vec<String>,
    remedy: String,
) {
    if value + 1.0e-12 < threshold {
        failures.push(format!("{name} {value:.4} is below {threshold:.4}"));
        remedies.push(remedy);
    }
}

fn weighted_geometric_mean(values: &[(f64, f64)]) -> f64 {
    let total_weight: f64 = values.iter().map(|(_, weight)| weight).sum();
    let weighted_log_sum: f64 = values
        .iter()
        .map(|(value, weight)| value.clamp(1.0e-9, 1.0).ln() * weight)
        .sum();
    (weighted_log_sum / total_weight).exp()
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_seam_error, registration_correction_pixels, registration_lock_score,
        structural_edge_alignment, tracking_lock_score, trajectory_quality,
    };
    use crate::{
        analyze::extraction::measure_structural_registration,
        color::Rgba,
        model::{Mat3, MotionSample, RectF},
        surface::Surface,
    };

    fn motion(frame: usize, x: f64) -> MotionSample {
        MotionSample {
            frame,
            transform: Mat3::translation(x, 0.0),
            inlier_ratio: 1.0,
            reprojection_error: 0.0,
            ecc: Some(1.0),
            plaque_visibility: 1.0,
            occluder_coverage: 0.0,
        }
    }

    #[test]
    fn canonical_seam_ignores_pixels_outside_text_and_occlusion() {
        let first = [10_i16, 20, 30, 80, 90, 100];
        let last = [10_i16, 20, 30, 0, 0, 0];
        let text = [255_u8, 255];
        let occluder = [0_u8, 255];
        assert_eq!(
            canonical_seam_error(Some(&first), &last, &text, Some(&occluder), &occluder),
            0.0
        );
    }

    #[test]
    fn structural_alignment_rewards_registered_edges() {
        let mut template = Surface::new(16, 16);
        let mut shifted = Surface::new(16, 16);
        for y in 0..16 {
            for x in 0..16 {
                let value = if x >= 8 { 240 } else { 10 };
                template.set_pixel(x, y, Rgba::new(value, value, value, 255));
                let shifted_value = if x >= 11 { 240 } else { 10 };
                shifted.set_pixel(
                    x,
                    y,
                    Rgba::new(shifted_value, shifted_value, shifted_value, 255),
                );
            }
        }
        let mut mask = vec![0_u8; 16 * 16];
        for y in 1..15 {
            mask[y * 16 + 7] = 255;
            mask[y * 16 + 8] = 255;
        }
        let registered = structural_edge_alignment(&template, &template, &mask);
        let displaced = structural_edge_alignment(&shifted, &template, &mask);
        assert!(registered > 0.99);
        assert!(displaced < 0.10);
    }

    #[test]
    fn trajectory_score_rejects_a_single_frame_jump() {
        let rect = RectF {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 50.0,
        };
        let smooth = (0..9)
            .map(|frame| motion(frame, frame as f64))
            .collect::<Vec<_>>();
        let mut jumped = smooth.clone();
        jumped[4].transform = Mat3::translation(8.0, 0.0);

        assert!(trajectory_quality(&smooth, rect, false).temporal_score > 0.99);
        assert!(trajectory_quality(&jumped, rect, false).temporal_score < 0.80);
    }

    #[test]
    fn registration_score_rejects_a_displaced_structure() {
        let mut template = Surface::new(64, 64);
        let mut shifted = Surface::new(64, 64);
        let mut mask = vec![0_u8; 64 * 64];
        for y in 8..56 {
            for x in 8..56 {
                let edge = x == 8 || x == 55 || y == 8 || y == 55;
                if edge {
                    template.set_pixel(x, y, Rgba::new(240, 240, 240, 255));
                    shifted.set_pixel(x + 3, y, Rgba::new(240, 240, 240, 255));
                }
                if edge {
                    mask[y as usize * 64 + x as usize] = 255;
                }
            }
        }
        let registration = measure_structural_registration(&template, &shifted, &mask, 4).unwrap();
        let correction = registration_correction_pixels(&registration, 64, 64);

        assert!(registration.after < registration.before);
        assert!(correction > 2.0);
        assert!(registration_lock_score(correction) < 0.80);
        assert!(tracking_lock_score(correction, 0.20) < 0.80);
        assert!(tracking_lock_score(correction, 0.98) > 0.95);
    }
}
