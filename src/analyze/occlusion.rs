use std::{collections::VecDeque, fs, path::Path};

use anyhow::{Context, Result};
use image::{GrayImage, ImageBuffer, Luma};

use crate::{
    color::Rgba,
    model::{MotionSample, RectF},
    progress::ProgressReporter,
    refinement::MotionRefinement,
    surface::Surface,
    video::{Decoder, VideoInfo},
};

use super::{
    extraction::{ExtractionResult, rectify, transformed_rect},
    tracking,
};

pub struct OcclusionResult {
    pub has_occluder: bool,
    pub confidence: f64,
    pub mean_coverage: f64,
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
    refinement_track: Option<&MotionRefinement>,
    progress: &mut ProgressReporter,
) -> Result<OcclusionResult> {
    let masks_dir = output_root.join("occluder");
    fs::create_dir_all(&masks_dir)?;
    let width = extraction.median.width() as usize;
    let height = extraction.median.height() as usize;
    // Milestone 3 accepts only text-free production sources. The robust median
    // is therefore the canonical plaque appearance; no synthetic blanking is done.
    let model = extraction.median.pixels();
    let mut decoder = Decoder::spawn(ffmpeg, input, info)?;
    let mut structural_scores = Vec::with_capacity(info.frames);
    let mut residuals = Vec::with_capacity(info.frames);
    let mut canonical_masks = Vec::with_capacity(info.frames);
    let structural_guard = dilate(&extraction.structural_mask, width, height, 4);

    for (frame_index, sample) in motion.iter().take(info.frames).enumerate() {
        let Some(frame) = decoder.next_frame()? else {
            break;
        };
        let transform = sample.transform;
        let rectified = rectify(&frame, rect, transform)?;
        let structural_score = structural_match_score(
            rectified.pixels(),
            model,
            &extraction.structural_mask,
            width,
            height,
        );
        structural_scores.push(structural_score.max(tracking_presence(
            sample.inlier_ratio,
            sample.reprojection_error,
        )));
        let mut residual = vec![0u8; width * height];
        for (pixel, residual_value) in residual.iter_mut().enumerate() {
            let base = pixel * 4;
            let d = (0..3)
                .map(|c| rectified.pixels()[base + c].abs_diff(model[base + c]) as u16)
                .sum::<u16>()
                / 3;
            let base_threshold = if extraction.content_mask[pixel] > 32 {
                // Animated cavity detail becomes a residual too, but it remains
                // wholly inside the cavity and is rejected at component selection.
                20.0 + extraction.mad[pixel] as f64 * 1.5
            } else {
                16.0 + extraction.mad[pixel] as f64 * 2.5
            };
            let threshold = (base_threshold * sensitivity.clamp(0.35, 3.0)).round() as u16;
            if d > threshold.min(90) {
                *residual_value = 255;
            }
        }
        for (value, &guard) in residual.iter_mut().zip(&structural_guard) {
            if guard > 0 {
                *value = 0;
            }
        }
        let selected =
            select_foreground_components(&residual, &extraction.content_mask, width, height);
        let candidate_coverage = selected.iter().filter(|&&value| value > 0).count() as f64
            / selected.len().max(1) as f64;
        residuals.push(residual);
        canonical_masks.push(selected);
        progress.update(
            frame_index + 1,
            format!("candidate coverage {:.3}%", candidate_coverage * 100.0),
        );
    }
    decoder.finish()?;

    let canonical_masks = recover_temporal_details(&canonical_masks, &residuals, width, height);
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
        save_luma(
            info.width,
            info.height,
            &full.alpha_mask(),
            &masks_dir.join(format!("{frame_index:06}.png")),
        )?;
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
    let confidence = if has_occluder {
        (0.42
            + 0.25 * agreement
            + 0.13 * (occupied_ratio / 0.25).clamp(0.0, 1.0)
            + 0.10 * (persistence / 0.10).clamp(0.0, 1.0)
            + 0.10 * (max_coverage / 0.08).clamp(0.0, 1.0))
        .clamp(0.0, 0.96)
    } else {
        (0.72 + 0.18 * (1.0 - occupied_ratio).clamp(0.0, 1.0)).clamp(0.0, 0.90)
    };
    if !has_occluder {
        // Candidate masks remain visible in diagnostics, but are not allowed to
        // contaminate rendering when temporal evidence is weak.
        crate::staged_output::remove_child(output_root, &masks_dir)?;
        for sample in motion.iter_mut() {
            sample.occluder_coverage = 0.0;
        }
    }

    let visibility = smooth_visibility(&structural_scores, loop_closed);
    for (sample, &value) in motion.iter_mut().zip(&visibility) {
        sample.plaque_visibility = value;
    }
    let automatic_mean_visibility = mean(&visibility);
    if let Some(track) = refinement_track {
        tracking::apply_visibility_refinements(motion, track)?;
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
            "nonempty_adjacent_mask_iou": agreement
            ,"mean_content_occlusion": mean(&content_coverages)
            ,"automatic_mean_plaque_visibility": automatic_mean_visibility
            ,"mean_plaque_visibility": mean_visibility
            ,"minimum_plaque_visibility": minimum_visibility
        }))?,
    )?;
    Ok(OcclusionResult {
        has_occluder,
        confidence,
        mean_coverage,
    })
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
    let cleaned = morph_close(&morph_open(residual, width, height, 1), width, height, 2);
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
            for i in component {
                output[i] = 255;
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
            blur_mask(&morph_close(&recovered, width, height, 1), width, height, 2)
        })
        .collect()
}

fn morph_open(src: &[u8], w: usize, h: usize, r: usize) -> Vec<u8> {
    dilate(&erode(src, w, h, r), w, h, r)
}
fn morph_close(src: &[u8], w: usize, h: usize, r: usize) -> Vec<u8> {
    erode(&dilate(src, w, h, r), w, h, r)
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
fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}
fn save_luma(width: u32, height: u32, data: &[u8], path: &Path) -> Result<()> {
    let image: GrayImage = ImageBuffer::<Luma<u8>, _>::from_raw(width, height, data.to_vec())
        .context("invalid luma mask")?;
    image.save(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        classify_occluder, mask_iou, recover_temporal_details, select_foreground_components,
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
