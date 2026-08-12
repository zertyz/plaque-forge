//! Plaque candidate detection.
//!
//! Samples source frames, scores plausible planar title surfaces, and chooses a reference
//! enclosing planar region for tracking or proposes alternatives for human refinement.

use std::{fs, path::Path};

use anyhow::{Context, Result};
use opencv::{
    core::{self, Mat, Point, Rect, Scalar, Vector},
    geometry, imgcodecs, imgproc,
    prelude::*,
    videoio::{CAP_PROP_POS_FRAMES, VideoCapture},
};

use crate::{cli::AnalyzeArgs, model::RectF, video::VideoInfo};

#[derive(Debug, Clone)]
pub struct Candidate {
    pub rect: RectF,
    pub frame_index: usize,
    pub confidence: f64,
    /// How consistently a similar enclosure appears across sampled frames.
    pub temporal_support: f64,
    /// Stricter support for a region staying at nearly the same screen coordinates.
    pub screen_stationarity: f64,
    pub edge_completeness: f64,
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
    screen_stationarity: f64,
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
            confidence: if args.writable_region_hint.is_some() {
                0.98
            } else {
                0.92
            },
            temporal_support: 1.0,
            screen_stationarity: 1.0,
            edge_completeness: 1.0,
            canonical_width: width.round().max(1.0) as u32,
            canonical_height: height.round().max(1.0) as u32,
        });
    }

    detect_proposals(&args.input, args.candidate_samples, info, Some(diagnostics))?
        .map(|report| report.selected)
        .context("no plausible writing-surface candidate found; add the smallest refinement that identifies the intended region")
}

pub fn detect_proposals(
    input: &Path,
    candidate_samples: usize,
    info: &VideoInfo,
    diagnostics: Option<&Path>,
) -> Result<Option<DetectionReport>> {
    let mut capture = crate::video::open_capture(input)?;

    let actual_frames = info.frames.max(1);
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
        write_ranking(
            diagnostics,
            &ranked,
            best,
            info.width as i32,
            info.height as i32,
        )?;
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
    // Preserve the strongest detector hypothesis by default. Earlier releases selected
    // the best temporal/reference frame *within that hypothesis*, which was stable for
    // the established holographic-plaque assets. A global "largest plausible rectangle
    // wins" rule regressed those scenes by letting nearby room/body enclosures steal the
    // title surface.
    //
    // The one useful part of area priority is kept as an escape hatch for a clearly
    // larger, independently plausible writing surface when the strongest local response
    // is a small high-contrast prop (for example a magnifying glass). The alternative
    // must be at least 1.8x the area and normally retain at least 72% of the top detector
    // score; an extremely larger surface (3x+) may recover from a very strong small prop
    // down to a 55% score ratio.
    let initial = ranked.first()?;
    // A broad/architectural response may score first even when a clear compact plaque is
    // also present. Prefer that compact hypothesis when it retains substantial detector
    // support; this restores the older "real plaque beats room enclosure" behavior.
    let initial_is_compact = candidate_is_compact_surface(initial, width, height);
    let seed = if initial_is_compact {
        initial
    } else {
        ranked
            .iter()
            .filter(|candidate| candidate_is_compact_surface(candidate, width, height))
            .filter(|candidate| candidate.score >= initial.score * 0.65)
            .max_by(|left, right| {
                reference_quality(left, width, height)
                    .total_cmp(&reference_quality(right, width, height))
            })
            .unwrap_or(initial)
    };

    // Area dominance is only an escape from a compact high-contrast prop. If the
    // broad top response has already been replaced by a credible compact plaque,
    // applying the escape hatch again would simply select the rejected room/frame.
    let target = if !initial_is_compact && !std::ptr::eq(seed, initial) {
        seed
    } else {
        let seed_area = candidate_area(seed).max(1) as f64;
        ranked
            .iter()
            .filter(|candidate| {
                if same_hypothesis(seed.rect, candidate.rect, width, height)
                    || !(candidate_is_compact_surface(candidate, width, height)
                        || candidate_is_broad_canvas(candidate, width, height))
                {
                    return false;
                }
                let area_ratio = candidate_area(candidate) as f64 / seed_area;
                let score_ratio = candidate.score / seed.score.max(0.001);
                area_ratio >= 1.80
                    && (score_ratio >= 0.72 || (area_ratio >= 3.0 && score_ratio >= 0.55))
            })
            .max_by(|left, right| {
                dominant_surface_quality(left, width, height)
                    .total_cmp(&dominant_surface_quality(right, width, height))
            })
            .unwrap_or(seed)
    };

    // Once a surface hypothesis is chosen, select the clearest representative frame
    // exactly as the pre-regression detector did. Tiny frame-to-frame rectangle changes
    // should never become "pick whichever rectangle has four percent more area".
    ranked
        .iter()
        .filter(|candidate| same_hypothesis(target.rect, candidate.rect, width, height))
        .max_by(|left, right| {
            reference_quality(left, width, height)
                .total_cmp(&reference_quality(right, width, height))
        })
}

fn reference_quality(candidate: &ScoredRect, width: i32, height: i32) -> f64 {
    let area_ratio = candidate_area_ratio(candidate, width, height);
    let resolution = (area_ratio.min(0.32) / 0.32).sqrt();
    let oversize = ((area_ratio - 0.55) / 0.25).clamp(0.0, 1.0);
    candidate.score + 1.5 * candidate.edge_completeness + 2.0 * resolution
        - 1.5 * candidate.interior_clutter
        - 2.0 * oversize
}

fn same_hypothesis(a: Rect, b: Rect, width: i32, height: i32) -> bool {
    let diagonal = f64::from(width).hypot(f64::from(height)).max(1.0);
    let width_change = (f64::from(a.width) / f64::from(b.width)).ln().abs();
    let height_change = (f64::from(a.height) / f64::from(b.height)).ln().abs();
    let a_area = f64::from(a.width.max(1) * a.height.max(1));
    let b_area = f64::from(b.width.max(1) * b.height.max(1));
    let area_ratio = (a_area / b_area).max(b_area / a_area);
    let intersection_width = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
    let intersection_height = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);
    let overlap = f64::from(intersection_width.max(0) * intersection_height.max(0));
    let nested_enclosure = area_ratio > 1.20 && overlap / a_area.min(b_area) > 0.95;
    rect_center_distance(a, b) / diagonal <= 0.18
        && width_change <= 0.45
        && height_change <= 0.45
        && !nested_enclosure
}

fn candidate_area_ratio(candidate: &ScoredRect, width: i32, height: i32) -> f64 {
    candidate_area(candidate) as f64 / f64::from(width * height).max(1.0)
}

fn candidate_has_independent_evidence(candidate: &ScoredRect) -> bool {
    score_to_confidence(candidate.score) >= 0.60
        && (candidate.edge_completeness >= 0.35
            || (candidate.screen_stationarity >= 0.66 && candidate.interior_clutter <= 0.24)
            || (candidate.temporal_support >= 0.58 && candidate.interior_clutter <= 0.22))
}

fn candidate_is_compact_surface(candidate: &ScoredRect, width: i32, height: i32) -> bool {
    let area = candidate_area_ratio(candidate, width, height);
    let aspect = f64::from(candidate.rect.width) / f64::from(candidate.rect.height.max(1));
    candidate_has_independent_evidence(candidate)
        && area <= 0.42
        && score_to_confidence(candidate.score) >= 0.72
        && candidate.edge_completeness >= 0.42
        && candidate.interior_clutter <= 0.48
        // A rectangular plaque may be wide, nearly square, or mildly oval. Very tall
        // compact boxes are much more commonly bodies/doors than title surfaces.
        && aspect >= 0.72
}

fn candidate_is_broad_canvas(candidate: &ScoredRect, width: i32, height: i32) -> bool {
    let area = candidate_area_ratio(candidate, width, height);
    (0.34..=0.78).contains(&area)
        && score_to_confidence(candidate.score) >= 0.78
        && candidate.interior_clutter <= 0.42
        && (candidate.edge_completeness >= 0.18
            || candidate.screen_stationarity >= 0.52
            || candidate.temporal_support >= 0.68)
}

fn dominant_surface_quality(candidate: &ScoredRect, width: i32, height: i32) -> f64 {
    let area = candidate_area_ratio(candidate, width, height);
    candidate.score
        + 0.90 * candidate.edge_completeness
        + 0.70 * candidate.screen_stationarity
        + 0.55 * candidate.temporal_support
        + 0.60 * area.sqrt()
        - 0.65 * candidate.interior_clutter
}

fn candidate_area(candidate: &ScoredRect) -> i64 {
    i64::from(candidate.rect.width) * i64::from(candidate.rect.height)
}

fn write_ranking(
    diagnostics: &Path,
    ranked: &[ScoredRect],
    selected: Option<&ScoredRect>,
    frame_width: i32,
    frame_height: i32,
) -> Result<()> {
    let mut visible = ranked.iter().take(20).collect::<Vec<_>>();
    if let Some(selected) = selected
        && !visible.iter().any(|candidate| {
            candidate.frame_index == selected.frame_index && candidate.rect == selected.rect
        })
    {
        visible.push(selected);
    }
    fs::write(
        diagnostics.join("candidate-ranking.json"),
        serde_json::to_vec_pretty(
            &visible
                .into_iter()
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
                        "screen_stationarity": candidate.screen_stationarity,
                        "area_pixels": candidate_area(candidate),
                        "plausible_compact_surface": candidate_is_compact_surface(candidate, frame_width, frame_height),
                        "plausible_broad_canvas": candidate_is_broad_canvas(candidate, frame_width, frame_height),
                        "area_ratio": candidate_area_ratio(candidate, frame_width, frame_height),
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
        temporal_support: candidate.temporal_support,
        screen_stationarity: candidate.screen_stationarity,
        edge_completeness: candidate.edge_completeness,
        canonical_width: candidate.rect.width as u32,
        canonical_height: candidate.rect.height as u32,
    }
}

fn frame_candidates(frame: &Mat) -> Result<Vec<ScoredRect>> {
    let mut output = Vec::new();
    let edges = geometry_edges(frame)?;

    output.extend(color_candidates(frame)?);
    output.extend(strict_neutral_surface_candidates(frame)?);
    output.extend(neutral_surface_candidates(frame)?);
    output.extend(contour_rectangles(
        &edges,
        frame.cols(),
        frame.rows(),
        1.25,
    )?);
    // Large circular/oval title canvases often have a strong but interrupted arc rather
    // than four connected sides. Bridge moderate gaps before contour extraction so the
    // enclosing region can enter the same ranking pipeline as rectangular plaques.
    output.extend(broad_arc_candidates(&edges, frame.cols(), frame.rows())?);
    output.extend(circle_arc_candidates(&edges, frame.cols(), frame.rows())?);
    output.extend(text_density_candidates(frame)?);

    for candidate in &mut output {
        let evidence = edge_evidence(&edges, candidate.rect)?;
        candidate.edge_completeness = candidate.edge_completeness.max(evidence.border_support);
        candidate.interior_clutter = candidate.interior_clutter.max(evidence.interior_clutter);
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

fn strict_neutral_surface_candidates(frame: &Mat) -> Result<Vec<ScoredRect>> {
    let mut hsv = Mat::default();
    imgproc::cvt_color(
        frame,
        &mut hsv,
        imgproc::COLOR_BGR2HSV,
        0,
        core::AlgorithmHint::ALGO_HINT_DEFAULT,
    )?;
    // Very low saturation isolates bright neutral writing surfaces such as clouds without
    // merging them into a blue sky. The broader neutral vote below remains useful for
    // off-white metal/fabric, but receives a lower source weight.
    let mut neutral = Mat::default();
    core::in_range(
        &hsv,
        &Scalar::new(0.0, 0.0, 170.0, 0.0),
        &Scalar::new(179.0, 20.0, 255.0, 0.0),
        &mut neutral,
    )?;
    let kernel = imgproc::get_structuring_element(
        imgproc::MORPH_ELLIPSE,
        core::Size::new(17, 11),
        Point::new(-1, -1),
    )?;
    let mut closed = Mat::default();
    imgproc::morphology_ex(
        &neutral,
        &mut closed,
        imgproc::MORPH_CLOSE,
        &kernel,
        Point::new(-1, -1),
        2,
        core::BORDER_CONSTANT,
        imgproc::morphology_default_border_value()?,
    )?;
    contour_rectangles(&closed, frame.cols(), frame.rows(), 1.05)
}

fn neutral_surface_candidates(frame: &Mat) -> Result<Vec<ScoredRect>> {
    let mut hsv = Mat::default();
    imgproc::cvt_color(
        frame,
        &mut hsv,
        imgproc::COLOR_BGR2HSV,
        0,
        core::AlgorithmHint::ALGO_HINT_DEFAULT,
    )?;
    // Bright, low-saturation surfaces cover cases such as cloud/fabric plaques that
    // the saturated-color vote intentionally misses. Geometry and temporal support
    // still have to corroborate the region, so this is only another proposal source.
    let mut neutral = Mat::default();
    core::in_range(
        &hsv,
        &Scalar::new(0.0, 0.0, 150.0, 0.0),
        &Scalar::new(179.0, 85.0, 255.0, 0.0),
        &mut neutral,
    )?;
    let kernel = imgproc::get_structuring_element(
        imgproc::MORPH_ELLIPSE,
        core::Size::new(17, 11),
        Point::new(-1, -1),
    )?;
    let mut closed = Mat::default();
    imgproc::morphology_ex(
        &neutral,
        &mut closed,
        imgproc::MORPH_CLOSE,
        &kernel,
        Point::new(-1, -1),
        2,
        core::BORDER_CONSTANT,
        imgproc::morphology_default_border_value()?,
    )?;
    contour_rectangles(&closed, frame.cols(), frame.rows(), 0.72)
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

fn broad_arc_candidates(edges: &Mat, width: i32, height: i32) -> Result<Vec<ScoredRect>> {
    let odd = |value: i32| {
        let value = value.max(15);
        if value % 2 == 0 { value + 1 } else { value }
    };
    let kernel = imgproc::get_structuring_element(
        imgproc::MORPH_ELLIPSE,
        core::Size::new(odd(width / 36), odd(height / 28)),
        Point::new(-1, -1),
    )?;
    let mut closed = Mat::default();
    imgproc::morphology_ex(
        edges,
        &mut closed,
        imgproc::MORPH_CLOSE,
        &kernel,
        Point::new(-1, -1),
        2,
        core::BORDER_CONSTANT,
        imgproc::morphology_default_border_value()?,
    )?;
    let frame_area = f64::from(width * height).max(1.0);
    let mut candidates = contour_rectangles(&closed, width, height, 0.92)?;
    candidates.retain(|candidate| {
        f64::from(candidate.rect.width * candidate.rect.height) / frame_area >= 0.10
    });
    Ok(candidates)
}

/// Propose large circular title canvases even when their circumference is only partly visible.
///
/// Generic contour bounding is deliberately insufficient here: a large quiet circle may have
/// one interrupted luminous arc while a small prop (for example a magnifying glass) has a much
/// cleaner closed contour. We therefore search a small deterministic circle parameter grid,
/// retain only the strongest few hypotheses, and let the normal temporal/area ranking combine
/// them with rectangular proposals.
fn circle_arc_candidates(edges: &Mat, width: i32, height: i32) -> Result<Vec<ScoredRect>> {
    if width <= 0 || height <= 0 {
        return Ok(Vec::new());
    }
    // Expand edge evidence once rather than probing a neighborhood around every sampled
    // circumference point. The circle grid contains thousands of hypotheses, so this
    // keeps the detector practical without weakening its tolerance for broken arcs.
    let kernel = imgproc::get_structuring_element(
        imgproc::MORPH_ELLIPSE,
        core::Size::new(9, 9),
        Point::new(-1, -1),
    )?;
    let mut expanded_edges = Mat::default();
    imgproc::dilate(
        edges,
        &mut expanded_edges,
        &kernel,
        Point::new(-1, -1),
        1,
        core::BORDER_CONSTANT,
        imgproc::morphology_default_border_value()?,
    )?;

    let min_dimension = f64::from(width.min(height));
    let x_steps = 14usize;
    let y_steps = 14usize;
    let radius_steps = 9usize;
    let mut candidates = Vec::new();

    for yi in 0..y_steps {
        let center_y = lerp(
            -0.15 * f64::from(height),
            0.85 * f64::from(height),
            yi,
            y_steps,
        );
        for xi in 0..x_steps {
            let center_x = lerp(
                -0.05 * f64::from(width),
                1.05 * f64::from(width),
                xi,
                x_steps,
            );
            // A title circle may extend beyond the frame, but its center should still be
            // on-screen. This rejects partial circles formed by off-screen data/props.
            if center_x < 0.0
                || center_x > f64::from(width)
                || center_y < -0.05 * f64::from(height)
                || center_y > f64::from(height)
            {
                continue;
            }
            for ri in 0..radius_steps {
                let radius = lerp(0.26 * min_dimension, 0.62 * min_dimension, ri, radius_steps);
                let (support, visible_fraction) =
                    sampled_circle_edge_support(&expanded_edges, center_x, center_y, radius)?;
                if visible_fraction < 0.40 || support < 0.18 {
                    continue;
                }
                let clutter = sampled_circle_interior_clutter(edges, center_x, center_y, radius)?;
                let quiet = 1.0 - clutter;
                let radius_fit = (radius / (0.45 * min_dimension)).clamp(0.45, 1.0).sqrt();
                let score =
                    radius_fit * support * (0.65 + 0.35 * visible_fraction) * (0.45 + 0.55 * quiet);

                let left = (center_x - radius).floor().max(0.0) as i32;
                let top = (center_y - radius).floor().max(0.0) as i32;
                let right = (center_x + radius).ceil().min(f64::from(width)) as i32;
                let bottom = (center_y + radius).ceil().min(f64::from(height)) as i32;
                if right - left < 70 || bottom - top < 70 {
                    continue;
                }
                let rect = Rect::new(left, top, right - left, bottom - top);
                let area_ratio =
                    f64::from(rect.width * rect.height) / f64::from(width * height).max(1.0);
                if !(0.08..=0.82).contains(&area_ratio) {
                    continue;
                }
                candidates.push(ScoredRect {
                    rect,
                    score,
                    edge_completeness: support,
                    interior_clutter: clutter,
                    temporal_support: 0.0,
                    screen_stationarity: 0.0,
                    oversize_penalty: 0.0,
                    frame_index: 0,
                });
            }
        }
    }

    candidates.sort_by(|left, right| right.score.total_cmp(&left.score));
    candidates.truncate(12);
    Ok(candidates)
}

fn lerp(start: f64, end: f64, index: usize, count: usize) -> f64 {
    if count <= 1 {
        return start;
    }
    start + (end - start) * index as f64 / (count - 1) as f64
}

fn sampled_circle_edge_support(
    expanded_edges: &Mat,
    center_x: f64,
    center_y: f64,
    radius: f64,
) -> Result<(f64, f64)> {
    let samples = 96usize;
    let mut visible = 0usize;
    let mut supported = 0usize;
    for sample in 0..samples {
        let angle = std::f64::consts::TAU * sample as f64 / samples as f64;
        let x = (center_x + radius * angle.cos()).round() as i32;
        let y = (center_y + radius * angle.sin()).round() as i32;
        if x < 0 || y < 0 || x >= expanded_edges.cols() || y >= expanded_edges.rows() {
            continue;
        }
        visible += 1;
        supported += usize::from(*expanded_edges.at_2d::<u8>(y, x)? > 0);
    }
    Ok((
        supported as f64 / visible.max(1) as f64,
        visible as f64 / samples as f64,
    ))
}

fn sampled_circle_interior_clutter(
    edges: &Mat,
    center_x: f64,
    center_y: f64,
    radius: f64,
) -> Result<f64> {
    let inner = radius * 0.65;
    let steps = 13usize;
    let mut sampled = 0usize;
    let mut edge_points = 0usize;
    for yi in 0..steps {
        let dy = lerp(-inner, inner, yi, steps);
        for xi in 0..steps {
            let dx = lerp(-inner, inner, xi, steps);
            if dx * dx + dy * dy > inner * inner {
                continue;
            }
            let x = (center_x + dx).round() as i32;
            let y = (center_y + dy).round() as i32;
            if x < 0 || y < 0 || x >= edges.cols() || y >= edges.rows() {
                continue;
            }
            sampled += 1;
            edge_points += usize::from(*edges.at_2d::<u8>(y, x)? > 0);
        }
    }
    let density = edge_points as f64 / sampled.max(1) as f64;
    Ok((density / 0.08).clamp(0.0, 1.0))
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
        if !(0.025..=0.82).contains(&area_ratio) || rect.width < 70 || rect.height < 35 {
            continue;
        }
        let touches_frame = rect.x <= 2
            || rect.y <= 2
            || rect.x + rect.width >= width - 2
            || rect.y + rect.height >= height - 2;
        if touches_frame && area_ratio < 0.30 {
            continue;
        }
        let aspect = rect.width as f64 / rect.height.max(1) as f64;
        if !(0.55..=8.0).contains(&aspect) {
            continue;
        }
        let contour_area = geometry::contour_area(&contour, false)?.abs();
        let rectangularity = (contour_area / (rect.width * rect.height) as f64).clamp(0.0, 1.0);
        let center_y = (rect.y + rect.height / 2) as f64 / height as f64;
        let upper_fit = (1.0 - (center_y - 0.55).max(0.0) / 0.45).clamp(0.0, 1.0);
        let area_fit = if area_ratio <= 0.45 {
            1.0
        } else {
            (1.0 - (area_ratio - 0.45) / 0.37).clamp(0.0, 1.0)
        };
        let wide_fit = (1.0 - ((aspect / 2.8).ln().abs() / 1.4)).clamp(0.0, 1.0);
        let oval_fit = (1.0 - (aspect.ln().abs() / 0.85)).clamp(0.0, 1.0);
        let aspect_fit = wide_fit.max(0.88 * oval_fit);
        let score = source_weight
            * (0.45 * rectangularity + 0.25 * area_fit + 0.20 * aspect_fit + 0.10 * upper_fit);
        result.push(ScoredRect {
            rect,
            score,
            edge_completeness: 0.0,
            interior_clutter: 0.0,
            temporal_support: 0.0,
            screen_stationarity: 0.0,
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
    let rectangular_support = (0.55 * minimum + 0.45 * mean).clamp(0.0, 1.0);
    let elliptical_support = ellipse_edge_support(edges, rect)?;
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
        border_support: rectangular_support.max(0.95 * elliptical_support),
        interior_clutter: (density / 0.16).clamp(0.0, 1.0),
    })
}

fn ellipse_edge_support(edges: &Mat, rect: Rect) -> Result<f64> {
    let center_x = rect.x as f64 + rect.width as f64 * 0.5;
    let center_y = rect.y as f64 + rect.height as f64 * 0.5;
    let radius_x = rect.width as f64 * 0.5;
    let radius_y = rect.height as f64 * 0.5;
    let neighborhood = ((rect.width.min(rect.height) as f64 * 0.02).round() as i32).clamp(2, 10);
    let samples = 144usize;
    let mut supported = 0usize;
    for sample in 0..samples {
        let angle = std::f64::consts::TAU * sample as f64 / samples as f64;
        let x = (center_x + radius_x * angle.cos()).round() as i32;
        let y = (center_y + radius_y * angle.sin()).round() as i32;
        let mut found = false;
        for yy in (y - neighborhood).max(0)..=(y + neighborhood).min(edges.rows() - 1) {
            for xx in (x - neighborhood).max(0)..=(x + neighborhood).min(edges.cols() - 1) {
                if edges.at_2d::<u8>(yy, xx).is_ok_and(|value| *value > 0) {
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
        }
        supported += usize::from(found);
    }
    Ok(supported as f64 / samples as f64)
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
        let stationary = screen_stationarity(candidate, candidates, width, height, sample_count);
        let oversize = candidates
            .iter()
            .filter(|other| other.frame_index == candidate.frame_index)
            .filter_map(|other| oversize_evidence(candidate, other))
            .fold(0.0, f64::max);
        let area_ratio = f64::from(candidate.rect.width * candidate.rect.height)
            / f64::from(width * height).max(1.0);
        let area_penalty = ((area_ratio - 0.60) / 0.22).clamp(0.0, 1.0);
        let resolution_fit = (area_ratio.min(0.32) / 0.32).sqrt();
        let layout_weight = (1.0 - ((area_ratio - 0.30) / 0.35).clamp(0.0, 1.0)) * 0.45;
        let objective = candidate.score
            + 0.80 * persistence
            + 0.85 * candidate.edge_completeness
            + 0.45 * horizontal_center_fit(candidate.rect, width)
            + layout_weight * vertical_layout_fit(candidate.rect, width, height)
            + 0.15 * resolution_fit
            - 0.45 * candidate.interior_clutter
            - 0.35 * area_penalty
            - 1.10 * oversize;
        let mut selected = candidate.clone();
        selected.score = objective;
        selected.temporal_support = persistence;
        selected.screen_stationarity = stationary;
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

fn screen_stationarity(
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
        if center_distance > 0.04 || width_change > 0.12 || height_change > 0.12 {
            continue;
        }
        let score = (-(center_distance / 0.018).powi(2)
            - (width_change / 0.06).powi(2)
            - (height_change / 0.06).powi(2))
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
            screen_stationarity: 0.0,
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
    fn reference_selection_prefers_largest_plausible_surface_over_small_high_contrast_object() {
        let plaque = scored(Rect::new(95, 100, 525, 305), 1.55, 0.42, 20);
        let magnifying_glass = scored(Rect::new(245, 660, 220, 220), 2.60, 0.92, 20);
        let ranked = vec![magnifying_glass, plaque];

        assert_eq!(
            select_reference(&ranked, 720, 1280).unwrap().rect,
            Rect::new(95, 100, 525, 305)
        );
    }

    #[test]
    fn area_priority_does_not_override_a_much_stronger_compact_plaque() {
        let plaque = scored(Rect::new(335, 36, 611, 132), 3.60, 0.99, 41);
        let mut weaker_large = scored(Rect::new(220, 20, 850, 250), 2.05, 0.58, 41);
        weaker_large.interior_clutter = 0.18;
        weaker_large.temporal_support = 0.75;
        let ranked = vec![plaque.clone(), weaker_large];

        assert_eq!(
            select_reference(&ranked, 1280, 720).unwrap().rect,
            plaque.rect
        );
    }

    #[test]
    fn largest_surface_policy_rejects_a_large_busy_architectural_enclosure() {
        let plaque = scored(Rect::new(175, 95, 930, 155), 1.55, 0.62, 20);
        let mut room = scored(Rect::new(286, 162, 706, 448), 1.85, 0.22, 20);
        room.interior_clutter = 0.26;
        room.temporal_support = 0.63;
        let ranked = vec![room, plaque.clone()];

        assert_eq!(
            select_reference(&ranked, 1280, 720).unwrap().rect,
            plaque.rect
        );
    }

    #[test]
    fn largest_surface_policy_still_rejects_an_implausible_giant_region() {
        let plaque = scored(Rect::new(110, 90, 500, 260), 1.70, 0.55, 20);
        let giant_noise = scored(Rect::new(5, 5, 700, 1180), 0.15, 0.04, 20);
        let ranked = vec![plaque.clone(), giant_noise];

        assert_eq!(
            select_reference(&ranked, 720, 1280).unwrap().rect,
            plaque.rect
        );
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
