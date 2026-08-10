use std::path::Path;

use anyhow::{Context, Result, bail};
use image::{GrayImage, ImageBuffer, Luma, RgbaImage};

use crate::{
    geometry::{Point, Quad, homography},
    metadata::HumanMotionTrack,
    model::{Mat3, MotionSample, RectF},
    progress::ProgressReporter,
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
    local_refinement_radius: i32,
    refine_automatic_track: bool,
    human_track: Option<&HumanMotionTrack>,
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
    let mut content_mask = detect_content_cavity(&median);
    let mut structural_mask = detect_structural_mask(&median, &content_mask, &initial_mad);
    progress.finish("initial canonical model");

    progress.start(5, 7, "Plaque structural lock", Some(info.frames));
    if refine_automatic_track {
        refine_motion_from_border(
            ffmpeg,
            input,
            info,
            rect,
            motion,
            &median,
            &structural_mask,
            local_refinement_radius.max(0),
            reference_frame,
            tracking_inertia,
            loop_closed,
            progress,
        )?;
        regularize_refined_motion(motion, rect, reference_frame, tracking_inertia, loop_closed)?;
        if let Some(track) = human_track {
            tracking::reapply_locked_human_constraints(motion, track, rect)?;
        }
        progress.finish("subpixel refinement, smoothing, and human constraints");
    } else {
        progress.update(
            info.frames,
            "reviewed track retained without automatic correction",
        );
        progress.finish("supervised track retained");
    }

    progress.start(6, 7, "Build title-pack assets", Some(sample_indices.len()));
    samples =
        decode_rectified_samples(ffmpeg, input, info, rect, motion, &sample_indices, progress)?;
    let (refined_median, mad) = robust_median(&samples)?;
    median = refined_median;
    content_mask = detect_content_cavity(&median);
    structural_mask = detect_structural_mask(&median, &content_mask, &mad);

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
    let stable = mad.iter().filter(|&&v| v < 18).count() as f64 / mad.len().max(1) as f64;
    let confidence = (0.45 * stable
        + 0.35 * cavity_area_score(cavity_area)
        + 0.20 * structural_area_score(structural_area))
    .clamp(0.0, 0.98);
    progress.finish(format!(
        "cavity {:.1}%, structure {:.1}%",
        cavity_area * 100.0,
        structural_area * 100.0
    ));

    Ok(ExtractionResult {
        median,
        mad,
        content_mask,
        structural_mask,
        cavity_area,
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

fn masked_template(median: &Surface, mask: &[u8]) -> Result<Surface> {
    let mut output = median.clone();
    output.apply_alpha_mask(mask)?;
    Ok(output)
}

#[allow(clippy::too_many_arguments)]
fn refine_motion_from_border(
    ffmpeg: &Path,
    input: &Path,
    info: &VideoInfo,
    rect: RectF,
    motion: &mut [MotionSample],
    template: &Surface,
    structural_mask: &[u8],
    radius: i32,
    reference_frame: usize,
    inertia: f64,
    loop_closed: bool,
    progress: &mut ProgressReporter,
) -> Result<()> {
    let Some(matcher) = StructuralMatcher::new(template, structural_mask) else {
        return Ok(());
    };
    let width = template.width() as usize;
    let height = template.height() as usize;

    let mut decoder = Decoder::spawn(ffmpeg, input, info)?;
    let mut corrections = vec![(0.0_f64, 0.0_f64, 1.0_f64); motion.len().min(info.frames)];
    for frame_index in 0..motion.len().min(info.frames) {
        let Some(frame) = decoder.next_frame()? else {
            break;
        };
        if frame_index == reference_frame {
            progress.update(frame_index + 1, "reference frame fixed");
            continue;
        }
        let current = rectify(&frame, rect, motion[frame_index].transform)?;
        let result = matcher
            .measure(&current, radius)
            .context("rectified plaque dimensions changed during structural lock")?;
        if result.after + 0.25 < result.before {
            corrections[frame_index] = (result.dx, result.dy, result.scale);
            motion[frame_index].reprojection_error =
                motion[frame_index].reprojection_error.min(result.after);
        }
        progress.update(
            frame_index + 1,
            format!("residual {:.2}px", result.displacement()),
        );
    }
    decoder.finish()?;

    let corrections = regularize_similarity_corrections(
        &corrections,
        reference_frame,
        inertia,
        loop_closed,
        width,
        height,
    );
    let cx = rect.x + rect.width * 0.5;
    let cy = rect.y + rect.height * 0.5;
    for (sample, &(dx, dy, scale)) in motion.iter_mut().zip(&corrections) {
        let correction = Mat3::translation(cx, cy)
            .multiply(Mat3::translation(dx, dy))
            .multiply(Mat3::scale(scale, scale))
            .multiply(Mat3::translation(-cx, -cy));
        sample.transform = sample.transform.multiply(correction);
    }
    Ok(())
}

fn regularize_similarity_corrections(
    raw: &[(f64, f64, f64)],
    reference_frame: usize,
    inertia: f64,
    loop_closed: bool,
    width: usize,
    height: usize,
) -> Vec<(f64, f64, f64)> {
    if raw.len() < 3 || inertia <= 0.0 {
        let mut result = raw.to_vec();
        if let Some(reference) = result.get_mut(reference_frame) {
            *reference = (0.0, 0.0, 1.0);
        }
        return result;
    }
    let half_diagonal = (width as f64).hypot(height as f64) * 0.5;
    let median_at = |index: usize, component: usize| {
        let mut values = Vec::with_capacity(5);
        for offset in -2_i32..=2 {
            let candidate = index as i32 + offset;
            let neighbor = if loop_closed {
                candidate.rem_euclid(raw.len() as i32) as usize
            } else {
                candidate.clamp(0, raw.len() as i32 - 1) as usize
            };
            let value = match component {
                0 => raw[neighbor].0,
                1 => raw[neighbor].1,
                _ => raw[neighbor].2,
            };
            values.push(value);
        }
        median_f64(values)
    };
    let mut result = Vec::with_capacity(raw.len());
    for (index, &(dx, dy, scale)) in raw.iter().enumerate() {
        let median = (
            median_at(index, 0),
            median_at(index, 1),
            median_at(index, 2),
        );
        let correction_deviation = (dx - median.0)
            .hypot(dy - median.1)
            .hypot((scale - median.2).abs() * half_diagonal);
        let weight = if correction_deviation > 1.5 {
            1.0
        } else {
            inertia.clamp(0.0, 0.85)
        };
        result.push((
            dx + (median.0 - dx) * weight,
            dy + (median.1 - dy) * weight,
            scale + (median.2 - scale) * weight,
        ));
    }
    if let Some(reference) = result.get_mut(reference_frame) {
        *reference = (0.0, 0.0, 1.0);
    }
    result
}

fn regularize_refined_motion(
    samples: &mut [MotionSample],
    plaque: RectF,
    reference_frame: usize,
    inertia: f64,
    loop_closed: bool,
) -> Result<()> {
    if samples.len() < 13 || inertia <= 0.0 {
        return Ok(());
    }

    let source = Quad::from_rect(plaque.x, plaque.y, plaque.width, plaque.height);
    let raw = samples
        .iter()
        .map(|sample| transformed_rect(plaque, sample.transform))
        .collect::<Vec<_>>();
    let mut smooth = raw.clone();
    let coefficients = [
        -11.0, 0.0, 9.0, 16.0, 21.0, 24.0, 25.0, 24.0, 21.0, 16.0, 9.0, 0.0, -11.0,
    ];
    let strength = inertia.clamp(0.0, 0.98);
    for _ in 0..8 {
        let previous = smooth.clone();
        for index in 0..previous.len() {
            let mut coordinates = [[0.0_f64; 2]; 4];
            for (offset, coefficient) in (-6_i32..=6).zip(coefficients) {
                let candidate = index as i32 + offset;
                let neighbor = if loop_closed {
                    candidate.rem_euclid(previous.len() as i32) as usize
                } else {
                    candidate.clamp(0, previous.len() as i32 - 1) as usize
                };
                for (corner, point) in previous[neighbor].points().into_iter().enumerate() {
                    coordinates[corner][0] += coefficient * point.x / 143.0;
                    coordinates[corner][1] += coefficient * point.y / 143.0;
                }
            }
            let filtered = Quad::new(
                Point::new(coordinates[0][0], coordinates[0][1]),
                Point::new(coordinates[1][0], coordinates[1][1]),
                Point::new(coordinates[2][0], coordinates[2][1]),
                Point::new(coordinates[3][0], coordinates[3][1]),
            );
            let candidate = previous[index].lerp(filtered, strength);
            if candidate.validate("all-frame smoothed plaque").is_ok() {
                smooth[index] = candidate;
            }
        }
    }

    let reference_frame = reference_frame.min(samples.len() - 1);
    let raw_reference = raw[reference_frame].points();
    let smooth_reference = smooth[reference_frame].points();
    let offsets = std::array::from_fn::<_, 4, _>(|corner| {
        Point::new(
            raw_reference[corner].x - smooth_reference[corner].x,
            raw_reference[corner].y - smooth_reference[corner].y,
        )
    });
    for (sample, quad) in samples.iter_mut().zip(smooth) {
        let points = quad.points();
        let aligned = Quad::new(
            Point::new(points[0].x + offsets[0].x, points[0].y + offsets[0].y),
            Point::new(points[1].x + offsets[1].x, points[1].y + offsets[1].y),
            Point::new(points[2].x + offsets[2].x, points[2].y + offsets[2].y),
            Point::new(points[3].x + offsets[3].x, points[3].y + offsets[3].y),
        );
        aligned.validate("reference-aligned smoothed plaque")?;
        let matrix = homography(source, aligned)?;
        sample.transform = Mat3 { values: matrix.m };
    }
    Ok(())
}

pub(crate) struct StructuralRegistration {
    pub dx: f64,
    pub dy: f64,
    pub scale: f64,
    pub before: f64,
    pub after: f64,
}

pub(crate) struct StructuralMatcher {
    template_gray: Vec<u8>,
    width: usize,
    height: usize,
    stable_points: Vec<(usize, usize)>,
}

impl StructuralMatcher {
    pub(crate) fn new(template: &Surface, structural_mask: &[u8]) -> Option<Self> {
        let width = template.width() as usize;
        let height = template.height() as usize;
        if structural_mask.len() != width * height {
            return None;
        }
        let stable_points = select_structural_points(structural_mask, width, 768);
        (stable_points.len() >= 80).then(|| Self {
            template_gray: grayscale(template),
            width,
            height,
            stable_points,
        })
    }

    pub(crate) fn measure(&self, current: &Surface, radius: i32) -> Option<StructuralRegistration> {
        if current.width() as usize != self.width || current.height() as usize != self.height {
            return None;
        }
        Some(search_similarity(
            &self.template_gray,
            &grayscale(current),
            self.width,
            self.height,
            &self.stable_points,
            radius,
        ))
    }
}

impl StructuralRegistration {
    fn displacement(&self) -> f64 {
        self.dx.hypot(self.dy)
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
    let cost = |dx: f64, dy: f64, scale: f64| -> f64 {
        let mut errors = Vec::with_capacity(points.len());
        for &(x, y) in points {
            let source = template[y * width + x] as f64;
            let sample_x = center_x + (x as f64 - center_x) * scale + dx;
            let sample_y = center_y + (y as f64 - center_y) * scale + dy;
            if let Some(value) = sample_gray(current, width, height, sample_x, sample_y) {
                errors.push((source - value).abs());
            }
        }
        if errors.len() < points.len() / 3 {
            f64::INFINITY
        } else {
            median_f64(errors)
        }
    };

    let before = cost(0.0, 0.0, 1.0);
    let mut best = StructuralRegistration {
        dx: 0.0,
        dy: 0.0,
        scale: 1.0,
        before,
        after: before,
    };
    let radius = radius.max(0);
    for scale_step in -4..=4 {
        let scale = 1.0 + scale_step as f64 * 0.005;
        for dy in (-radius..=radius).step_by(2) {
            for dx in (-radius..=radius).step_by(2) {
                let value = cost(dx as f64, dy as f64, scale);
                if value < best.after {
                    best = StructuralRegistration {
                        dx: dx as f64,
                        dy: dy as f64,
                        scale,
                        before,
                        after: value,
                    };
                }
            }
        }
    }
    let coarse_dx = best.dx;
    let coarse_dy = best.dy;
    let coarse_scale = best.scale;
    for scale_step in -4..=4 {
        let scale = coarse_scale + scale_step as f64 * 0.001;
        for dy_step in -4..=4 {
            let dy = coarse_dy + dy_step as f64 * 0.25;
            for dx_step in -4..=4 {
                let dx = coarse_dx + dx_step as f64 * 0.25;
                let value = cost(dx, dy, scale);
                if value < best.after {
                    best = StructuralRegistration {
                        dx,
                        dy,
                        scale,
                        before,
                        after: value,
                    };
                }
            }
        }
    }
    best
}

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
    let a = data[y0 * width + x0] as f64 * (1.0 - tx) + data[y0 * width + x1] as f64 * tx;
    let b = data[y1 * width + x0] as f64 * (1.0 - tx) + data[y1 * width + x1] as f64 * tx;
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
        .chunks_exact(4)
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

fn evenly_spaced(frames: usize, count: usize) -> Vec<usize> {
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

fn median_f64(mut values: Vec<f64>) -> f64 {
    if values.is_empty() {
        return f64::INFINITY;
    }
    let middle = values.len() / 2;
    let (_, median, _) = values.select_nth_unstable_by(middle, f64::total_cmp);
    *median
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
        regularize_refined_motion, regularize_similarity_corrections, structural_area_score,
        transformed_rect,
    };
    use crate::model::{Mat3, MotionSample, RectF};

    #[test]
    fn correction_regularization_removes_an_isolated_scale_translation_spike() {
        let mut corrections = vec![(0.25, -0.25, 1.001); 9];
        corrections[4] = (8.0, -7.0, 1.03);

        let regularized = regularize_similarity_corrections(&corrections, 0, 0.35, false, 458, 268);

        assert_eq!(regularized[0], (0.0, 0.0, 1.0));
        assert!((regularized[4].0 - 0.25).abs() < 1.0e-9);
        assert!((regularized[4].1 + 0.25).abs() < 1.0e-9);
        assert!((regularized[4].2 - 1.001).abs() < 1.0e-9);
    }

    #[test]
    fn all_frame_regularization_removes_frame_to_frame_motion_bounce() {
        let rect = RectF {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 50.0,
        };
        let mut samples = (0..25)
            .map(|frame| MotionSample {
                frame,
                transform: Mat3::translation(frame as f64 + (frame % 2) as f64 * 4.0, 0.0),
                inlier_ratio: 1.0,
                reprojection_error: 0.0,
                ecc: Some(1.0),
                plaque_visibility: 1.0,
                occluder_coverage: 0.0,
            })
            .collect::<Vec<_>>();

        regularize_refined_motion(&mut samples, rect, 0, 0.35, false).unwrap();

        for frame in 1..samples.len() - 1 {
            let before = transformed_rect(rect, samples[frame - 1].transform).tl.x;
            let current = transformed_rect(rect, samples[frame].transform).tl.x;
            let after = transformed_rect(rect, samples[frame + 1].transform).tl.x;
            assert!((current - (before + after) * 0.5).abs() < 0.2);
        }
    }

    #[test]
    fn structureless_candidate_has_zero_confidence() {
        assert_eq!(structural_area_score(0.0), 0.0);
        assert_eq!(structural_area_score(0.0009), 0.0);
        assert!(structural_area_score(0.003) > 0.99);
    }
}
