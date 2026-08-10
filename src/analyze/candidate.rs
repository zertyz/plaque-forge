use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use opencv::{
    core::{self, Mat, Point, Rect, Scalar, Vector},
    geometry, imgcodecs, imgproc,
    prelude::*,
    videoio::{CAP_ANY, CAP_PROP_FRAME_COUNT, CAP_PROP_POS_FRAMES, VideoCapture},
};

use crate::{cli::AnalyzeArgs, model::RectF, video::VideoInfo};

#[derive(Debug, Clone)]
pub struct Candidate {
    pub rect: RectF,
    pub frame_index: usize,
    pub confidence: f64,
    pub canonical_width: u32,
    pub canonical_height: u32,
}

#[derive(Debug, Clone)]
pub struct DetectionReport {
    pub selected: Candidate,
    pub alternatives: Vec<Candidate>,
}

#[derive(Debug, Clone)]
struct ScoredRect {
    rect: Rect,
    score: f64,
    edge_completeness: f64,
    interior_clutter: f64,
    temporal_support: f64,
    oversize_penalty: f64,
    frame_index: usize,
}

pub fn detect(args: &AnalyzeArgs, info: &VideoInfo, diagnostics: &Path) -> Result<Candidate> {
    if let Some([x, y, width, height]) = args.plaque_hint {
        return Ok(Candidate {
            rect: RectF {
                x,
                y,
                width,
                height,
            },
            frame_index: args.plaque_frame.unwrap_or(0),
            confidence: 0.90,
            canonical_width: width.round().max(1.0) as u32,
            canonical_height: height.round().max(1.0) as u32,
        });
    }

    detect_proposals(&args.input, args.candidate_samples, info, Some(diagnostics))?
        .map(|report| report.selected)
        .context("no plausible plaque candidate found; create a refinement and set plaque bounds")
}

pub fn detect_proposals(
    input: &Path,
    candidate_samples: usize,
    info: &VideoInfo,
    diagnostics: Option<&Path>,
) -> Result<Option<DetectionReport>> {
    let mut capture = VideoCapture::from_file(&input.to_string_lossy(), CAP_ANY)?;
    if !capture.is_opened()? {
        bail!("failed to open {}", input.display());
    }

    let actual_frames = capture.get(CAP_PROP_FRAME_COUNT)?.round().max(1.0) as usize;
    let sample_count = candidate_samples.min(actual_frames).max(1);
    let mut candidates = Vec::new();

    for sample in 0..sample_count {
        let frame_index = if sample_count == 1 {
            0
        } else {
            sample * (actual_frames - 1) / (sample_count - 1)
        };
        capture.set(CAP_PROP_POS_FRAMES, frame_index as f64)?;
        let mut frame = Mat::default();
        if !capture.read(&mut frame)? || frame.empty() {
            continue;
        }
        let mut frame_rects = frame_candidates(&frame)?;
        for candidate in &mut frame_rects {
            candidate.frame_index = frame_index;
        }
        candidates.extend(frame_rects);
    }

    let ranked = rank_candidates(
        &candidates,
        info.width as i32,
        info.height as i32,
        sample_count,
    );
    let best = select_reference(&ranked, info.width as i32, info.height as i32);
    if let Some(diagnostics) = diagnostics {
        write_ranking(diagnostics, &ranked, best)?;
    }
    let Some(best) = best else {
        return Ok(None);
    };
    let alternatives = distinct_alternatives(&ranked, best, 3);
    if let Some(diagnostics) = diagnostics {
        write_candidate_image(&mut capture, diagnostics, best, &alternatives)?;
    }

    Ok(Some(DetectionReport {
        selected: candidate_from_scored(best),
        alternatives: alternatives
            .iter()
            .map(|candidate| candidate_from_scored(candidate))
            .collect(),
    }))
}

fn select_reference(ranked: &[ScoredRect], width: i32, height: i32) -> Option<&ScoredRect> {
    let target = ranked.first()?;
    ranked
        .iter()
        .filter(|candidate| same_hypothesis(target.rect, candidate.rect, width, height))
        .max_by(|left, right| {
            reference_quality(left, width, height)
                .total_cmp(&reference_quality(right, width, height))
        })
}

fn reference_quality(candidate: &ScoredRect, width: i32, height: i32) -> f64 {
    let area_ratio = f64::from(candidate.rect.width * candidate.rect.height)
        / f64::from(width * height).max(1.0);
    let resolution = (area_ratio.min(0.22) / 0.22).sqrt();
    let oversize = ((area_ratio - 0.22) / 0.20).clamp(0.0, 1.0);
    candidate.score + 1.5 * candidate.edge_completeness + 2.0 * resolution
        - 1.5 * candidate.interior_clutter
        - 2.0 * oversize
}

fn same_hypothesis(a: Rect, b: Rect, width: i32, height: i32) -> bool {
    let diagonal = f64::from(width).hypot(f64::from(height)).max(1.0);
    let width_change = (f64::from(a.width) / f64::from(b.width)).ln().abs();
    let height_change = (f64::from(a.height) / f64::from(b.height)).ln().abs();
    rect_center_distance(a, b) / diagonal <= 0.18 && width_change <= 0.45 && height_change <= 0.45
}

fn write_ranking(
    diagnostics: &Path,
    ranked: &[ScoredRect],
    selected: Option<&ScoredRect>,
) -> Result<()> {
    fs::write(
        diagnostics.join("candidate-ranking.json"),
        serde_json::to_vec_pretty(
            &ranked
                .iter()
                .take(20)
                .map(|candidate| {
                    serde_json::json!({
                        "frame": candidate.frame_index,
                        "rect": [candidate.rect.x, candidate.rect.y, candidate.rect.width, candidate.rect.height],
                        "selected": selected.is_some_and(|selected| {
                            selected.frame_index == candidate.frame_index && selected.rect == candidate.rect
                        }),
                        "score": candidate.score,
                        "confidence": score_to_confidence(candidate.score),
                        "edge_completeness": candidate.edge_completeness,
                        "interior_clutter": candidate.interior_clutter,
                        "temporal_support": candidate.temporal_support,
                        "oversize_penalty": candidate.oversize_penalty,
                    })
                })
                .collect::<Vec<_>>(),
        )?,
    )?;
    Ok(())
}

fn distinct_alternatives<'a>(
    ranked: &'a [ScoredRect],
    selected: &ScoredRect,
    limit: usize,
) -> Vec<&'a ScoredRect> {
    let mut alternatives: Vec<&ScoredRect> = Vec::new();
    for candidate in ranked {
        if candidate.frame_index != selected.frame_index
            || candidate.rect == selected.rect
            || rect_iou(candidate.rect, selected.rect) >= 0.5
            || alternatives
                .iter()
                .any(|other| rect_iou(candidate.rect, other.rect) >= 0.5)
        {
            continue;
        }
        alternatives.push(candidate);
        if alternatives.len() == limit {
            break;
        }
    }
    alternatives
}

fn rect_iou(a: Rect, b: Rect) -> f64 {
    let left = a.x.max(b.x);
    let top = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    let intersection = (right - left).max(0) * (bottom - top).max(0);
    let union = a.width * a.height + b.width * b.height - intersection;
    f64::from(intersection) / f64::from(union.max(1))
}

fn write_candidate_image(
    capture: &mut VideoCapture,
    diagnostics: &Path,
    selected: &ScoredRect,
    alternatives: &[&ScoredRect],
) -> Result<()> {
    capture.set(CAP_PROP_POS_FRAMES, selected.frame_index as f64)?;
    let mut frame = Mat::default();
    if !capture.read(&mut frame)? || frame.empty() {
        return Ok(());
    }
    draw_candidate(
        &mut frame,
        selected.rect,
        "selected",
        Scalar::new(0.0, 255.0, 255.0, 0.0),
    )?;
    for (index, candidate) in alternatives.iter().enumerate() {
        draw_candidate(
            &mut frame,
            candidate.rect,
            &format!("alternative {}", index + 1),
            Scalar::new(255.0, 180.0, 0.0, 0.0),
        )?;
    }
    imgcodecs::imwrite(
        &diagnostics.join("candidate.png").to_string_lossy(),
        &frame,
        &Vector::new(),
    )?;
    Ok(())
}

fn draw_candidate(frame: &mut Mat, rect: Rect, label: &str, color: Scalar) -> Result<()> {
    imgproc::rectangle(frame, rect, color, 3, imgproc::LINE_AA, 0)?;
    imgproc::put_text(
        frame,
        label,
        Point::new(rect.x, (rect.y - 8).max(18)),
        imgproc::FONT_HERSHEY_SIMPLEX,
        0.55,
        color,
        2,
        imgproc::LINE_AA,
        false,
    )?;
    Ok(())
}

fn candidate_from_scored(candidate: &ScoredRect) -> Candidate {
    Candidate {
        rect: RectF {
            x: candidate.rect.x as f64,
            y: candidate.rect.y as f64,
            width: candidate.rect.width as f64,
            height: candidate.rect.height as f64,
        },
        frame_index: candidate.frame_index,
        confidence: score_to_confidence(candidate.score),
        canonical_width: candidate.rect.width as u32,
        canonical_height: candidate.rect.height as u32,
    }
}

fn frame_candidates(frame: &Mat) -> Result<Vec<ScoredRect>> {
    let mut output = Vec::new();
    let edges = geometry_edges(frame)?;

    output.extend(color_candidates(frame)?);
    output.extend(contour_rectangles(
        &edges,
        frame.cols(),
        frame.rows(),
        1.25,
    )?);
    output.extend(text_density_candidates(frame)?);

    for candidate in &mut output {
        let evidence = edge_evidence(&edges, candidate.rect)?;
        candidate.edge_completeness = evidence.border_support;
        candidate.interior_clutter = evidence.interior_clutter;
    }
    Ok(output)
}

fn color_candidates(frame: &Mat) -> Result<Vec<ScoredRect>> {
    let mut hsv = Mat::default();
    imgproc::cvt_color(
        frame,
        &mut hsv,
        imgproc::COLOR_BGR2HSV,
        0,
        core::AlgorithmHint::ALGO_HINT_DEFAULT,
    )?;

    // Color is deliberately broad. It is only one vote; geometry and persistence
    // must corroborate it in the ensemble detector.
    let mut saturated = Mat::default();
    core::in_range(
        &hsv,
        &Scalar::new(0.0, 45.0, 70.0, 0.0),
        &Scalar::new(179.0, 255.0, 255.0, 0.0),
        &mut saturated,
    )?;
    contour_rectangles(&saturated, frame.cols(), frame.rows(), 1.0)
}

fn geometry_edges(frame: &Mat) -> Result<Mat> {
    let mut gray = Mat::default();
    imgproc::cvt_color(
        frame,
        &mut gray,
        imgproc::COLOR_BGR2GRAY,
        0,
        core::AlgorithmHint::ALGO_HINT_DEFAULT,
    )?;
    let mut blurred = Mat::default();
    imgproc::gaussian_blur(
        &gray,
        &mut blurred,
        core::Size::new(5, 5),
        1.2,
        1.2,
        core::BORDER_DEFAULT,
        core::AlgorithmHint::ALGO_HINT_DEFAULT,
    )?;
    let mut edges = Mat::default();
    imgproc::canny(&blurred, &mut edges, 60.0, 160.0, 3, false)?;
    Ok(edges)
}

fn text_density_candidates(frame: &Mat) -> Result<Vec<ScoredRect>> {
    let mut gray = Mat::default();
    imgproc::cvt_color(
        frame,
        &mut gray,
        imgproc::COLOR_BGR2GRAY,
        0,
        core::AlgorithmHint::ALGO_HINT_DEFAULT,
    )?;
    let mut gradient = Mat::default();
    imgproc::sobel(
        &gray,
        &mut gradient,
        core::CV_8U,
        1,
        0,
        3,
        1.0,
        0.0,
        core::BORDER_DEFAULT,
    )?;
    let kernel = imgproc::get_structuring_element(
        imgproc::MORPH_RECT,
        core::Size::new(17, 5),
        Point::new(-1, -1),
    )?;
    let mut closed = Mat::default();
    imgproc::morphology_ex(
        &gradient,
        &mut closed,
        imgproc::MORPH_CLOSE,
        &kernel,
        Point::new(-1, -1),
        2,
        core::BORDER_CONSTANT,
        imgproc::morphology_default_border_value()?,
    )?;
    contour_rectangles(&closed, frame.cols(), frame.rows(), 0.8)
}

fn contour_rectangles(
    mask: &Mat,
    width: i32,
    height: i32,
    source_weight: f64,
) -> Result<Vec<ScoredRect>> {
    let mut contours = Vector::<Vector<Point>>::new();
    imgproc::find_contours(
        mask,
        &mut contours,
        imgproc::RETR_LIST,
        imgproc::CHAIN_APPROX_SIMPLE,
        Point::new(0, 0),
    )?;

    let frame_area = (width * height) as f64;
    let mut result = Vec::new();
    for contour in contours {
        let rect = geometry::bounding_rect(&contour)?;
        let area_ratio = (rect.width * rect.height) as f64 / frame_area;
        // The supported source class contains one dominant plaque. Small HUD,
        // terminal, and monitor rectangles are not valid automatic candidates.
        if !(0.04..=0.55).contains(&area_ratio) || rect.width < 80 || rect.height < 35 {
            continue;
        }
        if rect.x <= 2
            || rect.y <= 2
            || rect.x + rect.width >= width - 2
            || rect.y + rect.height >= height - 2
        {
            continue;
        }
        let aspect = rect.width as f64 / rect.height.max(1) as f64;
        if !(1.15..=8.0).contains(&aspect) {
            continue;
        }
        let contour_area = geometry::contour_area(&contour, false)?.abs();
        let rectangularity = (contour_area / (rect.width * rect.height) as f64).clamp(0.0, 1.0);
        let center_y = (rect.y + rect.height / 2) as f64 / height as f64;
        let upper_fit = (1.0 - (center_y - 0.55).max(0.0) / 0.45).clamp(0.0, 1.0);
        let area_fit = if area_ratio <= 0.30 {
            1.0
        } else {
            (1.0 - (area_ratio - 0.30) / 0.25).clamp(0.0, 1.0)
        };
        let aspect_fit = (1.0 - ((aspect / 2.8).ln().abs() / 1.4)).clamp(0.0, 1.0);
        let score = source_weight
            * (0.45 * rectangularity + 0.25 * area_fit + 0.20 * aspect_fit + 0.10 * upper_fit);
        result.push(ScoredRect {
            rect,
            score,
            edge_completeness: 0.0,
            interior_clutter: 0.0,
            temporal_support: 0.0,
            oversize_penalty: 0.0,
            frame_index: 0,
        });
    }
    Ok(result)
}

struct EdgeEvidence {
    border_support: f64,
    interior_clutter: f64,
}

fn edge_evidence(edges: &Mat, rect: Rect) -> Result<EdgeEvidence> {
    let horizontal_radius = ((rect.height as f64 * 0.035).round() as i32).clamp(2, 12);
    let vertical_radius = ((rect.width as f64 * 0.018).round() as i32).clamp(2, 12);
    let horizontal = |y: i32| -> Result<f64> {
        let mut supported = 0usize;
        let mut total = 0usize;
        for x in (rect.x..rect.x + rect.width).step_by(2) {
            total += 1;
            let start = (y - horizontal_radius).max(0);
            let end = (y + horizontal_radius).min(edges.rows() - 1);
            if (start..=end).any(|yy| edges.at_2d::<u8>(yy, x).is_ok_and(|value| *value > 0)) {
                supported += 1;
            }
        }
        Ok(supported as f64 / total.max(1) as f64)
    };
    let vertical = |x: i32| -> Result<f64> {
        let mut supported = 0usize;
        let mut total = 0usize;
        for y in (rect.y..rect.y + rect.height).step_by(2) {
            total += 1;
            let start = (x - vertical_radius).max(0);
            let end = (x + vertical_radius).min(edges.cols() - 1);
            if (start..=end).any(|xx| edges.at_2d::<u8>(y, xx).is_ok_and(|value| *value > 0)) {
                supported += 1;
            }
        }
        Ok(supported as f64 / total.max(1) as f64)
    };
    let sides = [
        horizontal(rect.y)?,
        horizontal(rect.y + rect.height - 1)?,
        vertical(rect.x)?,
        vertical(rect.x + rect.width - 1)?,
    ];
    let minimum = sides.iter().copied().fold(f64::INFINITY, f64::min);
    let mean = sides.iter().sum::<f64>() / sides.len() as f64;
    let inner = Rect::new(
        rect.x + rect.width / 5,
        rect.y + rect.height / 5,
        (rect.width * 3 / 5).max(1),
        (rect.height * 3 / 5).max(1),
    );
    let roi = Mat::roi(edges, inner)?;
    let density =
        core::count_non_zero(&roi)? as f64 / f64::from(inner.width * inner.height).max(1.0);
    Ok(EdgeEvidence {
        border_support: (0.55 * minimum + 0.45 * mean).clamp(0.0, 1.0),
        interior_clutter: (density / 0.16).clamp(0.0, 1.0),
    })
}

fn rank_candidates(
    candidates: &[ScoredRect],
    width: i32,
    height: i32,
    sample_count: usize,
) -> Vec<ScoredRect> {
    let mut ranked = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let persistence = temporal_support(candidate, candidates, width, height, sample_count);
        let oversize = candidates
            .iter()
            .filter(|other| other.frame_index == candidate.frame_index)
            .filter_map(|other| oversize_evidence(candidate, other))
            .fold(0.0, f64::max);
        let area_ratio = f64::from(candidate.rect.width * candidate.rect.height)
            / f64::from(width * height).max(1.0);
        let area_penalty = ((area_ratio - 0.22) / 0.20).clamp(0.0, 1.0);
        let resolution_fit = (area_ratio.min(0.22) / 0.22).sqrt();
        let objective = candidate.score
            + 0.80 * persistence
            + 0.85 * candidate.edge_completeness
            + 0.45 * horizontal_center_fit(candidate.rect, width)
            + 0.45 * vertical_layout_fit(candidate.rect, width, height)
            + 0.15 * resolution_fit
            - 0.45 * candidate.interior_clutter
            - 0.35 * area_penalty
            - 1.10 * oversize;
        let mut selected = candidate.clone();
        selected.score = objective;
        selected.temporal_support = persistence;
        selected.oversize_penalty = oversize;
        selected.rect.x = selected.rect.x.clamp(0, width - 1);
        selected.rect.y = selected.rect.y.clamp(0, height - 1);
        selected.rect.width = selected.rect.width.min(width - selected.rect.x).max(1);
        selected.rect.height = selected.rect.height.min(height - selected.rect.y).max(1);
        ranked.push(selected);
    }
    ranked.sort_by(|left, right| right.score.total_cmp(&left.score));
    ranked
}

fn horizontal_center_fit(rect: Rect, frame_width: i32) -> f64 {
    let center =
        (f64::from(rect.x) + f64::from(rect.width) * 0.5) / f64::from(frame_width).max(1.0);
    (1.0 - (center - 0.5).abs() * 2.0).clamp(0.0, 1.0)
}

fn vertical_layout_fit(rect: Rect, frame_width: i32, frame_height: i32) -> f64 {
    let center =
        (f64::from(rect.y) + f64::from(rect.height) * 0.5) / f64::from(frame_height).max(1.0);
    let expected = if frame_width >= frame_height {
        0.22
    } else {
        0.27
    };
    (-((center - expected) / 0.18).powi(2)).exp()
}

fn temporal_support(
    candidate: &ScoredRect,
    candidates: &[ScoredRect],
    width: i32,
    height: i32,
    sample_count: usize,
) -> f64 {
    let diagonal = f64::from(width).hypot(f64::from(height)).max(1.0);
    let mut by_frame = std::collections::HashMap::<usize, f64>::new();
    for other in candidates {
        let center_distance = rect_center_distance(candidate.rect, other.rect) / diagonal;
        let width_change = (f64::from(other.rect.width) / f64::from(candidate.rect.width))
            .ln()
            .abs();
        let height_change = (f64::from(other.rect.height) / f64::from(candidate.rect.height))
            .ln()
            .abs();
        if center_distance > 0.20 || width_change > 0.45 || height_change > 0.45 {
            continue;
        }
        let score = (-(center_distance / 0.12).powi(2)
            - (width_change / 0.30).powi(2)
            - (height_change / 0.30).powi(2))
        .exp();
        by_frame
            .entry(other.frame_index)
            .and_modify(|current| *current = current.max(score))
            .or_insert(score);
    }
    by_frame.values().sum::<f64>() / sample_count.max(1) as f64
}

fn oversize_evidence(candidate: &ScoredRect, other: &ScoredRect) -> Option<f64> {
    if candidate.rect == other.rect {
        return None;
    }
    let candidate_area = f64::from(candidate.rect.width * candidate.rect.height).max(1.0);
    let other_area = f64::from(other.rect.width * other.rect.height).max(1.0);
    let area_ratio = other_area / candidate_area;
    if !(0.12..=0.88).contains(&area_ratio) {
        return None;
    }
    let intersection = rect_intersection(candidate.rect, other.rect);
    let containment = intersection / other_area;
    let edge_gain = (other.edge_completeness - candidate.edge_completeness).max(0.0);
    let clutter_gain = (candidate.interior_clutter - other.interior_clutter).max(0.0);
    (containment >= 0.90 && edge_gain + clutter_gain >= 0.025)
        .then_some(containment * (1.0 - area_ratio) * (0.25 + edge_gain + clutter_gain))
}

fn rect_center_distance(a: Rect, b: Rect) -> f64 {
    let ax = f64::from(a.x) + f64::from(a.width) * 0.5;
    let ay = f64::from(a.y) + f64::from(a.height) * 0.5;
    let bx = f64::from(b.x) + f64::from(b.width) * 0.5;
    let by = f64::from(b.y) + f64::from(b.height) * 0.5;
    (ax - bx).hypot(ay - by)
}

fn rect_intersection(a: Rect, b: Rect) -> f64 {
    let width = ((a.x + a.width).min(b.x + b.width) - a.x.max(b.x)).max(0);
    let height = ((a.y + a.height).min(b.y + b.height) - a.y.max(b.y)).max(0);
    f64::from(width * height)
}

fn score_to_confidence(score: f64) -> f64 {
    (1.0 - (-score.max(0.0)).exp()).clamp(0.0, 0.98)
}

#[cfg(test)]
mod tests {
    use super::{ScoredRect, distinct_alternatives, rank_candidates, select_reference};
    use opencv::core::Rect;

    fn scored(rect: Rect, score: f64, edge: f64, frame: usize) -> ScoredRect {
        ScoredRect {
            rect,
            score,
            edge_completeness: edge,
            interior_clutter: 0.0,
            temporal_support: 0.0,
            oversize_penalty: 0.0,
            frame_index: frame,
        }
    }

    #[test]
    fn nested_plaque_geometry_outranks_an_internal_strip() {
        let candidates = vec![
            scored(Rect::new(100, 100, 520, 300), 0.55, 0.8, 0),
            scored(Rect::new(120, 210, 480, 130), 0.80, 0.2, 0),
            scored(Rect::new(170, 150, 360, 100), 0.30, 0.4, 0),
        ];

        let ranked = rank_candidates(&candidates, 720, 1280, 1);

        assert_eq!(ranked[0].rect, candidates[0].rect);
    }

    #[test]
    fn alternatives_share_the_reference_frame_and_suppress_overlaps() {
        let ranked = vec![
            scored(Rect::new(100, 100, 500, 250), 3.0, 0.9, 12),
            scored(Rect::new(110, 110, 490, 240), 2.9, 0.8, 12),
            scored(Rect::new(700, 100, 300, 150), 2.8, 0.8, 12),
            scored(Rect::new(50, 50, 300, 150), 2.7, 0.8, 24),
        ];

        let alternatives = distinct_alternatives(&ranked, &ranked[0], 3);

        assert_eq!(alternatives.len(), 1);
        assert_eq!(alternatives[0].rect, ranked[2].rect);
    }

    #[test]
    fn persistence_requires_position_consistency() {
        let target = scored(Rect::new(100, 80, 400, 140), 0.5, 0.8, 0);
        let candidates = vec![
            target.clone(),
            scored(Rect::new(108, 84, 395, 142), 0.5, 0.8, 1),
            scored(Rect::new(700, 500, 400, 140), 0.5, 0.8, 2),
        ];

        let ranked = rank_candidates(&candidates, 1280, 720, 3);

        assert!(ranked[0].temporal_support > ranked[2].temporal_support);
    }

    #[test]
    fn reference_selection_prefers_the_clearest_target_frame() {
        let mut large = scored(Rect::new(84, 55, 557, 321), 2.40, 0.86, 72);
        large.interior_clutter = 0.08;
        let ranked = vec![scored(Rect::new(128, 150, 476, 270), 2.62, 0.78, 20), large];

        assert_eq!(
            select_reference(&ranked, 720, 1280).unwrap().frame_index,
            72
        );
    }

    #[test]
    fn reference_selection_rejects_an_oversized_enclosure() {
        let mut full = scored(Rect::new(327, 30, 641, 285), 2.30, 0.42, 72);
        full.interior_clutter = 0.51;
        let mut oversized = scored(Rect::new(335, 29, 781, 301), 2.25, 0.59, 197);
        oversized.interior_clutter = 0.55;
        let ranked = vec![full, oversized];

        assert_eq!(
            select_reference(&ranked, 1280, 720).unwrap().frame_index,
            72
        );
    }
}
