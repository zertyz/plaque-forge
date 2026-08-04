use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use opencv::{
    core::{self, Mat, Point, Rect, Scalar, Vector},
    geometry, imgcodecs, imgproc,
    prelude::*,
    videoio::{CAP_ANY, CAP_PROP_FRAME_COUNT, CAP_PROP_POS_FRAMES, VideoCapture},
};

use crate::{
    cli::{AnalyzeArgs, CandidateDetector},
    model::RectF,
    video::VideoInfo,
};

pub struct Candidate {
    pub rect: RectF,
    pub frame_index: usize,
    pub confidence: f64,
    pub canonical_width: u32,
    pub canonical_height: u32,
}

#[derive(Debug, Clone)]
struct ScoredRect {
    rect: Rect,
    score: f64,
    edge_completeness: f64,
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
            frame_index: 0,
            confidence: 0.90,
            canonical_width: width.round().max(1.0) as u32,
            canonical_height: height.round().max(1.0) as u32,
        });
    }

    let mut capture = VideoCapture::from_file(&args.input.to_string_lossy(), CAP_ANY)?;
    if !capture.is_opened()? {
        bail!("failed to open {}", args.input.display());
    }

    let actual_frames = capture.get(CAP_PROP_FRAME_COUNT)?.round().max(1.0) as usize;
    let sample_count = args.candidate_samples.min(actual_frames).max(1);
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
        let mut frame_rects = frame_candidates(&frame, args.detector)?;
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
                        "score": candidate.score,
                        "edge_completeness": candidate.edge_completeness,
                    })
                })
                .collect::<Vec<_>>(),
        )?,
    )?;
    let best = ranked
        .into_iter()
        .next()
        .context("no plausible plaque candidate found; use --plaque-hint x,y,w,h")?;

    capture.set(CAP_PROP_POS_FRAMES, best.frame_index as f64)?;
    let mut frame = Mat::default();
    if capture.read(&mut frame)? && !frame.empty() {
        imgproc::rectangle(
            &mut frame,
            best.rect,
            Scalar::new(0.0, 255.0, 255.0, 0.0),
            3,
            imgproc::LINE_AA,
            0,
        )?;
        imgcodecs::imwrite(
            &diagnostics.join("candidate.png").to_string_lossy(),
            &frame,
            &Vector::new(),
        )?;
    }

    Ok(Candidate {
        rect: RectF {
            x: best.rect.x as f64,
            y: best.rect.y as f64,
            width: best.rect.width as f64,
            height: best.rect.height as f64,
        },
        frame_index: best.frame_index,
        confidence: score_to_confidence(best.score),
        canonical_width: best.rect.width as u32,
        canonical_height: best.rect.height as u32,
    })
}

fn frame_candidates(frame: &Mat, detector: CandidateDetector) -> Result<Vec<ScoredRect>> {
    let mut output = Vec::new();
    let edges = geometry_edges(frame)?;

    if matches!(
        detector,
        CandidateDetector::Ensemble | CandidateDetector::Color
    ) {
        output.extend(color_candidates(frame)?);
    }
    if matches!(
        detector,
        CandidateDetector::Ensemble | CandidateDetector::Geometry
    ) {
        output.extend(contour_rectangles(
            &edges,
            frame.cols(),
            frame.rows(),
            1.25,
        )?);
    }
    if matches!(
        detector,
        CandidateDetector::Ensemble | CandidateDetector::Text
    ) {
        output.extend(text_density_candidates(frame)?);
    }

    for candidate in &mut output {
        candidate.edge_completeness = edge_completeness(&edges, candidate.rect)?;
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
        let center_bias = 1.0
            - (((rect.y + rect.height / 2) as f64 / height as f64) - 0.35)
                .abs()
                .min(1.0);
        let score = source_weight
            * (0.35 * rectangularity
                + 0.30 * area_ratio.sqrt()
                + 0.20 * center_bias
                + 0.15 * aspect.min(4.0) / 4.0);
        result.push(ScoredRect {
            rect,
            score,
            edge_completeness: 0.0,
            frame_index: 0,
        });
    }
    Ok(result)
}

fn edge_completeness(edges: &Mat, rect: Rect) -> Result<f64> {
    let horizontal_band = (rect.height as f64 * 0.22).round().max(3.0) as i32;
    let vertical_band = (rect.width as f64 * 0.08).round().max(3.0) as i32;
    let top = Rect::new(rect.x, rect.y, rect.width, horizontal_band.min(rect.height));
    let bottom = Rect::new(
        rect.x,
        rect.y + rect.height - horizontal_band.min(rect.height),
        rect.width,
        horizontal_band.min(rect.height),
    );
    let left = Rect::new(rect.x, rect.y, vertical_band.min(rect.width), rect.height);
    let right = Rect::new(
        rect.x + rect.width - vertical_band.min(rect.width),
        rect.y,
        vertical_band.min(rect.width),
        rect.height,
    );
    let density = |band: Rect| -> Result<f64> {
        let roi = Mat::roi(edges, band)?;
        Ok(core::count_non_zero(&roi)? as f64 / f64::from(band.width * band.height).max(1.0))
    };
    let scores = [
        density(top)?,
        density(bottom)?,
        density(left)?,
        density(right)?,
    ]
    .map(|value| value / (value + 0.035));
    let minimum = scores.iter().copied().fold(f64::INFINITY, f64::min);
    let mean = scores.iter().sum::<f64>() / scores.len() as f64;
    Ok((0.65 * minimum + 0.35 * mean).clamp(0.0, 1.0))
}

fn rank_candidates(
    candidates: &[ScoredRect],
    width: i32,
    height: i32,
    sample_count: usize,
) -> Vec<ScoredRect> {
    let mut ranked = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let aspect = candidate.rect.width as f64 / candidate.rect.height.max(1) as f64;
        let area = (candidate.rect.width * candidate.rect.height).max(1) as f64;
        let mut frames = std::collections::HashSet::new();
        for other in candidates {
            let oa = other.rect.width as f64 / other.rect.height.max(1) as f64;
            let oarea = (other.rect.width * other.rect.height).max(1) as f64;
            if ((oa / aspect).ln()).abs() < 0.18 && ((oarea / area).ln()).abs() < 0.70 {
                frames.insert(other.frame_index);
            }
        }
        let persistence = (frames.len() as f64 / sample_count.max(1) as f64).min(1.0);
        let area_ratio = area / f64::from(width * height);
        let nested_evidence = candidates
            .iter()
            .filter(|other| {
                if other.frame_index != candidate.frame_index {
                    return false;
                }
                let other_area = f64::from(other.rect.width * other.rect.height);
                let ratio = other_area / area;
                (0.20..=0.85).contains(&ratio)
                    && other.rect.x >= candidate.rect.x
                    && other.rect.y >= candidate.rect.y
                    && other.rect.x + other.rect.width <= candidate.rect.x + candidate.rect.width
                    && other.rect.y + other.rect.height <= candidate.rect.y + candidate.rect.height
            })
            .count()
            .min(3) as f64
            / 3.0;
        let objective = candidate.score
            + 0.65 * persistence
            + 0.80 * area_ratio.sqrt()
            + 0.40 * candidate.edge_completeness
            + 0.42 * nested_evidence;
        let mut selected = candidate.clone();
        selected.score = objective;
        selected.rect.x = selected.rect.x.clamp(0, width - 1);
        selected.rect.y = selected.rect.y.clamp(0, height - 1);
        selected.rect.width = selected.rect.width.min(width - selected.rect.x).max(1);
        selected.rect.height = selected.rect.height.min(height - selected.rect.y).max(1);
        ranked.push(selected);
    }
    ranked.sort_by(|left, right| right.score.total_cmp(&left.score));
    ranked
}

fn score_to_confidence(score: f64) -> f64 {
    (1.0 - (-score.max(0.0)).exp()).clamp(0.0, 0.98)
}

#[cfg(test)]
mod tests {
    use super::{ScoredRect, rank_candidates};
    use opencv::core::Rect;

    #[test]
    fn nested_plaque_geometry_outranks_an_internal_strip() {
        let candidates = vec![
            ScoredRect {
                rect: Rect::new(100, 100, 520, 300),
                score: 0.55,
                edge_completeness: 0.8,
                frame_index: 0,
            },
            ScoredRect {
                rect: Rect::new(120, 210, 480, 130),
                score: 0.80,
                edge_completeness: 0.2,
                frame_index: 0,
            },
            ScoredRect {
                rect: Rect::new(170, 150, 360, 100),
                score: 0.30,
                edge_completeness: 0.4,
                frame_index: 0,
            },
        ];

        let ranked = rank_candidates(&candidates, 720, 1280, 1);

        assert_eq!(ranked[0].rect, candidates[0].rect);
    }
}
