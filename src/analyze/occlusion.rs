//! Automatic foreground-occlusion analysis.
//!
//! Detects source content that passes in front of the plaque so those pixels can be
//! restored over newly rendered text during compositing.

use std::{collections::VecDeque, fs, path::Path};

use anyhow::{Context, Result};

use crate::{
    analysis::{
        AUTHORED_OCCLUDER_WORK_DIR, AUTOMATIC_MATERIAL_WORK_DIR, AUTOMATIC_OCCLUDER_WORK_DIR,
        OCCLUDER_DIR,
    },
    color::Rgba,
    layers::{self, LayerInput},
    model::{MotionSample, RectF},
    progress::ProgressReporter,
    scene::SurfaceTrajectory,
    surface::Surface,
    video::{Decoder, VideoInfo},
};

use super::{
    extraction::{ExtractionResult, rectify, transformed_rect},
    tracking,
};
use crate::geometry::Quad;
use crate::image_io::save_luma_png;
use crate::stats::mean;

pub struct OcclusionResult {
    pub has_occluder: bool,
    pub confidence: f64,
    pub mean_coverage: f64,
}

/// Recompute the public foreground diagnostics from the exact masks that the
/// renderer will consume. This is deliberately separate from ML's own result:
/// semantic confidence cannot certify optical opacity, and reports must never
/// describe a pre-fusion intermediate while rendering a different artifact.
#[allow(clippy::too_many_arguments)]
pub fn summarize_installed_masks(
    info: &VideoInfo,
    rect: RectF,
    motion: &mut [MotionSample],
    extraction: &ExtractionResult,
    output_root: &Path,
    diagnostics: &Path,
) -> Result<OcclusionResult> {
    let masks_dir = output_root.join("occluder");
    let mut coverages = Vec::with_capacity(info.frames);
    let mut content_coverages = Vec::with_capacity(info.frames);
    let mut canonical_masks = Vec::with_capacity(info.frames);
    let content_weight = extraction
        .content_mask
        .iter()
        .map(|&value| f64::from(value) / 255.0)
        .sum::<f64>()
        .max(1.0);

    for (frame_index, sample) in motion.iter_mut().take(info.frames).enumerate() {
        let path = masks_dir.join(format!("{frame_index:06}.png"));
        let source = image::open(&path)
            .with_context(|| {
                format!(
                    "failed to load installed foreground mask {}",
                    path.display()
                )
            })?
            .to_luma8();
        if source.dimensions() != (info.width, info.height) {
            anyhow::bail!(
                "installed foreground mask {} is {}x{}, expected {}x{}",
                path.display(),
                source.width(),
                source.height(),
                info.width,
                info.height
            );
        }
        let full = Surface::from_alpha_mask(
            info.width,
            info.height,
            source.as_raw(),
            Rgba::new(255, 255, 255, 255),
        )?;
        let canonical = rectify(&full, rect, sample.transform)?;
        let alpha = canonical.alpha_mask();
        let coverage = alpha
            .iter()
            .map(|&value| f64::from(value) / 255.0)
            .sum::<f64>()
            / alpha.len().max(1) as f64;
        let content_coverage = alpha
            .iter()
            .zip(&extraction.content_mask)
            .map(|(&foreground, &content)| {
                f64::from(foreground) / 255.0 * f64::from(content) / 255.0
            })
            .sum::<f64>()
            / content_weight;
        sample.occluder_coverage = content_coverage;
        coverages.push(coverage);
        content_coverages.push(content_coverage);
        canonical_masks.push(alpha);
    }

    let mut temporal_agreement = Vec::new();
    for pair in canonical_masks.windows(2) {
        if let Some(iou) = mask_iou(&pair[0], &pair[1]) {
            temporal_agreement.push(iou);
        }
    }
    let max_coverage = coverages.iter().copied().fold(0.0, f64::max);
    let mean_coverage = mean(&coverages);
    let occupied_frames = coverages
        .iter()
        .filter(|&&coverage| coverage > 0.0025)
        .count();
    let occupied_ratio = occupied_frames as f64 / coverages.len().max(1) as f64;
    let agreement = mean(&temporal_agreement);
    let persistence = if max_coverage > 0.0 {
        (mean_coverage / max_coverage).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let has_occluder = classify_occluder(max_coverage, occupied_ratio, agreement, persistence);
    let confidence = occluder_confidence(
        has_occluder,
        max_coverage,
        occupied_ratio,
        agreement,
        persistence,
    );

    let summary_path = diagnostics.join("occlusion-summary.json");
    let mut summary: serde_json::Value = serde_json::from_slice(
        &fs::read(&summary_path)
            .with_context(|| format!("failed to read {}", summary_path.display()))?,
    )?;
    let object = summary
        .as_object_mut()
        .context("occlusion summary is not a JSON object")?;
    let seed = serde_json::json!({
        "mask_basis": object.get("mask_basis").cloned().unwrap_or(serde_json::Value::Null),
        "confidence": object.get("confidence").cloned().unwrap_or(serde_json::Value::Null),
        "mean_coverage": object.get("mean_coverage").cloned().unwrap_or(serde_json::Value::Null),
        "max_coverage": object.get("max_coverage").cloned().unwrap_or(serde_json::Value::Null),
        "occupied_frames": object.get("occupied_frames").cloned().unwrap_or(serde_json::Value::Null),
        "occupied_ratio": object.get("occupied_ratio").cloned().unwrap_or(serde_json::Value::Null),
        "mean_content_occlusion": object.get("mean_content_occlusion").cloned().unwrap_or(serde_json::Value::Null),
    });
    object.insert("has_occluder".into(), has_occluder.into());
    object.insert("confidence".into(), confidence.into());
    object.insert("mean_coverage".into(), mean_coverage.into());
    object.insert("max_coverage".into(), max_coverage.into());
    object.insert("occupied_frames".into(), occupied_frames.into());
    object.insert("occupied_ratio".into(), occupied_ratio.into());
    object.insert("coverage_persistence".into(), persistence.into());
    object.insert("nonempty_adjacent_mask_iou".into(), agreement.into());
    object.insert(
        "mean_content_occlusion".into(),
        mean(&content_coverages).into(),
    );
    object.insert(
        "mask_basis".into(),
        "lossless-photometric-material-intersected-with-semantic-two-pixel-object-support".into(),
    );
    object.insert("mask_coordinates".into(), "source-pixels".into());
    object.insert(
        "mask_frames_summarized".into(),
        canonical_masks.len().into(),
    );
    object.insert("summary_matches_installed_masks".into(), true.into());
    object.insert("photometric_seed_statistics".into(), seed);
    fs::write(&summary_path, serde_json::to_vec_pretty(&summary)?)
        .with_context(|| format!("failed to update {}", summary_path.display()))?;

    Ok(OcclusionResult {
        has_occluder,
        confidence,
        mean_coverage,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn extract(
    ffmpeg: &Path,
    input: &Path,
    info: &VideoInfo,
    rect: RectF,
    motion: &mut [MotionSample],
    extraction: &ExtractionResult,
    output_root: &Path,
    diagnostics: &Path,
    sensitivity: f64,
    loop_closed: bool,
    scene_track: Option<&SurfaceTrajectory>,
    authored_layers: &[LayerInput],
    automatic_candidates: bool,
    progress: &mut ProgressReporter,
) -> Result<OcclusionResult> {
    let masks_dir = output_root.join(OCCLUDER_DIR);
    let automatic_masks_dir = output_root.join(AUTOMATIC_OCCLUDER_WORK_DIR);
    let automatic_material_dir = output_root.join(AUTOMATIC_MATERIAL_WORK_DIR);
    let authored_masks_dir = output_root.join(AUTHORED_OCCLUDER_WORK_DIR);
    fs::create_dir_all(&masks_dir)?;
    if automatic_candidates {
        fs::create_dir_all(&automatic_masks_dir)?;
        fs::create_dir_all(&automatic_material_dir)?;
    }
    let has_authored_channel = layers::has_authored_opaque_source_foreground(authored_layers);
    let authored_matte = layers::shared_authored_opaque_source_matte(authored_layers);
    if has_authored_channel {
        fs::create_dir_all(&authored_masks_dir)?;
    }
    let width = extraction.median.width() as usize;
    let height = extraction.median.height() as usize;
    // The robust median is the canonical plaque appearance for the supported
    // text-free source contract; no synthetic blanking is needed.
    let model = extraction.median.pixels();
    let mut decoder = Decoder::spawn(ffmpeg, input, info)?;
    let mut structural_scores = Vec::with_capacity(info.frames);
    let mut residuals = Vec::with_capacity(info.frames);
    let mut automatic_masks = Vec::with_capacity(info.frames);
    let mut authored_masks = Vec::with_capacity(info.frames);
    let structural_guard = dilate(&extraction.structural_mask, width, height, 4);

    for (frame_index, sample) in motion.iter().take(info.frames).enumerate() {
        let Some(frame) = decoder.next_frame()? else {
            break;
        };
        let transform = sample.transform;
        let rectified = rectify(&frame, rect, transform)?;
        let authored_foreground = layers::source_opaque_foreground_mask(
            authored_layers,
            frame_index,
            info.width,
            info.height,
        )?
        .map(|mask| {
            let full = Surface::from_alpha_mask(
                info.width,
                info.height,
                &mask,
                Rgba::new(255, 255, 255, 255),
            )?;
            Ok::<_, anyhow::Error>(rectify(&full, rect, transform)?.alpha_mask())
        })
        .transpose()?;
        let structural_score = structural_match_score(
            rectified.pixels(),
            model,
            &extraction.structural_mask,
            width,
            height,
        );
        let in_frame = visible_quad_fraction(
            transformed_rect(rect, sample.transform),
            info.width,
            info.height,
        );
        // Viewport clipping already removes off-screen title pixels. It must not
        // also fade every still-visible glyph merely because part of the plaque
        // is outside the frame and therefore cannot contribute structural evidence.
        structural_scores.push(if in_frame < 0.985 {
            1.0
        } else {
            structural_score.max(tracking_presence(
                sample.inlier_ratio,
                sample.reprojection_error,
            ))
        });
        let mut residual = vec![0u8; width * height];
        let mut authored_photometric = vec![0u8; width * height];
        let mut deltas = vec![0_u16; width * height];
        let mut source_luma = vec![0_u16; width * height];
        for pixel in 0..deltas.len() {
            let base = pixel * 4;
            deltas[pixel] = (0..3)
                .map(|channel| {
                    rectified.pixels()[base + channel].abs_diff(model[base + channel]) as u16
                })
                .sum::<u16>()
                / 3;
            source_luma[pixel] = (u16::from(rectified.pixels()[base]) * 54
                + u16::from(rectified.pixels()[base + 1]) * 183
                + u16::from(rectified.pixels()[base + 2]) * 19
                + 128)
                / 256;
        }
        for (pixel, residual_value) in residual.iter_mut().enumerate() {
            let d = deltas[pixel];
            let base_threshold = if extraction.content_mask[pixel] > 32 {
                // Animated cavity detail becomes a residual too, but it remains
                // wholly inside the cavity and is rejected at component selection.
                20.0 + extraction.mad[pixel] as f64 * 1.5
            } else {
                16.0 + extraction.mad[pixel] as f64 * 2.5
            };
            let threshold =
                ((base_threshold * sensitivity.clamp(0.35, 3.0)).round() as u16).min(90);
            if d > threshold {
                *residual_value = 255;
            }
            if authored_foreground.is_some()
                && authored_material_changed(
                    d,
                    threshold,
                    local_range(&source_luma, pixel, width, height),
                    local_range(&deltas, pixel, width, height),
                )
            {
                authored_photometric[pixel] = 255;
            }
        }
        for (value, &guard) in residual.iter_mut().zip(&structural_guard) {
            if guard > 0 {
                *value = 0;
            }
        }
        let mut selected = if automatic_candidates {
            select_foreground_components(&residual, &extraction.content_mask, width, height)
        } else {
            vec![0_u8; residual.len()]
        };
        let mut authored = vec![0_u8; residual.len()];
        if let Some(semantic) = authored_foreground.as_deref() {
            let authored_exclusion_radius = (width.min(height) / 24).clamp(3, 12);
            remove_known_foreground(
                &mut selected,
                semantic,
                width,
                height,
                authored_exclusion_radius,
            );
            recover_authored_photometric_detail(
                &mut authored,
                &authored_photometric,
                semantic,
                width,
                height,
            );
        }
        let candidate_coverage = selected
            .iter()
            .zip(&authored)
            .filter(|(automatic, authored)| **automatic > 0 || **authored > 0)
            .count() as f64
            / selected.len().max(1) as f64;
        residuals.push(residual);
        automatic_masks.push(selected);
        authored_masks.push(authored);
        progress.update(
            frame_index + 1,
            format!("candidate coverage {:.3}%", candidate_coverage * 100.0),
        );
    }
    decoder.finish()?;

    // An authored opaque layer already supplies an identity observation on every
    // frame. Keep that detail in a separate channel so temporal recovery can only
    // expand genuinely automatic foreground. Otherwise a moving cast shadow near
    // an authored spider becomes an opaque halo and the authored actor can also
    // dominate prompts intended for an unrelated crossing web.
    let (automatic_masks, canonical_masks) = merge_temporal_foreground_channels(
        &automatic_masks,
        &authored_masks,
        &residuals,
        width,
        height,
    );
    let mut coverages = Vec::with_capacity(canonical_masks.len());
    let mut content_coverages = Vec::with_capacity(canonical_masks.len());
    let mut temporal_agreement = Vec::new();
    let content_weight = extraction
        .content_mask
        .iter()
        .map(|&value| value as f64 / 255.0)
        .sum::<f64>()
        .max(1.0);
    for (frame_index, softened) in canonical_masks.iter().enumerate() {
        if frame_index > 0
            && let Some(iou) = mask_iou(&canonical_masks[frame_index - 1], softened)
        {
            temporal_agreement.push(iou);
        }
        coverages.push(
            softened.iter().map(|&v| v as f64 / 255.0).sum::<f64>() / softened.len().max(1) as f64,
        );
        let content_coverage = softened
            .iter()
            .zip(&extraction.content_mask)
            .map(|(&foreground, &content)| foreground as f64 / 255.0 * content as f64 / 255.0)
            .sum::<f64>()
            / content_weight;
        content_coverages.push(content_coverage);
        motion[frame_index].occluder_coverage = content_coverage;

        let canonical = Surface::from_alpha_mask(
            width as u32,
            height as u32,
            softened,
            Rgba::new(255, 255, 255, 255),
        )?;
        let mut full = Surface::new(info.width, info.height);
        full.warp_blend(
            &canonical,
            transformed_rect(rect, motion[frame_index].transform),
            1.0,
        )?;
        save_luma_png(
            info.width,
            info.height,
            &full.alpha_mask(),
            &masks_dir.join(format!("{frame_index:06}.png")),
        )?;
        if automatic_candidates {
            save_canonical_source_mask(
                &automatic_masks[frame_index],
                width,
                height,
                info,
                rect,
                motion[frame_index].transform,
                None,
                &automatic_masks_dir.join(format!("{frame_index:06}.png")),
            )?;
            save_canonical_source_mask(
                &residuals[frame_index],
                width,
                height,
                info,
                rect,
                motion[frame_index].transform,
                None,
                &automatic_material_dir.join(format!("{frame_index:06}.png")),
            )?;
        }
        if has_authored_channel {
            save_canonical_source_mask(
                &authored_masks[frame_index],
                width,
                height,
                info,
                rect,
                motion[frame_index].transform,
                authored_matte,
                &authored_masks_dir.join(format!("{frame_index:06}.png")),
            )?;
        }
    }

    let max_coverage = coverages.iter().copied().fold(0.0, f64::max);
    let mean_coverage = mean(&coverages);
    let occupied_frames = coverages
        .iter()
        .filter(|&&coverage| coverage > 0.0025)
        .count();
    let occupied_ratio = occupied_frames as f64 / coverages.len().max(1) as f64;
    // Empty/empty pairs do not count as temporal agreement. A foreground layer
    // must persist as a coherent shape; sporadic title-transition residuals are
    // safer to ignore than to restore over newly rendered typography.
    let agreement = if temporal_agreement.is_empty() {
        0.0
    } else {
        mean(&temporal_agreement)
    };
    let persistence = if max_coverage > 0.0 {
        (mean_coverage / max_coverage).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let has_occluder = classify_occluder(max_coverage, occupied_ratio, agreement, persistence);
    let confidence = occluder_confidence(
        has_occluder,
        max_coverage,
        occupied_ratio,
        agreement,
        persistence,
    );
    if !has_occluder {
        // Candidate masks remain visible in diagnostics, but are not allowed to
        // contaminate rendering when temporal evidence is weak.
        crate::staged_output::remove_child(output_root, &masks_dir)?;
        for working in [
            &automatic_masks_dir,
            &automatic_material_dir,
            &authored_masks_dir,
        ] {
            if working.exists() {
                crate::staged_output::remove_child(output_root, working)?;
            }
        }
        for sample in motion.iter_mut() {
            sample.occluder_coverage = 0.0;
        }
    }

    let visibility = smooth_visibility(&structural_scores, loop_closed);
    for (sample, &value) in motion.iter_mut().zip(&visibility) {
        sample.plaque_visibility = value;
    }
    let automatic_mean_visibility = mean(&visibility);
    if let Some(track) = scene_track {
        tracking::apply_visibility_scenes(motion, track)?;
    }
    let final_visibility = motion
        .iter()
        .map(|sample| sample.plaque_visibility)
        .collect::<Vec<_>>();
    let mean_visibility = mean(&final_visibility);
    let minimum_visibility = final_visibility.iter().copied().fold(1.0, f64::min);

    fs::write(
        diagnostics.join("occlusion-summary.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "has_occluder": has_occluder,
            "confidence": confidence,
            "mean_coverage": mean_coverage,
            "max_coverage": max_coverage,
            "occupied_frames": occupied_frames,
            "occupied_ratio": occupied_ratio,
            "coverage_persistence": persistence,
            "nonempty_adjacent_mask_iou": agreement,
            "mean_content_occlusion": mean(&content_coverages),
            "automatic_mean_plaque_visibility": automatic_mean_visibility,
            "mean_plaque_visibility": mean_visibility,
            "minimum_plaque_visibility": minimum_visibility,
            "mask_basis": if automatic_candidates {
                "lossless-photometric-material"
            } else {
                "lossless-photometric-material-near-authored-opaque-semantics"
            },
            "mask_coordinates": "source-pixels",
            "mask_frames_summarized": canonical_masks.len(),
            "summary_matches_installed_masks": has_occluder,
        }))?,
    )?;
    Ok(OcclusionResult {
        has_occluder,
        confidence,
        mean_coverage,
    })
}

fn occluder_confidence(
    has_occluder: bool,
    max_coverage: f64,
    occupied_ratio: f64,
    agreement: f64,
    persistence: f64,
) -> f64 {
    if has_occluder {
        (0.42
            + 0.25 * agreement
            + 0.13 * (occupied_ratio / 0.25).clamp(0.0, 1.0)
            + 0.10 * (persistence / 0.10).clamp(0.0, 1.0)
            + 0.10 * (max_coverage / 0.08).clamp(0.0, 1.0))
        .clamp(0.0, 0.96)
    } else {
        (0.72 + 0.18 * (1.0 - occupied_ratio).clamp(0.0, 1.0)).clamp(0.0, 0.90)
    }
}

fn classify_occluder(
    max_coverage: f64,
    occupied_ratio: f64,
    agreement: f64,
    persistence: f64,
) -> bool {
    max_coverage > 0.0025
        && occupied_ratio >= 0.04
        && agreement >= 0.12
        && (persistence >= 0.02 || occupied_ratio >= 0.10)
}

fn structural_match_score(
    observed: &[u8],
    model: &[u8],
    structural_mask: &[u8],
    width: usize,
    height: usize,
) -> f64 {
    if observed.len() != width * height * 4
        || model.len() != observed.len()
        || structural_mask.len() != width * height
        || width < 3
        || height < 3
    {
        return 0.0;
    }
    let luma = |pixels: &[u8], index: usize| {
        let base = index * 4;
        (f64::from(pixels[base]) * 54.0
            + f64::from(pixels[base + 1]) * 183.0
            + f64::from(pixels[base + 2]) * 19.0)
            / 256.0
    };
    let mut weighted_score = 0.0;
    let mut total_weight = 0.0;
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let index = y * width + x;
            if structural_mask[index] <= 64 {
                continue;
            }
            let model_x = luma(model, index + 1) - luma(model, index - 1);
            let model_y = luma(model, index + width) - luma(model, index - width);
            let model_magnitude = model_x.hypot(model_y);
            if model_magnitude < 8.0 {
                continue;
            }
            let observed_x = luma(observed, index + 1) - luma(observed, index - 1);
            let observed_y = luma(observed, index + width) - luma(observed, index - width);
            let observed_magnitude = observed_x.hypot(observed_y);
            let direction = ((model_x * observed_x + model_y * observed_y)
                / (model_magnitude * observed_magnitude).max(1.0))
            .clamp(0.0, 1.0);
            let presence = (observed_magnitude / 8.0).clamp(0.0, 1.0);
            let weight = f64::from(structural_mask[index]) / 255.0 * model_magnitude.min(255.0);
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

fn smooth_visibility(scores: &[f64], loop_closed: bool) -> Vec<f64> {
    let raw: Vec<f64> = scores
        .iter()
        .map(|&score| {
            let value = ((score - 0.22) / 0.38).clamp(0.0, 1.0);
            value * value * (3.0 - 2.0 * value)
        })
        .collect();
    (0..raw.len())
        .map(|index| {
            let mut neighborhood = if loop_closed && raw.len() >= 3 {
                vec![
                    raw[(index + raw.len() - 1) % raw.len()],
                    raw[index],
                    raw[(index + 1) % raw.len()],
                ]
            } else {
                let start = index.saturating_sub(1);
                let end = (index + 2).min(raw.len());
                raw[start..end].to_vec()
            };
            neighborhood.sort_by(f64::total_cmp);
            neighborhood[neighborhood.len() / 2]
        })
        .collect()
}

fn tracking_presence(inlier_ratio: f64, reprojection_error: f64) -> f64 {
    let support = ((inlier_ratio - 0.12) / 0.33).clamp(0.0, 1.0);
    let precision = (-reprojection_error.clamp(0.0, 20.0) / 12.0).exp();
    support * precision
}

fn select_foreground_components(
    residual: &[u8],
    content: &[u8],
    width: usize,
    height: usize,
) -> Vec<u8> {
    // Opening is used only to identify connected bodies. The returned alpha comes
    // from the original residual, so connectivity cleanup cannot turn a sparse web
    // or foliage silhouette into an opaque sheet.
    let cleaned = morph_open(residual, width, height, 1);
    let mut seen = vec![false; cleaned.len()];
    let mut output = vec![0u8; cleaned.len()];
    let minimum = (width * height / 1200).max(12);
    for seed in 0..cleaned.len() {
        if cleaned[seed] == 0 || seen[seed] {
            continue;
        }
        let mut queue = VecDeque::from([seed]);
        seen[seed] = true;
        let mut component = Vec::new();
        let mut inside_count = 0usize;
        let mut outside_count = 0usize;
        let mut touches_crop = false;
        let mut min_x = width;
        let mut min_y = height;
        let mut max_x = 0usize;
        let mut max_y = 0usize;
        while let Some(index) = queue.pop_front() {
            component.push(index);
            let x = index % width;
            let y = index / width;
            // Ignore the feather band. Treating it as outside caused title and
            // border strokes to masquerade as depth-crossing components.
            if content[index] > 200 {
                inside_count += 1
            } else if content[index] < 32 {
                outside_count += 1
            }
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
            if x < 3 || y < 3 || x + 3 >= width || y + 3 >= height {
                touches_crop = true
            }
            for (nx, ny) in neighbors(x, y, width, height) {
                let ni = ny * width + nx;
                if !seen[ni] && cleaned[ni] > 0 {
                    seen[ni] = true;
                    queue.push_back(ni)
                }
            }
        }
        // A true foreground occluder either enters from outside the canonical
        // crop or crosses the content cavity boundary. Text changes remain wholly
        // inside the cavity and are therefore rejected.
        let bbox_width = max_x.saturating_sub(min_x) + 1;
        let bbox_height = max_y.saturating_sub(min_y) + 1;
        let bbox_area = bbox_width.saturating_mul(bbox_height).max(1);
        let solidity = component.len() as f64 / bbox_area as f64;
        let plaque_wide = bbox_width.saturating_mul(4) >= width.saturating_mul(3)
            && bbox_height.saturating_mul(3) >= height;
        let excessive_area = component.len().saturating_mul(5) >= cleaned.len().saturating_mul(2);
        let plaque_shaped = bbox_width.saturating_mul(5) >= width
            && bbox_width.saturating_mul(5) >= bbox_height.saturating_mul(9);
        let minimum_thickness = ((width.min(height) as f64) * 0.025).round().max(5.0) as usize;
        let crosses_cavity = inside_count >= minimum / 3 && outside_count >= minimum / 3;
        // Reject thin glowing border traces and text/code strokes. A useful
        // occluder has 2-D body, not merely a long luminous edge.
        let has_body = bbox_width.min(bbox_height) >= minimum_thickness && solidity >= 0.12;
        if component.len() >= minimum
            && inside_count >= minimum / 3
            && has_body
            && !plaque_wide
            && !excessive_area
            && !plaque_shaped
            && (crosses_cavity || (touches_crop && outside_count > 0))
        {
            let support = component_mask(&component, cleaned.len());
            let support = dilate(&support, width, height, 1);
            for (index, (&source, &supported)) in residual.iter().zip(&support).enumerate() {
                if source > 0 && supported > 0 {
                    output[index] = source;
                }
            }
        }
    }
    output
}

fn neighbors(x: usize, y: usize, w: usize, h: usize) -> impl Iterator<Item = (usize, usize)> {
    let mut v = Vec::with_capacity(4);
    if x > 0 {
        v.push((x - 1, y))
    }
    if x + 1 < w {
        v.push((x + 1, y))
    }
    if y > 0 {
        v.push((x, y - 1))
    }
    if y + 1 < h {
        v.push((x, y + 1))
    }
    v.into_iter()
}

fn merge_temporal_foreground_channels(
    automatic: &[Vec<u8>],
    authored: &[Vec<u8>],
    residuals: &[Vec<u8>],
    width: usize,
    height: usize,
) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let recovered_automatic = recover_temporal_details(automatic, residuals, width, height);
    let combined = recovered_automatic
        .iter()
        .zip(authored)
        .map(|(automatic, authored)| {
            automatic
                .iter()
                .zip(authored)
                .map(|(&automatic, &authored)| automatic.max(authored))
                .collect()
        })
        .collect();
    (recovered_automatic, combined)
}

fn remove_known_foreground(
    automatic: &mut [u8],
    authored: &[u8],
    width: usize,
    height: usize,
    radius: usize,
) {
    if automatic.len() != authored.len()
        || automatic.len() != width.saturating_mul(height)
        || automatic.is_empty()
    {
        return;
    }
    let authored_support = dilate(authored, width, height, radius);
    for (automatic, authored) in automatic.iter_mut().zip(authored_support) {
        if authored > 0 {
            *automatic = 0;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn save_canonical_source_mask(
    mask: &[u8],
    width: usize,
    height: usize,
    info: &VideoInfo,
    rect: RectF,
    transform: crate::model::Mat3,
    matte: Option<crate::scene::LayerMatte>,
    path: &Path,
) -> Result<()> {
    let canonical = Surface::from_alpha_mask(
        width as u32,
        height as u32,
        mask,
        Rgba::new(255, 255, 255, 255),
    )?;
    let mut full = Surface::new(info.width, info.height);
    full.warp_blend(&canonical, transformed_rect(rect, transform), 1.0)?;
    let mut alpha = full.alpha_mask();
    if let Some(matte) = matte {
        // Projection interpolation must not silently turn declared opaque material
        // into translucent detail. Reapply the shared authored policy only to this
        // channel; automatic web material keeps its measured porous alpha.
        layers::apply_matte_policy(&mut alpha, matte);
    }
    save_luma_png(info.width, info.height, &alpha, path)
}

fn recover_temporal_details(
    selected: &[Vec<u8>],
    residuals: &[Vec<u8>],
    width: usize,
    height: usize,
) -> Vec<Vec<u8>> {
    selected
        .iter()
        .enumerate()
        .map(|(frame, current)| {
            let mut support = vec![0_u8; current.len()];
            for neighbor in [
                frame.checked_sub(1),
                (frame + 1 < selected.len()).then_some(frame + 1),
            ]
            .into_iter()
            .flatten()
            {
                let expanded = dilate(&selected[neighbor], width, height, 6);
                for (value, candidate) in support.iter_mut().zip(expanded) {
                    *value = (*value).max(candidate);
                }
            }
            let residual = residuals.get(frame).map(Vec::as_slice).unwrap_or(&[]);
            let mut recovered = current.clone();
            for ((value, &candidate), &supported) in
                recovered.iter_mut().zip(residual).zip(&support)
            {
                if candidate > 0 && supported > 0 {
                    *value = 255;
                }
            }
            // Feather only the actual material. A close/fill operation here used to
            // erase title through transparent holes between web strands.
            blur_mask(&recovered, width, height, 1)
        })
        .collect()
}

fn recover_authored_photometric_detail(
    selected: &mut [u8],
    photometric: &[u8],
    semantic: &[u8],
    width: usize,
    height: usize,
) {
    if selected.len() != photometric.len()
        || selected.len() != semantic.len()
        || selected.len() != width.saturating_mul(height)
    {
        return;
    }
    // Opaque semantic masks establish object identity; direct source residuals
    // establish the exact material silhouette.  A bounded neighborhood recovers
    // thin limbs and antialiased edges that semantic downsampling can omit without
    // ever creating alpha where the lossless source measured no changed material.
    let radius = (width.min(height) / 10).clamp(4, 18);
    let support = dilate(semantic, width, height, radius);
    for ((target, &material), &near_object) in selected.iter_mut().zip(photometric).zip(&support) {
        if material > 0 && near_object > 0 {
            *target = (*target).max(material);
        }
    }
}

fn authored_material_changed(
    delta: u16,
    generic_threshold: u16,
    source_local_range: u16,
    residual_local_range: u16,
) -> bool {
    // Semantic identity makes a lower material threshold safe here. Keep a hard
    // six-level floor so codec shimmer or rounding cannot become foreground. Thin
    // material also has a crisp source/residual transition; diffuse cast shadows do
    // not, and restoring those as opaque would erase whole title fragments.
    let detail_threshold = ((u32::from(generic_threshold) * 3 + 2) / 5).max(6) as u16;
    delta > detail_threshold && source_local_range >= 8 && residual_local_range >= 8
}

fn local_range(values: &[u16], pixel: usize, width: usize, height: usize) -> u16 {
    if width == 0 || height == 0 || values.len() != width.saturating_mul(height) {
        return 0;
    }
    let x = pixel % width;
    let y = pixel / width;
    let mut minimum = u16::MAX;
    let mut maximum = u16::MIN;
    for sample_y in y.saturating_sub(1)..=(y + 1).min(height - 1) {
        for sample_x in x.saturating_sub(1)..=(x + 1).min(width - 1) {
            let value = values[sample_y * width + sample_x];
            minimum = minimum.min(value);
            maximum = maximum.max(value);
        }
    }
    maximum - minimum
}

fn morph_open(src: &[u8], w: usize, h: usize, r: usize) -> Vec<u8> {
    dilate(&erode(src, w, h, r), w, h, r)
}
fn component_mask(component: &[usize], len: usize) -> Vec<u8> {
    let mut mask = vec![0_u8; len];
    for &index in component {
        mask[index] = 255;
    }
    mask
}

fn visible_quad_fraction(quad: Quad, width: u32, height: u32) -> f64 {
    let (min_x, min_y, max_x, max_y) = quad.bounds();
    let full_width = (max_x - min_x).max(0.0);
    let full_height = (max_y - min_y).max(0.0);
    let full_area = full_width * full_height;
    if !full_area.is_finite() || full_area <= f64::EPSILON {
        return 0.0;
    }
    let visible_width = (max_x.min(width as f64) - min_x.max(0.0)).max(0.0);
    let visible_height = (max_y.min(height as f64) - min_y.max(0.0)).max(0.0);
    (visible_width * visible_height / full_area).clamp(0.0, 1.0)
}
fn erode(src: &[u8], w: usize, h: usize, r: usize) -> Vec<u8> {
    let mut out = vec![0; src.len()];
    for y in 0..h {
        for x in 0..w {
            let mut value = 255u8;
            'scan: for yy in y.saturating_sub(r)..=(y + r).min(h - 1) {
                for xx in x.saturating_sub(r)..=(x + r).min(w - 1) {
                    if src[yy * w + xx] == 0 {
                        value = 0;
                        break 'scan;
                    }
                }
            }
            out[y * w + x] = value;
        }
    }
    out
}
fn dilate(src: &[u8], w: usize, h: usize, r: usize) -> Vec<u8> {
    let mut out = vec![0; src.len()];
    for y in 0..h {
        for x in 0..w {
            let mut value = 0u8;
            'scan: for yy in y.saturating_sub(r)..=(y + r).min(h - 1) {
                for xx in x.saturating_sub(r)..=(x + r).min(w - 1) {
                    if src[yy * w + xx] > 0 {
                        value = 255;
                        break 'scan;
                    }
                }
            }
            out[y * w + x] = value;
        }
    }
    out
}
fn blur_mask(src: &[u8], w: usize, h: usize, r: usize) -> Vec<u8> {
    let mut out = vec![0; src.len()];
    for y in 0..h {
        for x in 0..w {
            let mut sum = 0u32;
            let mut n = 0u32;
            for yy in y.saturating_sub(r)..=(y + r).min(h - 1) {
                for xx in x.saturating_sub(r)..=(x + r).min(w - 1) {
                    sum += src[yy * w + xx] as u32;
                    n += 1
                }
            }
            out[y * w + x] = (sum / n.max(1)) as u8;
        }
    }
    out
}
fn mask_iou(a: &[u8], b: &[u8]) -> Option<f64> {
    let mut i = 0usize;
    let mut u = 0usize;
    for (&x, &y) in a.iter().zip(b) {
        let x = x > 64;
        let y = y > 64;
        if x && y {
            i += 1
        }
        if x || y {
            u += 1
        }
    }
    if u == 0 {
        None
    } else {
        Some(i as f64 / u as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        authored_material_changed, classify_occluder, local_range, mask_iou,
        merge_temporal_foreground_channels, recover_authored_photometric_detail,
        recover_temporal_details, remove_known_foreground, select_foreground_components,
        smooth_visibility, structural_match_score, tracking_presence,
    };

    fn cavity(width: usize, height: usize) -> Vec<u8> {
        let mut mask = vec![0_u8; width * height];
        for y in 15..45 {
            for x in 20..80 {
                mask[y * width + x] = 255;
            }
        }
        mask
    }

    #[test]
    fn rejects_component_confined_to_text_cavity() {
        let (width, height) = (100, 60);
        let mut residual = vec![0_u8; width * height];
        for y in 22..34 {
            for x in 32..68 {
                residual[y * width + x] = 255;
            }
        }
        let selected =
            select_foreground_components(&residual, &cavity(width, height), width, height);
        assert!(selected.iter().all(|&value| value == 0));
    }

    #[test]
    fn keeps_body_crossing_cavity_boundary() {
        let (width, height) = (100, 60);
        let mut residual = vec![0_u8; width * height];
        for y in 22..40 {
            for x in 70..92 {
                residual[y * width + x] = 255;
            }
        }
        let selected =
            select_foreground_components(&residual, &cavity(width, height), width, height);
        assert!(selected.iter().any(|&value| value > 0));
    }

    #[test]
    fn rejects_plaque_wide_residual_component() {
        let (width, height) = (100, 60);
        let mut residual = vec![0_u8; width * height];
        for y in 10..51 {
            for x in 5..96 {
                residual[y * width + x] = 255;
            }
        }
        let selected =
            select_foreground_components(&residual, &cavity(width, height), width, height);
        assert!(selected.iter().all(|&value| value == 0));
    }

    #[test]
    fn rejects_elongated_plaque_texture_residual() {
        let (width, height) = (100, 60);
        let mut residual = vec![0_u8; width * height];
        for y in 20..35 {
            for x in 55..90 {
                residual[y * width + x] = 255;
            }
        }
        let selected =
            select_foreground_components(&residual, &cavity(width, height), width, height);
        assert!(selected.iter().all(|&value| value == 0));
    }

    #[test]
    fn empty_masks_have_no_temporal_evidence() {
        assert_eq!(mask_iou(&[0; 16], &[0; 16]), None);
    }

    #[test]
    fn coherent_intermittent_occlusion_is_not_discarded() {
        assert!(classify_occluder(0.29, 0.42, 0.60, 0.058));
        assert!(!classify_occluder(0.29, 0.01, 0.60, 0.50));
        assert!(!classify_occluder(0.29, 0.42, 0.02, 0.50));
    }

    #[test]
    fn temporal_support_recovers_thin_connected_detail() {
        let (width, height) = (40, 20);
        let mut selected = vec![vec![0_u8; width * height]; 3];
        let mut residuals = selected.clone();
        for frame in 0..3 {
            for y in 6..14 {
                for x in 10 + frame..18 + frame {
                    selected[frame][y * width + x] = 255;
                    residuals[frame][y * width + x] = 255;
                }
            }
        }
        for x in 18..25 {
            residuals[1][10 * width + x] = 255;
        }

        let recovered = recover_temporal_details(&selected, &residuals, width, height);

        assert!(recovered[1][10 * width + 22] > 0);
    }

    #[test]
    fn automatic_temporal_recovery_cannot_spread_authored_detail() {
        let (width, height) = (24, 12);
        let automatic = vec![vec![0_u8; width * height]; 3];
        let mut authored = automatic.clone();
        let mut residuals = automatic.clone();
        authored[1][6 * width + 12] = 255;
        for residual in &mut residuals {
            residual[6 * width + 13] = 255;
        }

        let (recovered_automatic, combined) =
            merge_temporal_foreground_channels(&automatic, &authored, &residuals, width, height);

        assert!(
            recovered_automatic
                .iter()
                .flatten()
                .all(|&alpha| alpha == 0)
        );
        assert_eq!(combined[0][6 * width + 13], 0);
        assert_eq!(combined[1][6 * width + 12], 255);
        assert_eq!(combined[2][6 * width + 13], 0);
    }

    #[test]
    fn authored_foreground_is_removed_from_automatic_discovery_only() {
        let (width, height) = (12, 5);
        let mut automatic = vec![0_u8; width * height];
        automatic[2 * width + 1] = 255;
        automatic[2 * width + 8] = 255;
        let mut authored = vec![0_u8; width * height];
        authored[2 * width + 8] = 255;

        remove_known_foreground(&mut automatic, &authored, width, height, 1);

        assert_eq!(automatic[2 * width + 1], 255, "remote web material remains");
        assert_eq!(automatic[2 * width + 7], 0);
        assert_eq!(
            automatic[2 * width + 8],
            0,
            "known spider is not rediscovered"
        );
        assert_eq!(automatic[2 * width + 9], 0);
    }

    #[test]
    fn authored_semantics_recover_only_nearby_measured_material() {
        let (width, height) = (48, 20);
        let mut selected = vec![0_u8; width * height];
        let mut semantic = selected.clone();
        let mut photometric = selected.clone();
        for y in 7..13 {
            for x in 8..15 {
                semantic[y * width + x] = 255;
                photometric[y * width + x] = 255;
            }
        }
        for x in 15..19 {
            photometric[10 * width + x] = 255;
        }
        for x in 38..44 {
            photometric[10 * width + x] = 255;
        }

        recover_authored_photometric_detail(&mut selected, &photometric, &semantic, width, height);

        assert_eq!(selected[10 * width + 18], 255);
        assert_eq!(selected[10 * width + 19], 0, "must not invent a halo");
        assert_eq!(selected[10 * width + 40], 0, "must reject unrelated motion");
    }

    #[test]
    fn authored_opaque_detail_uses_a_sensitive_but_nonzero_material_threshold() {
        assert!(authored_material_changed(16, 25, 8, 8));
        assert!(!authored_material_changed(5, 25, 20, 20));
        assert!(!authored_material_changed(15, 25, 20, 20));
    }

    #[test]
    fn authored_opaque_detail_rejects_a_diffuse_cast_shadow() {
        assert!(!authored_material_changed(24, 25, 4, 7));
        assert!(!authored_material_changed(24, 25, 8, 4));
    }

    #[test]
    fn local_range_measures_a_three_by_three_material_edge() {
        let values = [10, 10, 10, 10, 18, 18, 10, 18, 18];
        assert_eq!(local_range(&values, 4, 3, 3), 8);
        assert_eq!(local_range(&values, 0, 3, 3), 8);
    }

    #[test]
    fn visibility_suppresses_frames_with_missing_structure() {
        let visibility = smooth_visibility(&[0.9, 0.9, 0.1, 0.1, 0.1, 0.9, 0.9], false);
        assert!(visibility[0] > 0.95);
        assert!(visibility[3] < 0.05);
        assert!(visibility[6] > 0.95);
    }

    #[test]
    fn tracking_registration_confirms_plaque_presence() {
        assert!(tracking_presence(0.70, 1.0) > 0.90);
        assert_eq!(tracking_presence(0.0, 24.0), 0.0);
    }

    #[test]
    fn structural_visibility_uses_edges_not_exact_animated_color() {
        let (width, height) = (16, 16);
        let mut model = vec![0_u8; width * height * 4];
        let mut recolored = vec![0_u8; width * height * 4];
        let mut shifted = vec![0_u8; width * height * 4];
        for y in 0..height {
            for x in 0..width {
                let index = (y * width + x) * 4;
                if x >= 8 {
                    model[index..index + 4].copy_from_slice(&[20, 220, 240, 255]);
                    recolored[index..index + 4].copy_from_slice(&[245, 245, 245, 255]);
                }
                if x >= 11 {
                    shifted[index..index + 4].copy_from_slice(&[245, 245, 245, 255]);
                }
            }
        }
        let mut mask = vec![0_u8; width * height];
        for y in 1..height - 1 {
            mask[y * width + 7] = 255;
            mask[y * width + 8] = 255;
        }

        let color_changed = structural_match_score(&recolored, &model, &mask, width, height);
        let geometry_changed = structural_match_score(&shifted, &model, &mask, width, height);
        assert!(color_changed > 0.99);
        assert!(geometry_changed < 0.10);
    }
}
