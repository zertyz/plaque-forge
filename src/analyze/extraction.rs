//! Canonical plaque extraction.
//!
//! Rectifies tracked frames into one stable plaque coordinate system and separates the
//! writable content region from structural features used to validate tracking.

use std::path::Path;

use anyhow::{Context, Result, bail};
use image::{GrayImage, ImageBuffer, Luma, RgbaImage};
use opencv::{
    core::{self, Mat, Scalar},
    prelude::*,
    video::{self as cv_video, ECCParametersTrait},
};

use crate::{
    geometry::{Point, Quad},
    model::{Mat3, MotionSample, RectF},
    progress::ProgressReporter,
    scene::SurfaceTrajectory,
    surface::Surface,
    video::{Decoder, VideoInfo},
};

use super::tracking;

pub struct ExtractionResult {
    /// Robust canonical plaque observation used for diagnostics and structural locking.
    pub median: Surface,
    pub mad: Vec<u8>,
    /// The only region where custom typography may be drawn.
    pub content_mask: Vec<u8>,
    pub structural_mask: Vec<u8>,
    pub cavity_area: f64,
    pub registration_area: f64,
    pub structural_area: f64,
    pub confidence: f64,
}

#[allow(clippy::too_many_arguments)]
pub fn recover(
    ffmpeg: &Path,
    input: &Path,
    info: &VideoInfo,
    rect: RectF,
    motion: &mut [MotionSample],
    output_root: &Path,
    diagnostics: &Path,
    sample_count: usize,
    local_scene_radius: i32,
    refine_automatic_track: bool,
    scene_track: Option<&SurfaceTrajectory>,
    reference_frame: usize,
    tracking_inertia: f64,
    loop_closed: bool,
    progress: &mut ProgressReporter,
) -> Result<ExtractionResult> {
    let sample_indices = evenly_spaced(info.frames, info.frames.min(sample_count.max(1)).max(1));
    progress.start(4, 7, "Rectify plaque samples", Some(sample_indices.len()));
    let mut samples =
        decode_rectified_samples(ffmpeg, input, info, rect, motion, &sample_indices, progress)?;
    let (mut median, initial_mad) = robust_median(&samples)?;
    let mut registration_mask = detect_registration_mask(&median, &initial_mad);
    progress.finish("initial canonical model");

    progress.start(5, 7, "Plaque structural lock", Some(info.frames));
    if refine_automatic_track {
        for pass in 0..2 {
            refine_motion_from_surface(
                ffmpeg,
                input,
                info,
                rect,
                motion,
                &median,
                &registration_mask,
                local_scene_radius.max(0),
                reference_frame,
                progress,
            )?;
            tracking::repair_outliers(motion, rect);
            tracking::optimize_trajectory(
                motion,
                rect,
                reference_frame,
                tracking_inertia,
                loop_closed,
            )?;
            if let Some(track) = scene_track {
                tracking::reapply_locked_scenes(motion, track, rect)?;
            }
            if pass == 0 {
                samples = decode_rectified_samples(
                    ffmpeg,
                    input,
                    info,
                    rect,
                    motion,
                    &sample_indices,
                    progress,
                )?;
                let model = robust_median(&samples)?;
                median = model.0;
                registration_mask = detect_registration_mask(&median, &model.1);
            }
        }
        progress.finish("two-pass structural scene");
    } else {
        progress.update(info.frames, "structural correction skipped");
        progress.finish("current trajectory retained");
    }

    progress.start(6, 7, "Build analysis assets", Some(sample_indices.len()));
    samples =
        decode_rectified_samples(ffmpeg, input, info, rect, motion, &sample_indices, progress)?;
    let (refined_median, mad) = robust_median(&samples)?;
    median = refined_median;
    let content_mask = detect_content_cavity(&median);
    registration_mask = detect_registration_mask(&median, &mad);
    let structural_mask = detect_structural_mask(&median, &content_mask, &mad);

    save_luma_png(
        median.width(),
        median.height(),
        &content_mask,
        &output_root.join("content-mask.png"),
    )?;
    save_luma_png(
        median.width(),
        median.height(),
        &structural_mask,
        &output_root.join("structural-mask.png"),
    )?;
    let structural_template = masked_template(&median, &structural_mask)?;
    save_surface_png(
        &structural_template,
        &output_root.join("structural-template.png"),
    )?;
    save_luma_png(
        median.width(),
        median.height(),
        &registration_mask,
        &output_root.join("registration-mask.png"),
    )?;
    let registration_template = masked_template(&median, &registration_mask)?;
    save_surface_png(
        &registration_template,
        &output_root.join("registration-template.png"),
    )?;
    save_surface_png(&median, &diagnostics.join("canonical-reference.png"))?;
    save_luma_png(
        median.width(),
        median.height(),
        &mad,
        &diagnostics.join("temporal-mad.png"),
    )?;

    let cavity_area =
        content_mask.iter().filter(|&&v| v > 127).count() as f64 / content_mask.len().max(1) as f64;
    let structural_area = structural_mask.iter().filter(|&&v| v > 127).count() as f64
        / structural_mask.len().max(1) as f64;
    let registration_area = registration_mask.iter().filter(|&&v| v > 127).count() as f64
        / registration_mask.len().max(1) as f64;
    let stable = mad.iter().filter(|&&v| v < 18).count() as f64 / mad.len().max(1) as f64;
    let confidence = (0.45 * stable
        + 0.35 * cavity_area_score(cavity_area)
        + 0.20 * structural_area_score(structural_area))
    .clamp(0.0, 0.98);
    progress.finish(format!(
        "cavity {:.1}%, registration {:.1}%, structure {:.1}%",
        cavity_area * 100.0,
        registration_area * 100.0,
        structural_area * 100.0
    ));

    Ok(ExtractionResult {
        median,
        mad,
        content_mask,
        structural_mask,
        cavity_area,
        registration_area,
        structural_area,
        confidence,
    })
}

fn decode_rectified_samples(
    ffmpeg: &Path,
    input: &Path,
    info: &VideoInfo,
    rect: RectF,
    motion: &[MotionSample],
    wanted: &[usize],
    progress: &mut ProgressReporter,
) -> Result<Vec<Surface>> {
    let mut decoder = Decoder::spawn(ffmpeg, input, info)?;
    let mut output = Vec::with_capacity(wanted.len());
    let mut wanted_pos = 0usize;
    for frame_index in 0..info.frames {
        let Some(frame) = decoder.next_frame()? else {
            break;
        };
        if wanted_pos < wanted.len() && wanted[wanted_pos] == frame_index {
            let sample = motion.get(frame_index).context("missing motion sample")?;
            output.push(rectify(&frame, rect, sample.transform)?);
            wanted_pos += 1;
            progress.update(wanted_pos, format!("source frame {frame_index}"));
        }
    }
    decoder.finish()?;
    if output.is_empty() {
        bail!("no frames were available for plaque analysis");
    }
    Ok(output)
}

pub fn rectify(frame: &Surface, rect: RectF, transform: Mat3) -> Result<Surface> {
    let destination = transformed_rect(rect, transform);
    Surface::extract_quad(
        frame,
        destination,
        rect.width.round().max(1.0) as u32,
        rect.height.round().max(1.0) as u32,
    )
}

pub fn transformed_rect(rect: RectF, transform: Mat3) -> Quad {
    let p = |x, y| {
        let p = transform.transform(crate::model::PointF { x, y });
        Point::new(p.x, p.y)
    };
    Quad::new(
        p(rect.x, rect.y),
        p(rect.x + rect.width, rect.y),
        p(rect.x + rect.width, rect.y + rect.height),
        p(rect.x, rect.y + rect.height),
    )
}

fn robust_median(samples: &[Surface]) -> Result<(Surface, Vec<u8>)> {
    let first = samples.first().context("empty plaque sample set")?;
    let width = first.width();
    let height = first.height();
    if samples
        .iter()
        .any(|s| s.width() != width || s.height() != height)
    {
        bail!("rectified plaque samples have inconsistent dimensions");
    }
    let pixels = width as usize * height as usize;
    let mut median_bytes = vec![0u8; pixels * 4];
    let mut mad = vec![0u8; pixels];
    let mut values = Vec::with_capacity(samples.len());
    let mut deviations = Vec::with_capacity(samples.len());
    for pixel in 0..pixels {
        let mut channel_mad = 0u16;
        for channel in 0..3 {
            values.clear();
            values.extend(samples.iter().map(|s| s.pixels()[pixel * 4 + channel]));
            values.sort_unstable();
            let median = values[values.len() / 2];
            median_bytes[pixel * 4 + channel] = median;
            deviations.clear();
            deviations.extend(values.iter().map(|&value| value.abs_diff(median)));
            deviations.sort_unstable();
            channel_mad += deviations[deviations.len() / 2] as u16;
        }
        median_bytes[pixel * 4 + 3] = 255;
        mad[pixel] = (channel_mad / 3).min(255) as u8;
    }
    Ok((Surface::from_rgba(width, height, median_bytes)?, mad))
}

fn detect_content_cavity(median: &Surface) -> Vec<u8> {
    let width = median.width() as usize;
    let height = median.height() as usize;
    let gray = grayscale(median);
    let mut columns = vec![0f64; width];
    let mut rows = vec![0f64; height];
    for y in 1..height.saturating_sub(1) {
        for x in 1..width.saturating_sub(1) {
            let gx = gray[y * width + x + 1].abs_diff(gray[y * width + x - 1]) as f64;
            let gy = gray[(y + 1) * width + x].abs_diff(gray[(y - 1) * width + x]) as f64;
            let edge = (gx + gy).min(255.0);
            columns[x] += edge;
            rows[y] += edge;
        }
    }
    let left = peak(&columns, 0, (width as f64 * 0.14) as usize).unwrap_or(0);
    let right = peak(&columns, (width as f64 * 0.86) as usize, width).unwrap_or(width - 1);
    let top = peak(&rows, 0, (height as f64 * 0.14) as usize).unwrap_or(0);
    let bottom = peak(&rows, (height as f64 * 0.86) as usize, height).unwrap_or(height - 1);
    let mut x0 = left + (width as f64 * 0.055).round() as usize;
    let mut x1 = right.saturating_sub((width as f64 * 0.055).round() as usize);
    let mut y0 = top + (height as f64 * 0.095).round() as usize;
    let mut y1 = bottom.saturating_sub((height as f64 * 0.075).round() as usize);
    if x1 <= x0 || y1 <= y0 || x1 - x0 < width * 2 / 3 || y1 - y0 < height / 2 {
        x0 = width * 8 / 100;
        x1 = width * 92 / 100;
        y0 = height * 14 / 100;
        y1 = height * 90 / 100;
    }
    x1 = x1.min(width - 1);
    y1 = y1.min(height - 1);
    let chamfer = ((x1 - x0).min(y1 - y0) as f64 * 0.055).round() as usize;
    let mut hard = vec![0u8; width * height];
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = (x - x0).min(x1 - x);
            let dy = (y - y0).min(y1 - y);
            if dx + dy >= chamfer {
                hard[y * width + x] = 255;
            }
        }
    }
    blur_luma(
        &hard,
        width,
        height,
        ((width.min(height) as f64 * 0.010).round() as usize).max(1),
    )
}

fn detect_structural_mask(median: &Surface, content: &[u8], mad: &[u8]) -> Vec<u8> {
    let width = median.width() as usize;
    let height = median.height() as usize;
    let gray = grayscale(median);
    let mut mask = vec![0u8; width * height];
    for y in 2..height.saturating_sub(2) {
        for x in 2..width.saturating_sub(2) {
            let index = y * width + x;
            if content[index] > 32 || mad[index] > 32 {
                continue;
            }
            let gx = gray[y * width + x + 1].abs_diff(gray[y * width + x - 1]) as u16;
            let gy = gray[(y + 1) * width + x].abs_diff(gray[(y - 1) * width + x]) as u16;
            if gx + gy >= 42 {
                mask[index] = 255;
            }
        }
    }
    blur_luma(&mask, width, height, 1)
}

/// Select temporally stable texture across the whole source surface. This is a
/// distinct contract from `structural_mask`: writable material is valid tracking
/// evidence, but it must never become an occlusion-protection mask.
fn detect_registration_mask(median: &Surface, mad: &[u8]) -> Vec<u8> {
    let width = median.width() as usize;
    let height = median.height() as usize;
    let gray = grayscale(median);
    let mut mask = vec![0u8; width * height];
    for y in 2..height.saturating_sub(2) {
        for x in 2..width.saturating_sub(2) {
            let index = y * width + x;
            if mad[index] > 24 {
                continue;
            }
            let gx = gray[y * width + x + 1].abs_diff(gray[y * width + x - 1]) as u16;
            let gy = gray[(y + 1) * width + x].abs_diff(gray[(y - 1) * width + x]) as u16;
            if gx + gy >= 24 {
                // Keep thin cracks, grain, and engraved edges crisp. Averaging this
                // binary mask used to erase precisely the sparse evidence needed on
                // narrow plaques.
                mask[index] = 255;
            }
        }
    }
    mask
}

fn masked_template(median: &Surface, mask: &[u8]) -> Result<Surface> {
    let mut output = median.clone();
    output.apply_alpha_mask(mask)?;
    Ok(output)
}

#[allow(clippy::too_many_arguments)]
fn refine_motion_from_surface(
    ffmpeg: &Path,
    input: &Path,
    info: &VideoInfo,
    rect: RectF,
    motion: &mut [MotionSample],
    template: &Surface,
    structural_mask: &[u8],
    radius: i32,
    reference_frame: usize,
    progress: &mut ProgressReporter,
) -> Result<()> {
    let Some(matcher) = StructuralMatcher::new(template, structural_mask) else {
        return Ok(());
    };
    let mut decoder = Decoder::spawn(ffmpeg, input, info)?;
    let mut corrections = vec![Mat3::IDENTITY; motion.len().min(info.frames)];
    for frame_index in 0..motion.len().min(info.frames) {
        let Some(frame) = decoder.next_frame()? else {
            break;
        };
        if frame_index == reference_frame {
            progress.update(frame_index + 1, "reference frame fixed");
            continue;
        }
        let current = rectify(&frame, rect, motion[frame_index].transform)?;
        let Some(result) = matcher.measure(&current, radius) else {
            progress.update(frame_index + 1, "foreground-contaminated evidence rejected");
            continue;
        };
        if result.after + 0.25 < result.before {
            corrections[frame_index] = result.transform;
            motion[frame_index].reprojection_error =
                motion[frame_index].reprojection_error.min(result.after);
            motion[frame_index].ecc = result.ecc;
        }
        progress.update(
            frame_index + 1,
            format!(
                "residual {:.2}px",
                result.displacement(template.width(), template.height())
            ),
        );
    }
    decoder.finish()?;

    if let Some(reference) = corrections.get_mut(reference_frame) {
        *reference = Mat3::IDENTITY;
    }
    for (sample, correction) in motion.iter_mut().zip(corrections) {
        let correction = Mat3::translation(rect.x, rect.y)
            .multiply(correction)
            .multiply(Mat3::translation(-rect.x, -rect.y));
        sample.transform = sample.transform.multiply(correction);
    }
    Ok(())
}

pub(crate) struct StructuralRegistration {
    pub transform: Mat3,
    pub before: f64,
    pub after: f64,
    pub ecc: Option<f64>,
}

pub(crate) struct StructuralMatcher {
    template_gray: Vec<u8>,
    template_mat: Mat,
    structural_mask: Vec<u8>,
    width: usize,
    height: usize,
}

impl StructuralMatcher {
    pub(crate) fn new(template: &Surface, structural_mask: &[u8]) -> Option<Self> {
        let width = template.width() as usize;
        let height = template.height() as usize;
        if structural_mask.len() != width * height {
            return None;
        }
        let template_gray = grayscale(template);
        let stable_points = select_structural_points(structural_mask, width, 768);
        if stable_points.len() < 80 {
            return None;
        }
        Some(Self {
            template_mat: luma_mat(&template_gray, width, height).ok()?,
            structural_mask: structural_mask.to_vec(),
            template_gray,
            width,
            height,
        })
    }

    pub(crate) fn measure(&self, current: &Surface, radius: i32) -> Option<StructuralRegistration> {
        self.measure_excluding(current, radius, None)
    }

    /// Measure residual surface motion while removing a known per-frame
    /// foreground matte from the canonical registration support.
    pub(crate) fn measure_excluding(
        &self,
        current: &Surface,
        radius: i32,
        exclusion: Option<&[u8]>,
    ) -> Option<StructuralRegistration> {
        if current.width() as usize != self.width || current.height() as usize != self.height {
            return None;
        }
        if exclusion.is_some_and(|mask| mask.len() != self.structural_mask.len()) {
            return None;
        }
        let current = grayscale(current);
        let current_mat = luma_mat(&current, self.width, self.height).ok()?;
        let usable_mask = registration_mask_excluding(&self.structural_mask, exclusion);
        let initial_points = select_structural_points(&usable_mask, self.width, 768);
        let initial = search_similarity(
            &self.template_gray,
            &current,
            self.width,
            self.height,
            &initial_points,
            radius,
        );
        let visible_mask = visible_structural_mask(
            &self.template_gray,
            &current,
            &usable_mask,
            self.width,
            self.height,
            initial.transform,
            radius.clamp(2, 8),
        );
        let stable_points = select_structural_points(&visible_mask, self.width, 768);
        if !spatially_balanced(&stable_points, self.width, self.height) {
            return None;
        }
        let mask_mat = luma_mat(&visible_mask, self.width, self.height).ok()?;
        let similarity = search_similarity(
            &self.template_gray,
            &current,
            self.width,
            self.height,
            &stable_points,
            radius,
        );
        Some(self.refine_ecc(
            &current,
            &current_mat,
            &stable_points,
            &mask_mat,
            radius,
            similarity,
        ))
    }

    fn refine_ecc(
        &self,
        current: &[u8],
        current_mat: &Mat,
        stable_points: &[(usize, usize)],
        mask_mat: &Mat,
        radius: i32,
        initial: StructuralRegistration,
    ) -> StructuralRegistration {
        if radius <= 0 || self.width < 32 || self.height < 32 {
            return initial;
        }

        let before = initial.before;
        let mut best = initial;
        let mut best_objective = best.after;
        for (motion, penalty) in [
            (cv_video::MOTION_AFFINE, 0.12),
            (cv_video::MOTION_HOMOGRAPHY, 0.30),
        ] {
            let Ok(mut warp) = warp_mat(best.transform, motion) else {
                continue;
            };
            let Ok(mut parameters) = cv_video::ECCParameters::default() else {
                continue;
            };
            parameters.set_motion_type(motion);
            parameters.set_nlevels(3);
            let Ok(ecc) = cv_video::find_transform_ecc_multi_scale(
                &self.template_mat,
                current_mat,
                &mut warp,
                &parameters,
                mask_mat,
                &core::no_array(),
            ) else {
                continue;
            };
            let Ok(transform) = mat3_from_warp(&warp) else {
                continue;
            };
            if !valid_correction(transform, self.width, self.height, radius) {
                continue;
            }
            let after = registration_cost(
                &self.template_gray,
                current,
                self.width,
                self.height,
                stable_points,
                transform,
            );
            let objective = after + penalty;
            if objective < best_objective {
                best_objective = objective;
                best = StructuralRegistration {
                    transform,
                    before,
                    after,
                    ecc: Some(ecc),
                };
            }
        }
        best
    }
}

fn registration_mask_excluding(structural: &[u8], exclusion: Option<&[u8]>) -> Vec<u8> {
    let Some(exclusion) = exclusion else {
        return structural.to_vec();
    };
    structural
        .iter()
        .zip(exclusion)
        .map(|(&stable, &hidden)| if hidden >= 16 { 0 } else { stable })
        .collect()
}

impl StructuralRegistration {
    fn displacement(&self, width: u32, height: u32) -> f64 {
        correction_displacement(self.transform, f64::from(width), f64::from(height))
    }
}

#[cfg(test)]
pub(crate) fn measure_structural_registration(
    template: &Surface,
    current: &Surface,
    structural_mask: &[u8],
    radius: i32,
) -> Option<StructuralRegistration> {
    StructuralMatcher::new(template, structural_mask)?.measure(current, radius)
}

fn select_structural_points(mask: &[u8], width: usize, maximum: usize) -> Vec<(usize, usize)> {
    let eligible = mask.iter().filter(|&&alpha| alpha > 96).count();
    let stride = eligible.div_ceil(maximum.max(1)).max(1);
    mask.iter()
        .enumerate()
        .filter_map(|(index, &alpha)| (alpha > 96).then_some((index % width, index / width)))
        .step_by(stride)
        .take(maximum)
        .collect()
}

fn visible_structural_mask(
    template: &[u8],
    current: &[u8],
    structural: &[u8],
    width: usize,
    height: usize,
    alignment: Mat3,
    search_radius: i32,
) -> Vec<u8> {
    let mut visible = vec![0_u8; structural.len()];
    if template.len() != structural.len()
        || current.len() != structural.len()
        || width < 3
        || height < 3
    {
        return visible;
    }
    let max_x_bound = width - 1;
    let max_y_bound = height - 1;

    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let index = y * width + x;
            let struct_val = structural[index];
            if struct_val <= 96 {
                continue;
            }
            let mapped = alignment.transform(crate::model::PointF {
                x: x as f64,
                y: y as f64,
            });
            let tx = template[index + 1] as f64 - template[index - 1] as f64;
            let ty = template[index + width] as f64 - template[index - width] as f64;
            let template_gradient = tx.hypot(ty);
            let t_val = template[index] as f64;

            'search: for dy in -search_radius..=search_radius {
                for dx in -search_radius..=search_radius {
                    let sx = (mapped.x + f64::from(dx)).round() as isize;
                    let sy = (mapped.y + f64::from(dy)).round() as isize;
                    if sx < 0 || sy < 0 || sx as usize > max_x_bound || sy as usize > max_y_bound {
                        continue;
                    }
                    let sx = sx as usize;
                    let sy = sy as usize;
                    let center = current[sy * width + sx] as f64;
                    let photometric_match = (t_val - center).abs() <= 42.0;
                    if photometric_match {
                        visible[index] = struct_val;
                        break 'search;
                    }

                    if sx == 0 || sy == 0 || sx >= max_x_bound || sy >= max_y_bound {
                        continue;
                    }
                    let left = current[sy * width + (sx - 1)] as f64;
                    let right = current[sy * width + (sx + 1)] as f64;
                    let top = current[(sy - 1) * width + sx] as f64;
                    let bottom = current[(sy + 1) * width + sx] as f64;

                    let cx = right - left;
                    let cy = bottom - top;
                    let current_gradient = cx.hypot(cy);
                    let direction =
                        (tx * cx + ty * cy) / (template_gradient * current_gradient).max(1.0);
                    let ratio = current_gradient / template_gradient.max(1.0);
                    let edge_match = direction >= 0.55 && (0.25..=4.0).contains(&ratio);
                    if edge_match {
                        visible[index] = struct_val;
                        break 'search;
                    }
                }
            }
        }
    }
    visible
}

fn spatially_balanced(points: &[(usize, usize)], width: usize, height: usize) -> bool {
    if points.len() < 48 || width == 0 || height == 0 {
        return false;
    }
    let min_x = points.iter().map(|point| point.0).min().unwrap_or(width);
    let max_x = points.iter().map(|point| point.0).max().unwrap_or(0);
    let min_y = points.iter().map(|point| point.1).min().unwrap_or(height);
    let max_y = points.iter().map(|point| point.1).max().unwrap_or(0);
    if max_x.saturating_sub(min_x) * 2 < width || max_y.saturating_sub(min_y) * 3 < height {
        return false;
    }
    let mut quadrants = [0_usize; 4];
    for &(x, y) in points {
        quadrants[usize::from(x >= width / 2) + usize::from(y >= height / 2) * 2] += 1;
    }
    quadrants.iter().filter(|&&count| count >= 12).count() >= 3
}

fn search_similarity(
    template: &[u8],
    current: &[u8],
    width: usize,
    height: usize,
    points: &[(usize, usize)],
    radius: i32,
) -> StructuralRegistration {
    let center_x = (width.saturating_sub(1)) as f64 * 0.5;
    let center_y = (height.saturating_sub(1)) as f64 * 0.5;
    let transform = |dx: f64, dy: f64, scale: f64| {
        Mat3::translation(center_x + dx, center_y + dy)
            .multiply(Mat3::scale(scale, scale))
            .multiply(Mat3::translation(-center_x, -center_y))
    };
    let mut err_buf = Vec::with_capacity(points.len());
    let before = registration_cost_with_buf(
        template,
        current,
        width,
        height,
        points,
        Mat3::IDENTITY,
        &mut err_buf,
    );
    let mut best = StructuralRegistration {
        transform: Mat3::IDENTITY,
        before,
        after: before,
        ecc: None,
    };
    let mut best_parameters = (0.0, 0.0, 1.0);
    let radius = radius.max(0);
    for scale_step in -4..=4 {
        let scale = 1.0 + scale_step as f64 * 0.005;
        for dy in (-radius..=radius).step_by(2) {
            for dx in (-radius..=radius).step_by(2) {
                let matrix = transform(dx as f64, dy as f64, scale);
                let value = registration_cost_with_buf(
                    template,
                    current,
                    width,
                    height,
                    points,
                    matrix,
                    &mut err_buf,
                );
                if value < best.after {
                    best = StructuralRegistration {
                        transform: matrix,
                        before,
                        after: value,
                        ecc: None,
                    };
                    best_parameters = (dx as f64, dy as f64, scale);
                }
            }
        }
    }
    let (coarse_dx, coarse_dy, coarse_scale) = best_parameters;
    for scale_step in -4..=4 {
        let scale = coarse_scale + scale_step as f64 * 0.001;
        for dy_step in -4..=4 {
            let dy = coarse_dy + dy_step as f64 * 0.25;
            for dx_step in -4..=4 {
                let dx = coarse_dx + dx_step as f64 * 0.25;
                let matrix = transform(dx, dy, scale);
                let value = registration_cost_with_buf(
                    template,
                    current,
                    width,
                    height,
                    points,
                    matrix,
                    &mut err_buf,
                );
                if value < best.after {
                    best = StructuralRegistration {
                        transform: matrix,
                        before,
                        after: value,
                        ecc: None,
                    };
                }
            }
        }
    }
    best
}

fn registration_cost(
    template: &[u8],
    current: &[u8],
    width: usize,
    height: usize,
    points: &[(usize, usize)],
    transform: Mat3,
) -> f64 {
    let mut errors = Vec::with_capacity(points.len());
    registration_cost_with_buf(
        template,
        current,
        width,
        height,
        points,
        transform,
        &mut errors,
    )
}

fn registration_cost_with_buf(
    template: &[u8],
    current: &[u8],
    width: usize,
    height: usize,
    points: &[(usize, usize)],
    transform: Mat3,
    errors: &mut Vec<f64>,
) -> f64 {
    errors.clear();
    for &(x, y) in points {
        let mapped = transform.transform(crate::model::PointF {
            x: x as f64,
            y: y as f64,
        });
        if let Some(value) = sample_gray(current, width, height, mapped.x, mapped.y) {
            errors.push((template[y * width + x] as f64 - value).abs());
        }
    }
    if errors.len() < points.len() / 3 {
        f64::INFINITY
    } else {
        median_f64_slice(errors)
    }
}

fn median_f64_slice(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return f64::INFINITY;
    }
    let middle = values.len() / 2;
    let (_, median, _) = values.select_nth_unstable_by(middle, f64::total_cmp);
    *median
}

fn luma_mat(data: &[u8], width: usize, height: usize) -> Result<Mat> {
    if data.len() != width * height {
        bail!("invalid luma buffer dimensions");
    }
    let mut mat = Mat::new_rows_cols_with_default(
        height as i32,
        width as i32,
        core::CV_8UC1,
        Scalar::all(0.0),
    )?;
    mat.data_bytes_mut()?.copy_from_slice(data);
    Ok(mat)
}

fn warp_mat(transform: Mat3, motion: i32) -> Result<Mat> {
    if motion == cv_video::MOTION_HOMOGRAPHY {
        Ok(Mat::from_slice_2d(
            &transform.values.map(|row| row.map(|value| value as f32)),
        )?)
    } else {
        Ok(Mat::from_slice_2d(&[
            transform.values[0].map(|value| value as f32),
            transform.values[1].map(|value| value as f32),
        ])?)
    }
}

fn mat3_from_warp(warp: &Mat) -> Result<Mat3> {
    let mut values = Mat3::IDENTITY.values;
    for row in 0..warp.rows() {
        for column in 0..warp.cols() {
            values[row as usize][column as usize] = if warp.typ() == core::CV_32F {
                f64::from(*warp.at_2d::<f32>(row, column)?)
            } else {
                *warp.at_2d::<f64>(row, column)?
            };
        }
    }
    Ok(Mat3 { values })
}

fn valid_correction(transform: Mat3, width: usize, height: usize, radius: i32) -> bool {
    let displacement =
        correction_displacement(transform, (width.max(1)) as f64, (height.max(1)) as f64);
    displacement <= f64::from(radius.max(2)) * 1.75
}

fn correction_displacement(transform: Mat3, width: f64, height: f64) -> f64 {
    let max_x = (width - 1.0).max(0.0);
    let max_y = (height - 1.0).max(0.0);
    let corners = [(0.0, 0.0), (max_x, 0.0), (max_x, max_y), (0.0, max_y)];
    corners
        .into_iter()
        .map(|(x, y)| {
            let mapped = transform.transform(crate::model::PointF { x, y });
            (mapped.x - x).hypot(mapped.y - y)
        })
        .sum::<f64>()
        / 4.0
}

#[inline(always)]
fn sample_gray(data: &[u8], width: usize, height: usize, x: f64, y: f64) -> Option<f64> {
    if x < 0.0
        || y < 0.0
        || x > width.saturating_sub(1) as f64
        || y > height.saturating_sub(1) as f64
    {
        return None;
    }
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(width - 1);
    let y1 = (y0 + 1).min(height - 1);
    let tx = x - x0 as f64;
    let ty = y - y0 as f64;
    let row0 = y0 * width;
    let row1 = y1 * width;
    let a = data[row0 + x0] as f64 * (1.0 - tx) + data[row0 + x1] as f64 * tx;
    let b = data[row1 + x0] as f64 * (1.0 - tx) + data[row1 + x1] as f64 * tx;
    Some(a * (1.0 - ty) + b * ty)
}

fn blur_luma(src: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    let mut temp = vec![0u8; src.len()];
    let mut output = vec![0u8; src.len()];
    for y in 0..height {
        for x in 0..width {
            let start = x.saturating_sub(radius);
            let end = (x + radius).min(width - 1);
            let sum: u32 = (start..=end).map(|xx| src[y * width + xx] as u32).sum();
            temp[y * width + x] = (sum / (end - start + 1) as u32) as u8;
        }
    }
    for y in 0..height {
        for x in 0..width {
            let start = y.saturating_sub(radius);
            let end = (y + radius).min(height - 1);
            let sum: u32 = (start..=end).map(|yy| temp[yy * width + x] as u32).sum();
            output[y * width + x] = (sum / (end - start + 1) as u32) as u8;
        }
    }
    output
}

fn grayscale(surface: &Surface) -> Vec<u8> {
    surface
        .pixels()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| {
            ((pixel[0] as u32 * 54 + pixel[1] as u32 * 183 + pixel[2] as u32 * 19) >> 8) as u8
        })
        .collect()
}

fn peak(values: &[f64], start: usize, end: usize) -> Option<usize> {
    let end = end.min(values.len());
    if start >= end {
        return None;
    }
    (start..end).max_by(|&a, &b| values[a].total_cmp(&values[b]))
}

pub(crate) fn evenly_spaced(frames: usize, count: usize) -> Vec<usize> {
    if count <= 1 {
        return vec![0];
    }
    (0..count)
        .map(|index| index * (frames - 1) / (count - 1))
        .collect()
}

fn cavity_area_score(area: f64) -> f64 {
    (1.0 - ((area - 0.56).abs() / 0.56)).clamp(0.0, 1.0)
}

fn structural_area_score(area: f64) -> f64 {
    if area < 0.001 {
        0.0
    } else if (0.003..=0.18).contains(&area) {
        1.0
    } else {
        (area / 0.003).clamp(0.0, 1.0)
    }
}

fn save_surface_png(surface: &Surface, path: &Path) -> Result<()> {
    let image = RgbaImage::from_raw(surface.width(), surface.height(), surface.pixels().to_vec())
        .context("invalid RGBA surface")?;
    image
        .save(path)
        .with_context(|| format!("failed to save RGBA image {}", path.display()))?;
    Ok(())
}

fn save_luma_png(width: u32, height: u32, data: &[u8], path: &Path) -> Result<()> {
    let image: GrayImage = ImageBuffer::<Luma<u8>, _>::from_raw(width, height, data.to_vec())
        .context("invalid luma surface")?;
    image
        .save(path)
        .with_context(|| format!("failed to save grayscale image {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        registration_mask_excluding, spatially_balanced, structural_area_score,
        visible_structural_mask,
    };
    use crate::model::Mat3;

    #[test]
    fn structureless_candidate_has_zero_confidence() {
        assert_eq!(structural_area_score(0.0), 0.0);
        assert_eq!(structural_area_score(0.0009), 0.0);
        assert!(structural_area_score(0.003) > 0.99);
    }

    #[test]
    fn foreground_pixels_are_removed_from_structural_evidence() {
        let (width, height) = (40, 20);
        let template = vec![180_u8; width * height];
        let mut current = template.clone();
        let structural = vec![255_u8; width * height];
        for y in 2..18 {
            for x in 10..30 {
                current[y * width + x] = 0;
            }
        }

        let visible = visible_structural_mask(
            &template,
            &current,
            &structural,
            width,
            height,
            Mat3::IDENTITY,
            4,
        );

        assert_eq!(visible[10 * width + 20], 0);
        assert_eq!(visible[10 * width + 3], 255);
    }

    #[test]
    fn known_foreground_is_removed_before_registration() {
        let stable = vec![255_u8; 8];
        let exclusion = [0, 0, 15, 16, 64, 255, 0, 0];
        assert_eq!(
            registration_mask_excluding(&stable, Some(&exclusion)),
            vec![255, 255, 255, 0, 0, 0, 255, 255]
        );
    }

    #[test]
    fn one_sided_evidence_is_not_spatially_balanced() {
        let one_side = (0..100)
            .map(|index| (index % 10, index / 10))
            .collect::<Vec<_>>();
        assert!(!spatially_balanced(&one_side, 100, 40));
    }
}
