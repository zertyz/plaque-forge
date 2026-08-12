//! Plaque motion tracking across the source video.
//!
//! A tracker estimates a projective transform for each frame, then stabilizes that
//! trajectory and applies any locked or guiding motion refinements.

use std::path::Path;

use anyhow::{Context, Result, bail};
use opencv::{
    core::{self, DMatch, KeyPoint, Mat, Point, Point2f, Rect, Scalar, Vector},
    features2d, geometry, imgcodecs, imgproc,
    prelude::*,
    videoio::{CAP_ANY, CAP_PROP_FRAME_COUNT, CAP_PROP_POS_FRAMES, VideoCapture},
};

use crate::{
    cli::AnalyzeArgs,
    geometry::{Point as GeoPoint, Quad as GeoQuad, homography},
    model::{Mat3, MotionSample, PointF, RectF},
    progress::ProgressReporter,
    refinement::MotionRefinement,
    video::VideoInfo,
};

#[derive(Debug, Clone, Copy)]
enum MotionModel {
    Adaptive,
    Similarity,
    Affine,
    Projective,
}

pub fn apply_motion_refinement(
    result: &mut TrackingResult,
    track: &MotionRefinement,
    plaque: RectF,
) -> Result<()> {
    apply_refinement_constraints(&mut result.samples, track, plaque, ConstraintSelection::All)?;

    let dense = track.is_dense_locked(result.samples.len());
    let locked = track.locked_keyframes();
    let guides = track.guide_keyframes();
    let old_model = std::mem::take(&mut result.model_name);
    result.model_name = if dense {
        result.confidence = 0.99;
        for sample in &mut result.samples {
            sample.inlier_ratio = 1.0;
            sample.reprojection_error = 0.0;
            sample.ecc = Some(1.0);
        }
        format!(
            "authoritative-refined-quad-track-{}-frames",
            track.keyframes.len()
        )
    } else if locked > 0 && guides > 0 {
        format!("refined-mixed-quad-track-{locked}-locked-{guides}-guided+{old_model}")
    } else if locked > 0 {
        format!(
            "refined-constrained-quad-track-{}-keyframes+{old_model}",
            locked
        )
    } else {
        format!(
            "refined-guided-quad-track-{}-keyframes+{old_model}",
            track.keyframes.len()
        )
    };

    result.loop_closed = refined_loop_closed(&result.samples, plaque);
    Ok(())
}

pub fn reapply_locked_refinements(
    samples: &mut [MotionSample],
    track: &MotionRefinement,
    plaque: RectF,
) -> Result<()> {
    if track.locked_keyframes() == 0 {
        return Ok(());
    }
    apply_refinement_constraints(samples, track, plaque, ConstraintSelection::Locked)
}

#[derive(Clone, Copy)]
enum ConstraintSelection {
    All,
    Locked,
}

fn apply_refinement_constraints(
    samples: &mut [MotionSample],
    track: &MotionRefinement,
    plaque: RectF,
    selection: ConstraintSelection,
) -> Result<()> {
    if samples.is_empty() {
        bail!("automatic track contains no frames");
    }
    let source = GeoQuad::from_rect(plaque.x, plaque.y, plaque.width, plaque.height);
    let automatic = samples
        .iter()
        .map(|sample| transformed_quad(source, sample.transform))
        .collect::<Vec<_>>();
    let mut corrections = Vec::with_capacity(track.keyframes.len());
    for keyframe in track
        .sorted_keyframes()
        .into_iter()
        .filter(|keyframe| matches!(selection, ConstraintSelection::All) || keyframe.locked)
    {
        let current = automatic
            .get(keyframe.frame)
            .with_context(|| format!("missing automatic frame {}", keyframe.frame))?;
        let desired = refinement_quad(keyframe.quad);
        desired.validate(&format!("motion refinement keyframe {}", keyframe.frame))?;
        corrections.push((keyframe.frame, quad_difference(desired, *current)));
    }
    if corrections.is_empty() {
        return Ok(());
    }

    for (frame, sample) in samples.iter_mut().enumerate() {
        let correction = correction_at(&corrections, frame);
        let corrected = quad_sum(automatic[frame], correction);
        corrected.validate(&format!("refined-constrained frame {frame}"))?;
        sample.transform = Mat3 {
            values: homography(source, corrected)?.m,
        };
    }

    Ok(())
}

pub fn apply_visibility_refinements(
    samples: &mut [MotionSample],
    track: &MotionRefinement,
) -> Result<()> {
    if samples.is_empty() {
        bail!("automatic track contains no frames");
    }
    let automatic = samples
        .iter()
        .map(|sample| sample.plaque_visibility)
        .collect::<Vec<_>>();
    let mut corrections = Vec::with_capacity(track.keyframes.len());
    let mut has_authored_visibility = false;
    for keyframe in track.sorted_keyframes() {
        let current = automatic
            .get(keyframe.frame)
            .with_context(|| format!("missing automatic frame {}", keyframe.frame))?;
        let correction = keyframe
            .visibility
            .map(|visibility| {
                has_authored_visibility = true;
                visibility - current
            })
            .unwrap_or(0.0);
        corrections.push((keyframe.frame, correction));
    }
    if !has_authored_visibility {
        return Ok(());
    }

    for (frame, sample) in samples.iter_mut().enumerate() {
        sample.plaque_visibility =
            (automatic[frame] + scalar_correction_at(&corrections, frame)).clamp(0.0, 1.0);
    }
    Ok(())
}

fn transformed_quad(source: GeoQuad, transform: Mat3) -> GeoQuad {
    let transform_point = |point: GeoPoint| {
        let point = transform.transform(PointF {
            x: point.x,
            y: point.y,
        });
        GeoPoint::new(point.x, point.y)
    };
    GeoQuad::new(
        transform_point(source.tl),
        transform_point(source.tr),
        transform_point(source.br),
        transform_point(source.bl),
    )
}

fn refinement_quad(points: [[f64; 2]; 4]) -> GeoQuad {
    GeoQuad::new(
        GeoPoint::new(points[0][0], points[0][1]),
        GeoPoint::new(points[1][0], points[1][1]),
        GeoPoint::new(points[2][0], points[2][1]),
        GeoPoint::new(points[3][0], points[3][1]),
    )
}

fn quad_difference(a: GeoQuad, b: GeoQuad) -> [[f64; 2]; 4] {
    let a = a.points();
    let b = b.points();
    std::array::from_fn(|index| [a[index].x - b[index].x, a[index].y - b[index].y])
}

fn quad_sum(quad: GeoQuad, correction: [[f64; 2]; 4]) -> GeoQuad {
    let points = quad.points();
    GeoQuad::new(
        GeoPoint::new(
            points[0].x + correction[0][0],
            points[0].y + correction[0][1],
        ),
        GeoPoint::new(
            points[1].x + correction[1][0],
            points[1].y + correction[1][1],
        ),
        GeoPoint::new(
            points[2].x + correction[2][0],
            points[2].y + correction[2][1],
        ),
        GeoPoint::new(
            points[3].x + correction[3][0],
            points[3].y + correction[3][1],
        ),
    )
}

fn correction_at(corrections: &[(usize, [[f64; 2]; 4])], frame: usize) -> [[f64; 2]; 4] {
    if frame <= corrections[0].0 {
        return corrections[0].1;
    }
    let last = corrections[corrections.len() - 1];
    if frame >= last.0 {
        return last.1;
    }
    let upper = corrections.partition_point(|correction| correction.0 <= frame);
    let a = corrections[upper - 1];
    let b = corrections[upper];
    let t = (frame - a.0) as f64 / (b.0 - a.0) as f64;
    std::array::from_fn(|corner| {
        [
            a.1[corner][0] + (b.1[corner][0] - a.1[corner][0]) * t,
            a.1[corner][1] + (b.1[corner][1] - a.1[corner][1]) * t,
        ]
    })
}

fn scalar_correction_at(corrections: &[(usize, f64)], frame: usize) -> f64 {
    if frame <= corrections[0].0 {
        return corrections[0].1;
    }
    let last = corrections[corrections.len() - 1];
    if frame >= last.0 {
        return last.1;
    }
    let upper = corrections.partition_point(|correction| correction.0 <= frame);
    let a = corrections[upper - 1];
    let b = corrections[upper];
    let t = (frame - a.0) as f64 / (b.0 - a.0) as f64;
    a.1 + (b.1 - a.1) * t
}

fn refined_loop_closed(samples: &[MotionSample], plaque: RectF) -> bool {
    let Some((first, remainder)) = samples.split_first() else {
        return false;
    };
    let last = remainder.last().unwrap_or(first);
    let source = GeoQuad::from_rect(plaque.x, plaque.y, plaque.width, plaque.height);
    let first = transformed_quad(source, first.transform);
    let last = transformed_quad(source, last.transform);
    first
        .points()
        .into_iter()
        .zip(last.points())
        .map(|(a, b)| (a.x - b.x).hypot(a.y - b.y))
        .sum::<f64>()
        / 4.0
        < 2.0
}

pub struct TrackingResult {
    pub samples: Vec<MotionSample>,
    pub model_name: String,
    pub reference_frame: usize,
    pub confidence: f64,
    pub loop_closed: bool,
}

pub fn select_masked_refinement(
    mut baseline: TrackingResult,
    refinement: TrackingResult,
) -> TrackingResult {
    if refinement.confidence >= baseline.confidence {
        refinement
    } else {
        baseline.model_name.push_str("-masked-retrack-rejected");
        baseline
    }
}

pub fn load_dense_refinement(
    args: &AnalyzeArgs,
    info: &VideoInfo,
    plaque: RectF,
    track: &MotionRefinement,
    diagnostics: &Path,
    progress: &mut ProgressReporter,
) -> Result<TrackingResult> {
    if !track.is_dense_locked(info.frames) {
        bail!("authoritative motion requires one locked quad per source frame");
    }
    let source = GeoQuad::from_rect(plaque.x, plaque.y, plaque.width, plaque.height);
    let mut frames = vec![None; info.frames];
    for keyframe in &track.keyframes {
        frames[keyframe.frame] = Some(keyframe);
    }
    let samples = frames
        .into_iter()
        .enumerate()
        .map(|(frame, keyframe)| {
            let keyframe = keyframe.with_context(|| format!("missing refined frame {frame}"))?;
            let matrix = homography(source, refinement_quad(keyframe.quad))?;
            Ok(MotionSample {
                frame,
                transform: Mat3 { values: matrix.m },
                inlier_ratio: 1.0,
                reprojection_error: 0.0,
                ecc: Some(1.0),
                plaque_visibility: keyframe.visibility.unwrap_or(1.0),
                occluder_coverage: 0.0,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let mut capture = VideoCapture::from_file(&args.input.to_string_lossy(), CAP_ANY)?;
    if !capture.is_opened()? {
        bail!("failed to open {}", args.input.display());
    }
    progress.start(3, 7, "Load refined plaque track", Some(info.frames));
    write_tracking_diagnostics(&mut capture, &samples, plaque, diagnostics, info.frames)?;
    progress.update(
        info.frames,
        format!("{} locked frames", track.keyframes.len()),
    );
    progress.finish("authoritative all-frame quadrilateral track");

    let loop_closed = refined_loop_closed(&samples, plaque);
    Ok(TrackingResult {
        samples,
        model_name: format!(
            "authoritative-refined-quad-track-{}-frames",
            track.keyframes.len()
        ),
        reference_frame: args.plaque_frame.unwrap_or(0),
        confidence: 0.99,
        loop_closed,
    })
}

struct FeatureAnchor {
    frame: usize,
    gray: Mat,
    keypoints: Vector<KeyPoint>,
    descriptors: Mat,
    transform: Mat3,
}

pub fn track(
    args: &AnalyzeArgs,
    info: &VideoInfo,
    plaque: RectF,
    reference_frame: usize,
    diagnostics: &Path,
    progress: &mut ProgressReporter,
) -> Result<TrackingResult> {
    track_with_exclusions(
        args,
        info,
        plaque,
        reference_frame,
        diagnostics,
        progress,
        None,
    )
}

pub fn retrack_masked(
    args: &AnalyzeArgs,
    info: &VideoInfo,
    plaque: RectF,
    reference_frame: usize,
    diagnostics: &Path,
    progress: &mut ProgressReporter,
    occluder_masks: &Path,
) -> Result<TrackingResult> {
    let mut result = track_with_exclusions(
        args,
        info,
        plaque,
        reference_frame,
        diagnostics,
        progress,
        Some(occluder_masks),
    )?;
    result.model_name.push_str("-masked-occluders");
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn track_with_exclusions(
    args: &AnalyzeArgs,
    info: &VideoInfo,
    plaque: RectF,
    reference_frame: usize,
    diagnostics: &Path,
    progress: &mut ProgressReporter,
    occluder_masks: Option<&Path>,
) -> Result<TrackingResult> {
    if !(0.0..0.98).contains(&args.tracking_inertia) {
        bail!("tracking inertia must be in [0, 0.98)");
    }
    if args.anchor_interval == 0 {
        bail!("tracking anchor interval must be at least 1");
    }

    let mut capture = VideoCapture::from_file(&args.input.to_string_lossy(), CAP_ANY)
        .with_context(|| format!("failed to open input video {}", args.input.display()))?;
    if !capture.is_opened()? {
        bail!("input video could not be opened: {}", args.input.display());
    }
    let actual_frames = capture.get(CAP_PROP_FRAME_COUNT)?.round().max(1.0) as usize;
    let frame_count = info.frames.min(actual_frames).max(1);
    let reference_frame = reference_frame.min(frame_count.saturating_sub(1));

    let mut sift = features2d::SIFT::create(1_800, 3, 0.025, 12.0, 1.6, true)?;
    let reference_gray = read_gray(&mut capture, reference_frame)?;
    let reference_exclusion = load_exclusion(
        occluder_masks,
        reference_frame,
        reference_gray.cols(),
        reference_gray.rows(),
    )?;
    let root = make_anchor(
        &mut sift,
        reference_frame,
        reference_gray,
        plaque,
        Mat3::IDENTITY,
        reference_exclusion.as_ref(),
    )?;
    let root_contour = detect_plaque_contour(&root.gray, plaque, Mat3::IDENTITY)?;
    if root.descriptors.empty() || root.keypoints.len() < 20 {
        bail!(
            "insufficient stable plaque-border features at frame {reference_frame}; \
             inspect {} and correct the plaque bounds in the refinement if needed",
            diagnostics.join("candidate.png").display()
        );
    }

    let loop_closed = should_close_loop(&mut capture, frame_count)?;
    if let Some(confidence) = detect_static_plaque(
        &mut capture,
        &mut sift,
        &root,
        plaque,
        frame_count,
        occluder_masks,
    )? {
        progress.start(3, 7, "Static plaque validation", Some(frame_count));
        let samples = (0..frame_count)
            .map(|frame| MotionSample {
                frame,
                transform: Mat3::IDENTITY,
                inlier_ratio: confidence,
                reprojection_error: 0.0,
                ecc: Some(1.0),
                plaque_visibility: 1.0,
                occluder_coverage: 0.0,
            })
            .collect::<Vec<_>>();
        progress.update(frame_count, "identity model");
        progress.finish("static plaque confirmed");
        write_tracking_diagnostics(&mut capture, &samples, plaque, diagnostics, frame_count)?;
        return Ok(TrackingResult {
            samples,
            model_name: "static-plaque-identity".into(),
            reference_frame,
            confidence,
            loop_closed,
        });
    }
    let mut raw: Vec<Option<MotionSample>> = vec![None; frame_count];
    raw[reference_frame] = Some(MotionSample {
        frame: reference_frame,
        transform: Mat3::IDENTITY,
        inlier_ratio: 1.0,
        reprojection_error: 0.0,
        ecc: Some(1.0),
        plaque_visibility: 1.0,
        occluder_coverage: 0.0,
    });

    progress.start(3, 7, "Adaptive scene tracking", Some(frame_count));
    let mut completed = 1usize;
    let mut adaptive_anchor_count = 1usize;
    if reference_frame + 1 < frame_count {
        let forward: Vec<usize> = ((reference_frame + 1)..frame_count).collect();
        adaptive_anchor_count += process_direction(
            args,
            &mut capture,
            &mut sift,
            &root,
            root_contour.as_ref(),
            plaque,
            &forward,
            &mut raw,
            &mut completed,
            progress,
            occluder_masks,
        )?;
    }
    if reference_frame > 0 {
        let backward: Vec<usize> = (0..reference_frame).rev().collect();
        adaptive_anchor_count += process_direction(
            args,
            &mut capture,
            &mut sift,
            &root,
            root_contour.as_ref(),
            plaque,
            &backward,
            &mut raw,
            &mut completed,
            progress,
            occluder_masks,
        )?;
    }

    let mut samples: Vec<MotionSample> = raw
        .into_iter()
        .enumerate()
        .map(|(frame, sample)| {
            sample.unwrap_or(MotionSample {
                frame,
                transform: Mat3::IDENTITY,
                inlier_ratio: 0.0,
                reprojection_error: f64::INFINITY,
                ecc: None,
                plaque_visibility: 1.0,
                occluder_coverage: 0.0,
            })
        })
        .collect();
    let repaired_frames = repair_outliers(&mut samples, plaque);
    optimize_trajectory(
        &mut samples,
        plaque,
        reference_frame,
        args.tracking_inertia,
        loop_closed,
    )?;
    progress.finish(format!(
        "{adaptive_anchor_count} adaptive anchors, {repaired_frames} repaired frames"
    ));

    write_tracking_diagnostics(&mut capture, &samples, plaque, diagnostics, frame_count)?;

    let median_inliers = median(samples.iter().map(|sample| sample.inlier_ratio).collect());
    let median_error = median(
        samples
            .iter()
            .map(|sample| sample.reprojection_error)
            .filter(|value| value.is_finite())
            .collect(),
    );
    let repair_penalty =
        (repaired_frames as f64 / frame_count.max(1) as f64 * 2.0).clamp(0.0, 0.35);
    let confidence =
        (median_inliers * (-median_error / 4.0).exp() - repair_penalty).clamp(0.0, 0.99);

    Ok(TrackingResult {
        samples,
        model_name: format!(
            "adaptive-anchors-sift-{:?}-inertia-{:.2}",
            MotionModel::Adaptive,
            args.tracking_inertia
        )
        .to_lowercase(),
        reference_frame,
        confidence,
        loop_closed,
    })
}

#[allow(clippy::too_many_arguments)]
fn process_direction(
    args: &AnalyzeArgs,
    capture: &mut VideoCapture,
    sift: &mut core::Ptr<features2d::SIFT>,
    root: &FeatureAnchor,
    root_contour: Option<&PlaqueContour>,
    plaque: RectF,
    indices: &[usize],
    output: &mut [Option<MotionSample>],
    completed: &mut usize,
    progress: &mut ProgressReporter,
    occluder_masks: Option<&Path>,
) -> Result<usize> {
    let mut anchor = clone_anchor(root)?;
    let mut last_transform = root.transform;
    let mut before_last_transform: Option<Mat3> = None;
    let sequential_forward = indices
        .windows(2)
        .all(|pair| pair[1] == pair[0].saturating_add(1));
    if sequential_forward && let Some(&first) = indices.first() {
        capture.set(CAP_PROP_POS_FRAMES, first as f64)?;
    }
    let mut anchors_added = 0usize;
    for &frame_index in indices {
        let gray = if sequential_forward {
            read_next_gray(capture, frame_index)
        } else {
            read_gray(capture, frame_index)
        }
        .with_context(|| format!("failed to decode tracking frame {frame_index}"))?;
        let extrapolated = before_last_transform
            .map(|previous| extrapolate_matrix(previous, last_transform))
            .unwrap_or(last_transform);
        let predicted = if plaque_transform_is_valid(extrapolated, plaque) {
            extrapolated
        } else {
            last_transform
        };
        let mut current_mask =
            plaque_feature_mask_for_transform(gray.cols(), gray.rows(), plaque, predicted)?;
        let exclusion = load_exclusion(occluder_masks, frame_index, gray.cols(), gray.rows())?;
        let excluded = apply_exclusion(&mut current_mask, exclusion.as_ref())?;
        let contour = (excluded < 0.01)
            .then(|| {
                root_contour
                    .zip(
                        detect_plaque_contour(&gray, plaque, predicted)
                            .ok()
                            .flatten(),
                    )
                    .and_then(|(reference, current)| {
                        homography(reference.quad, current.quad)
                            .ok()
                            .map(|matrix| Estimate {
                                matrix: Mat3 { values: matrix.m },
                                inlier_ratio: current.confidence,
                                error: (1.0 - current.confidence) * 2.0,
                                ecc: None,
                                source: "geometry",
                                static_model: false,
                            })
                    })
                    .filter(|estimate| plaque_transform_is_valid(estimate.matrix, plaque))
            })
            .flatten();
        let mut keypoints = Vector::<KeyPoint>::new();
        let mut descriptors = Mat::default();
        sift.detect_and_compute(
            &gray,
            &current_mask,
            &mut keypoints,
            &mut descriptors,
            false,
        )?;

        let local = estimate_reference_transform(
            &anchor.keypoints,
            &anchor.descriptors,
            &keypoints,
            &descriptors,
            MotionModel::Adaptive,
            false,
        )
        .map(|estimate| estimate.anchored(anchor.transform, "adaptive"));

        // The fixed root estimate prevents cumulative drift. The adaptive
        // reference remains available when appearance changes make the root weak.
        let direct = estimate_reference_transform(
            &root.keypoints,
            &root.descriptors,
            &keypoints,
            &descriptors,
            MotionModel::Adaptive,
            false,
        )
        .ok()
        .map(|estimate| {
            let source = if estimate.static_model {
                "root-static"
            } else {
                "root"
            };
            estimate.named(source)
        });

        let feature = choose_estimate(local, direct, plaque, predicted);
        let estimate = choose_geometric_constraint(feature, contour, plaque).unwrap_or(Estimate {
            matrix: predicted,
            inlier_ratio: 0.0,
            error: 24.0,
            ecc: None,
            source: "inertial-fallback",
            static_model: false,
        });
        output[frame_index] = Some(MotionSample {
            frame: frame_index,
            transform: estimate.matrix,
            inlier_ratio: estimate.inlier_ratio,
            reprojection_error: estimate.error,
            ecc: estimate.ecc,
            plaque_visibility: 1.0,
            occluder_coverage: 0.0,
        });
        before_last_transform = Some(last_transform);
        last_transform = estimate.matrix;

        let trustworthy = estimate.inlier_ratio >= 0.22 && estimate.error <= 5.0;
        let anchor_motion = mean_corner_distance(anchor.transform, estimate.matrix, plaque);
        let motion_refresh = plaque
            .width
            .hypot(plaque.height)
            .mul_add(0.035, 0.0)
            .clamp(12.0, 32.0);
        let due = frame_index.abs_diff(anchor.frame) >= args.anchor_interval
            || anchor_motion >= motion_refresh;
        if trustworthy && due && excluded < 0.01 {
            anchor = make_anchor(
                sift,
                frame_index,
                gray,
                plaque,
                estimate.matrix,
                exclusion.as_ref(),
            )?;
            anchors_added += 1;
        }

        *completed += 1;
        progress.update(
            *completed,
            format!(
                "frame {frame_index}, {}, inliers {:.2}, error {:.2}px",
                estimate.source, estimate.inlier_ratio, estimate.error
            ),
        );
    }
    Ok(anchors_added)
}

fn detect_static_plaque(
    capture: &mut VideoCapture,
    sift: &mut core::Ptr<features2d::SIFT>,
    root: &FeatureAnchor,
    plaque: RectF,
    frame_count: usize,
    occluder_masks: Option<&Path>,
) -> Result<Option<f64>> {
    let mut displacements = Vec::new();
    let mut qualities = Vec::new();
    for slot in 0..11 {
        let frame = slot * frame_count.saturating_sub(1) / 10;
        if frame == root.frame {
            continue;
        }
        let gray = read_gray(capture, frame)?;
        let mut mask =
            plaque_feature_mask_for_transform(gray.cols(), gray.rows(), plaque, Mat3::IDENTITY)?;
        let exclusion = load_exclusion(occluder_masks, frame, gray.cols(), gray.rows())?;
        apply_exclusion(&mut mask, exclusion.as_ref())?;
        let mut keypoints = Vector::<KeyPoint>::new();
        let mut descriptors = Mat::default();
        sift.detect_and_compute(&gray, &mask, &mut keypoints, &mut descriptors, false)?;
        let Ok(estimate) = estimate_reference_transform(
            &root.keypoints,
            &root.descriptors,
            &keypoints,
            &descriptors,
            MotionModel::Adaptive,
            true,
        ) else {
            continue;
        };
        if estimate.inlier_ratio < 0.45 || estimate.error > 2.0 {
            continue;
        }
        displacements.push(mean_corner_distance(
            Mat3::IDENTITY,
            estimate.matrix,
            plaque,
        ));
        qualities.push(estimate.inlier_ratio * (-estimate.error / 2.0).exp());
    }
    if displacements.len() < 6 {
        return Ok(None);
    }
    displacements.sort_by(f64::total_cmp);
    let stationary = displacements
        .iter()
        .filter(|&&displacement| displacement <= 1.50)
        .count();
    let robust_limit = displacements[displacements.len() * 4 / 5];
    if stationary * 4 < displacements.len() * 3 || robust_limit > 2.50 {
        return Ok(None);
    }
    let support = stationary as f64 / displacements.len() as f64;
    Ok(Some((median(qualities) * support).clamp(0.80, 0.99)))
}

struct Estimate {
    matrix: Mat3,
    inlier_ratio: f64,
    error: f64,
    ecc: Option<f64>,
    source: &'static str,
    static_model: bool,
}

impl Estimate {
    fn anchored(mut self, anchor: Mat3, source: &'static str) -> Self {
        self.matrix = self.matrix.multiply(anchor);
        self.source = source;
        self
    }

    fn named(mut self, source: &'static str) -> Self {
        self.source = source;
        self
    }
}

fn choose_estimate(
    local: Result<Estimate>,
    direct: Option<Estimate>,
    plaque: RectF,
    predicted: Mat3,
) -> Option<Estimate> {
    let local = local
        .ok()
        .filter(|estimate| plaque_transform_is_valid(estimate.matrix, plaque));
    let direct = direct.filter(|estimate| plaque_transform_is_valid(estimate.matrix, plaque));
    match (local, direct) {
        (Some(local), Some(direct)) => {
            let local_score = local.error
                + (1.0 - local.inlier_ratio) * 3.0
                + continuity_penalty(local.matrix, predicted, plaque);
            let direct_score = direct.error
                + (1.0 - direct.inlier_ratio) * 3.0
                + continuity_penalty(direct.matrix, predicted, plaque);
            let direct_is_credible = direct.inlier_ratio >= 0.20 && direct.error <= 5.0;
            if direct_is_credible && direct_score <= local_score + 0.75 {
                Some(direct)
            } else {
                Some(local)
            }
        }
        (Some(local), None) => Some(local),
        (None, Some(direct)) => Some(direct),
        (None, None) => None,
    }
}

fn continuity_penalty(candidate: Mat3, predicted: Mat3, plaque: RectF) -> f64 {
    let mean = mean_corner_distance(candidate, predicted, plaque);
    (mean - 6.0).max(0.0) * 0.20
}

fn mean_corner_distance(left: Mat3, right: Mat3, plaque: RectF) -> f64 {
    plaque_corners(plaque)
        .into_iter()
        .map(|point| {
            let a = left.transform(point);
            let b = right.transform(point);
            (a.x - b.x).hypot(a.y - b.y)
        })
        .sum::<f64>()
        / 4.0
}

fn choose_geometric_constraint(
    feature: Option<Estimate>,
    geometry: Option<Estimate>,
    plaque: RectF,
) -> Option<Estimate> {
    match (feature, geometry) {
        (Some(feature), Some(geometry)) => {
            let disagreement = mean_corner_distance(feature.matrix, geometry.matrix, plaque);
            let tolerance = plaque
                .width
                .hypot(plaque.height)
                .mul_add(0.02, 0.0)
                .clamp(8.0, 18.0);
            let feature_score = feature.error + (1.0 - feature.inlier_ratio) * 3.0;
            let geometry_score = geometry.error + (1.0 - geometry.inlier_ratio) * 3.0;
            if disagreement > tolerance && geometry_score + 0.25 < feature_score {
                Some(geometry)
            } else {
                Some(feature)
            }
        }
        (Some(feature), None) => Some(feature),
        (None, Some(geometry)) => Some(geometry),
        (None, None) => None,
    }
}

fn transformed_plaque(plaque: RectF, transform: Mat3) -> GeoQuad {
    let points = plaque_corners(plaque).map(|point| {
        let mapped = transform.transform(point);
        GeoPoint::new(mapped.x, mapped.y)
    });
    GeoQuad::new(points[0], points[1], points[2], points[3])
}

#[derive(Clone, Copy)]
struct PlaqueContour {
    quad: GeoQuad,
    confidence: f64,
}

fn detect_plaque_contour(
    gray: &Mat,
    plaque: RectF,
    predicted: Mat3,
) -> Result<Option<PlaqueContour>> {
    let mut blurred = Mat::default();
    imgproc::gaussian_blur(
        gray,
        &mut blurred,
        core::Size::new(5, 5),
        1.2,
        1.2,
        core::BORDER_DEFAULT,
        core::AlgorithmHint::ALGO_HINT_DEFAULT,
    )?;
    let mut edges = Mat::default();
    imgproc::canny(&blurred, &mut edges, 60.0, 160.0, 3, false)?;
    let kernel = imgproc::get_structuring_element(
        imgproc::MORPH_RECT,
        core::Size::new(9, 5),
        Point::new(-1, -1),
    )?;
    let mut closed = Mat::default();
    imgproc::morphology_ex(
        &edges,
        &mut closed,
        imgproc::MORPH_CLOSE,
        &kernel,
        Point::new(-1, -1),
        2,
        core::BORDER_CONSTANT,
        imgproc::morphology_default_border_value()?,
    )?;
    let mut contours = Vector::<Vector<Point>>::new();
    imgproc::find_contours(
        &closed,
        &mut contours,
        imgproc::RETR_LIST,
        imgproc::CHAIN_APPROX_SIMPLE,
        Point::new(0, 0),
    )?;

    let expected = transformed_plaque(plaque, predicted);
    let expected_points = expected.points();
    let expected_center = GeoPoint::new(
        expected_points.iter().map(|point| point.x).sum::<f64>() / 4.0,
        expected_points.iter().map(|point| point.y).sum::<f64>() / 4.0,
    );
    let expected_area = expected.orientation().abs().max(1.0);
    let expected_width = ((expected.tr.x - expected.tl.x).hypot(expected.tr.y - expected.tl.y)
        + (expected.br.x - expected.bl.x).hypot(expected.br.y - expected.bl.y))
        * 0.5;
    let expected_height = ((expected.bl.x - expected.tl.x).hypot(expected.bl.y - expected.tl.y)
        + (expected.br.x - expected.tr.x).hypot(expected.br.y - expected.tr.y))
        * 0.5;
    let expected_aspect =
        expected_width.max(expected_height) / expected_width.min(expected_height).max(1.0);
    let maximum_center_error = expected_width.hypot(expected_height) * 0.30 + 20.0;

    let mut best: Option<(f64, PlaqueContour)> = None;
    for contour in contours {
        let rotated = geometry::min_area_rect(&contour)?;
        let long = f64::from(rotated.size.width.max(rotated.size.height));
        let short = f64::from(rotated.size.width.min(rotated.size.height));
        let area = long * short;
        if short < 20.0 || long < 80.0 {
            continue;
        }
        let area_ratio = area / expected_area;
        let aspect = long / short;
        if !(0.45..=1.50).contains(&area_ratio)
            || !((expected_aspect * 0.65)..=(expected_aspect * 1.45)).contains(&aspect)
        {
            continue;
        }
        let center_error = (f64::from(rotated.center.x) - expected_center.x)
            .hypot(f64::from(rotated.center.y) - expected_center.y);
        if center_error > maximum_center_error {
            continue;
        }
        let rectangularity =
            (geometry::contour_area(&contour, false)?.abs() / area).clamp(0.0, 1.0);
        if rectangularity < 0.55 {
            continue;
        }
        let quad = oriented_rect_quad(
            f64::from(rotated.center.x),
            f64::from(rotated.center.y),
            f64::from(rotated.size.width),
            f64::from(rotated.size.height),
            f64::from(rotated.angle),
        );
        let area_fit = (1.0 - area_ratio.ln().abs()).clamp(0.0, 1.0);
        let aspect_fit = (1.0 - (aspect / expected_aspect).ln().abs()).clamp(0.0, 1.0);
        let center_fit = (1.0 - center_error / maximum_center_error).clamp(0.0, 1.0);
        let confidence =
            (0.45 * rectangularity + 0.25 * area_fit + 0.15 * aspect_fit + 0.15 * center_fit)
                .clamp(0.0, 0.98);
        let objective = confidence + 0.15 * rectangularity;
        if best.as_ref().is_none_or(|(score, _)| objective > *score) {
            best = Some((objective, PlaqueContour { quad, confidence }));
        }
    }
    Ok(best.map(|(_, contour)| contour))
}

fn oriented_rect_quad(
    center_x: f64,
    center_y: f64,
    width: f64,
    height: f64,
    angle_degrees: f64,
) -> GeoQuad {
    let (long, short, angle) = if width >= height {
        (width, height, angle_degrees)
    } else {
        (height, width, angle_degrees + 90.0)
    };
    let angle = angle.to_radians();
    let along = (angle.cos() * long * 0.5, angle.sin() * long * 0.5);
    let across = (-angle.sin() * short * 0.5, angle.cos() * short * 0.5);
    GeoQuad::new(
        GeoPoint::new(center_x - along.0 - across.0, center_y - along.1 - across.1),
        GeoPoint::new(center_x + along.0 - across.0, center_y + along.1 - across.1),
        GeoPoint::new(center_x + along.0 + across.0, center_y + along.1 + across.1),
        GeoPoint::new(center_x - along.0 + across.0, center_y - along.1 + across.1),
    )
}

fn plaque_transform_is_valid(transform: Mat3, plaque: RectF) -> bool {
    let quad = transformed_plaque(plaque, transform);
    if quad.validate("tracked plaque").is_err() || quad.orientation() <= 0.0 {
        return false;
    }
    let source_area = plaque.width * plaque.height;
    let area_ratio = quad.orientation() / source_area.max(1.0);
    (0.15..=6.0).contains(&area_ratio)
}

fn extrapolate_matrix(previous: Mat3, current: Mat3) -> Mat3 {
    let mut values = current.values;
    for (row, cells) in values.iter_mut().enumerate() {
        for (column, cell) in cells.iter_mut().enumerate() {
            *cell = current.values[row][column]
                + (current.values[row][column] - previous.values[row][column]);
        }
    }
    values[2][2] = 1.0;
    Mat3 { values }
}

fn make_anchor(
    sift: &mut core::Ptr<features2d::SIFT>,
    frame: usize,
    gray: Mat,
    plaque: RectF,
    transform: Mat3,
    exclusion: Option<&Mat>,
) -> Result<FeatureAnchor> {
    let mut mask = plaque_feature_mask_for_transform(gray.cols(), gray.rows(), plaque, transform)?;
    apply_exclusion(&mut mask, exclusion)?;
    let mut keypoints = Vector::<KeyPoint>::new();
    let mut descriptors = Mat::default();
    sift.detect_and_compute(&gray, &mask, &mut keypoints, &mut descriptors, false)?;
    Ok(FeatureAnchor {
        frame,
        gray,
        keypoints,
        descriptors,
        transform,
    })
}

fn load_exclusion(
    root: Option<&Path>,
    frame: usize,
    width: i32,
    height: i32,
) -> Result<Option<Mat>> {
    let Some(root) = root else {
        return Ok(None);
    };
    let path = root.join(format!("{frame:06}.png"));
    if !path.is_file() {
        return Ok(None);
    }
    let mask = imgcodecs::imread(&path.to_string_lossy(), imgcodecs::IMREAD_GRAYSCALE)
        .with_context(|| format!("failed to read occluder mask {}", path.display()))?;
    if mask.cols() != width || mask.rows() != height {
        bail!("occluder mask dimensions differ from tracking frame");
    }
    let mut binary = Mat::default();
    imgproc::threshold(&mask, &mut binary, 8.0, 255.0, imgproc::THRESH_BINARY)?;
    let mut expanded = Mat::default();
    imgproc::dilate(
        &binary,
        &mut expanded,
        &Mat::default(),
        Point::new(-1, -1),
        2,
        core::BORDER_CONSTANT,
        imgproc::morphology_default_border_value()?,
    )?;
    Ok(Some(expanded))
}

fn apply_exclusion(mask: &mut Mat, exclusion: Option<&Mat>) -> Result<f64> {
    let Some(exclusion) = exclusion else {
        return Ok(0.0);
    };
    let before = core::count_non_zero(mask)?.max(1) as f64;
    let mut allowed = Mat::default();
    core::bitwise_not(exclusion, &mut allowed, &core::no_array())?;
    let mut filtered = Mat::default();
    core::bitwise_and(mask, &allowed, &mut filtered, &core::no_array())?;
    let after = core::count_non_zero(&filtered)? as f64;
    *mask = filtered;
    Ok((1.0 - after / before).clamp(0.0, 1.0))
}

fn clone_anchor(anchor: &FeatureAnchor) -> Result<FeatureAnchor> {
    Ok(FeatureAnchor {
        frame: anchor.frame,
        gray: anchor.gray.try_clone()?,
        keypoints: anchor.keypoints.clone(),
        descriptors: anchor.descriptors.try_clone()?,
        transform: anchor.transform,
    })
}

fn read_gray(capture: &mut VideoCapture, frame_index: usize) -> Result<Mat> {
    capture.set(CAP_PROP_POS_FRAMES, frame_index as f64)?;
    read_next_gray(capture, frame_index)
}

fn read_next_gray(capture: &mut VideoCapture, frame_index: usize) -> Result<Mat> {
    let mut frame = Mat::default();
    if !capture.read(&mut frame)? || frame.empty() {
        bail!("failed to decode frame {frame_index}");
    }
    grayscale(&frame)
}

fn plaque_feature_mask_for_transform(
    width: i32,
    height: i32,
    plaque: RectF,
    transform: Mat3,
) -> Result<Mat> {
    let mut mask = Mat::new_rows_cols_with_default(height, width, core::CV_8UC1, Scalar::all(0.0))?;
    let margin = 12.0;
    let outer = RectF {
        x: plaque.x - margin,
        y: plaque.y - margin,
        width: plaque.width + margin * 2.0,
        height: plaque.height + margin * 2.0,
    };
    let inner = RectF {
        x: plaque.x + plaque.width * 0.14,
        y: plaque.y + plaque.height * 0.22,
        width: plaque.width * 0.72,
        height: plaque.height * 0.56,
    };
    let polygon = |rect: RectF| {
        plaque_corners(rect)
            .into_iter()
            .map(|point| {
                let mapped = transform.transform(point);
                Point::new(mapped.x.round() as i32, mapped.y.round() as i32)
            })
            .collect::<Vector<Point>>()
    };
    imgproc::fill_convex_poly(
        &mut mask,
        &polygon(outer),
        Scalar::all(255.0),
        imgproc::LINE_8,
        0,
    )?;
    imgproc::fill_convex_poly(
        &mut mask,
        &polygon(inner),
        Scalar::all(0.0),
        imgproc::LINE_8,
        0,
    )?;
    Ok(mask)
}

pub(crate) fn optimize_trajectory(
    samples: &mut [MotionSample],
    plaque: RectF,
    reference_frame: usize,
    inertia: f64,
    loop_closed: bool,
) -> Result<()> {
    if samples.len() < 3 || inertia <= 0.0 {
        return Ok(());
    }
    let source = GeoQuad::from_rect(plaque.x, plaque.y, plaque.width, plaque.height);
    let raw: Vec<GeoQuad> = samples
        .iter()
        .map(|sample| {
            let points = source.points().map(|point| {
                let mapped = sample.transform.transform(PointF {
                    x: point.x,
                    y: point.y,
                });
                GeoPoint::new(mapped.x, mapped.y)
            });
            GeoQuad::new(points[0], points[1], points[2], points[3])
        })
        .collect();
    let mut smooth = raw.clone();
    for _ in 0..6 {
        let previous = smooth.clone();
        for index in 0..smooth.len() {
            if index == reference_frame {
                smooth[index] = raw[index];
                continue;
            }
            let left = if index == 0 {
                if loop_closed { previous.len() - 1 } else { 0 }
            } else {
                index - 1
            };
            let right = if index + 1 == previous.len() {
                if loop_closed { 0 } else { previous.len() - 1 }
            } else {
                index + 1
            };
            let confidence = sample_confidence(&samples[index]);
            let neighbor_weight = (inertia * (1.25 - 0.95 * confidence)).clamp(0.0, 0.48);
            let neighbor = previous[left].lerp(previous[right], 0.5);
            let candidate = raw[index].lerp(neighbor, neighbor_weight.clamp(0.0, 0.48));
            smooth[index] =
                if candidate.validate("smoothed plaque").is_ok() && candidate.orientation() > 0.0 {
                    candidate
                } else {
                    raw[index]
                };
        }
    }
    if smooth.len() >= 13 {
        let coefficients = [
            -11.0, 0.0, 9.0, 16.0, 21.0, 24.0, 25.0, 24.0, 21.0, 16.0, 9.0, 0.0, -11.0,
        ];
        let threshold = plaque
            .width
            .hypot(plaque.height)
            .mul_add(0.006, 0.0)
            .clamp(2.0, 5.0);
        for _ in 0..4 {
            let previous = smooth.clone();
            for index in 0..smooth.len() {
                if index == reference_frame {
                    smooth[index] = raw[index];
                    continue;
                }
                let mut coordinates = [[0.0; 2]; 4];
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
                let filtered = GeoQuad::new(
                    GeoPoint::new(coordinates[0][0], coordinates[0][1]),
                    GeoPoint::new(coordinates[1][0], coordinates[1][1]),
                    GeoPoint::new(coordinates[2][0], coordinates[2][1]),
                    GeoPoint::new(coordinates[3][0], coordinates[3][1]),
                );
                let confidence = sample_confidence(&samples[index]);
                let deviation = mean_quad_distance(previous[index], filtered);
                let weight = if deviation > threshold {
                    0.65 + 0.25 * (1.0 - confidence)
                } else {
                    inertia.clamp(0.0, 0.75)
                };
                let candidate = previous[index].lerp(filtered, weight);
                if candidate.validate("globally regularized plaque").is_ok()
                    && candidate.orientation() > 0.0
                {
                    smooth[index] = candidate;
                }
            }
        }
    }
    for (sample, quad) in samples.iter_mut().zip(smooth) {
        quad.validate("temporally regularized plaque")?;
        let matrix = homography(source, quad)?;
        sample.transform = Mat3 { values: matrix.m };
    }
    Ok(())
}

fn mean_quad_distance(left: GeoQuad, right: GeoQuad) -> f64 {
    left.points()
        .into_iter()
        .zip(right.points())
        .map(|(a, b)| (a.x - b.x).hypot(a.y - b.y))
        .sum::<f64>()
        / 4.0
}

fn sample_confidence(sample: &MotionSample) -> f64 {
    (sample.inlier_ratio * (-sample.reprojection_error.min(20.0) / 5.0).exp()).clamp(0.0, 1.0)
}

fn plaque_corners(plaque: RectF) -> [PointF; 4] {
    [
        PointF {
            x: plaque.x,
            y: plaque.y,
        },
        PointF {
            x: plaque.x + plaque.width,
            y: plaque.y,
        },
        PointF {
            x: plaque.x + plaque.width,
            y: plaque.y + plaque.height,
        },
        PointF {
            x: plaque.x,
            y: plaque.y + plaque.height,
        },
    ]
}

fn write_tracking_diagnostics(
    capture: &mut VideoCapture,
    samples: &[MotionSample],
    plaque: RectF,
    diagnostics: &Path,
    frame_count: usize,
) -> Result<()> {
    let mut frames = Vec::new();
    for slot in 0..12usize {
        let index = if frame_count <= 1 {
            0
        } else {
            slot * (frame_count - 1) / 11
        };
        capture.set(CAP_PROP_POS_FRAMES, index as f64)?;
        let mut frame = Mat::default();
        if capture.read(&mut frame)? && !frame.empty() {
            frames.push(draw_diagnostic(frame, plaque, samples[index].transform)?);
        }
    }
    write_contact_sheet(&frames, &diagnostics.join("tracking-contact-sheet.jpg"))?;
    Ok(())
}

fn estimate_reference_transform(
    reference_keypoints: &Vector<KeyPoint>,
    reference_descriptors: &Mat,
    keypoints: &Vector<KeyPoint>,
    descriptors: &Mat,
    requested_model: MotionModel,
    allow_static: bool,
) -> Result<Estimate> {
    if descriptors.empty() || keypoints.len() < 12 {
        bail!("insufficient frame descriptors");
    }

    let (source, destination, coverage) = mutual_correspondences(
        reference_keypoints,
        reference_descriptors,
        keypoints,
        descriptors,
    )?;

    if source.len() < 8 {
        bail!("insufficient robust feature matches");
    }

    let models = match requested_model {
        MotionModel::Adaptive => vec![
            MotionModel::Similarity,
            MotionModel::Affine,
            MotionModel::Projective,
        ],
        model => vec![model],
    };

    let mut best = if allow_static && matches!(requested_model, MotionModel::Adaptive) {
        static_estimate(&source, &destination, coverage)?
            .map(|estimate| (estimate_objective(&estimate, 0.0), estimate))
    } else {
        None
    };
    for model in models {
        let mut estimate = estimate_model(&source, &destination, model)?;
        estimate.inlier_ratio *= 0.55 + 0.45 * coverage;
        let complexity_penalty = match model {
            MotionModel::Similarity => 0.0,
            MotionModel::Affine => 0.15,
            MotionModel::Projective => 0.35,
            MotionModel::Adaptive => unreachable!(),
        };
        let objective = estimate_objective(&estimate, complexity_penalty);
        let replace = best
            .as_ref()
            .map(|(current_objective, _)| objective < *current_objective)
            .unwrap_or(true);
        if replace {
            best = Some((objective, estimate));
        }
    }

    best.map(|(_, estimate)| estimate)
        .context("no motion model could be estimated")
}

fn estimate_objective(estimate: &Estimate, complexity_penalty: f64) -> f64 {
    estimate.error + complexity_penalty + (1.0 - estimate.inlier_ratio) * 2.0
}

fn mutual_correspondences(
    reference_keypoints: &Vector<KeyPoint>,
    reference_descriptors: &Mat,
    keypoints: &Vector<KeyPoint>,
    descriptors: &Mat,
) -> Result<(Vector<Point2f>, Vector<Point2f>, f64)> {
    let matcher = features2d::BFMatcher::create(core::NORM_L2, false)?;
    let mut forward = Vector::<Vector<DMatch>>::new();
    let mut reverse = Vector::<Vector<DMatch>>::new();
    matcher.knn_train_match_def(reference_descriptors, descriptors, &mut forward, 2)?;
    matcher.knn_train_match_def(descriptors, reference_descriptors, &mut reverse, 2)?;

    let mut reverse_pairs = std::collections::HashSet::new();
    for pair in reverse {
        if pair.len() < 2 {
            continue;
        }
        let first = pair.get(0)?;
        let second = pair.get(1)?;
        if first.distance < 0.78 * second.distance {
            reverse_pairs.insert((first.query_idx, first.train_idx));
        }
    }

    let mut source = Vector::<Point2f>::new();
    let mut destination = Vector::<Point2f>::new();
    for pair in forward {
        if pair.len() < 2 {
            continue;
        }
        let first = pair.get(0)?;
        let second = pair.get(1)?;
        if first.distance >= 0.72 * second.distance
            || !reverse_pairs.contains(&(first.train_idx, first.query_idx))
        {
            continue;
        }
        source.push(reference_keypoints.get(first.query_idx as usize)?.pt());
        destination.push(keypoints.get(first.train_idx as usize)?.pt());
    }
    let coverage = match_coverage(reference_keypoints, &source)?;
    Ok((source, destination, coverage))
}

fn match_coverage(reference: &Vector<KeyPoint>, matched: &Vector<Point2f>) -> Result<f64> {
    if reference.is_empty() || matched.is_empty() {
        return Ok(0.0);
    }
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for index in 0..reference.len() {
        let point = reference.get(index)?.pt();
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }
    let center_x = (min_x + max_x) * 0.5;
    let center_y = (min_y + max_y) * 0.5;
    let mut occupied = [false; 4];
    let mut matched_min_x = f32::INFINITY;
    let mut matched_min_y = f32::INFINITY;
    let mut matched_max_x = f32::NEG_INFINITY;
    let mut matched_max_y = f32::NEG_INFINITY;
    for point in matched {
        let index = usize::from(point.x >= center_x) + 2 * usize::from(point.y >= center_y);
        occupied[index] = true;
        matched_min_x = matched_min_x.min(point.x);
        matched_min_y = matched_min_y.min(point.y);
        matched_max_x = matched_max_x.max(point.x);
        matched_max_y = matched_max_y.max(point.y);
    }
    let quadrants = occupied.into_iter().filter(|value| *value).count() as f64 / 4.0;
    let horizontal = f64::from((matched_max_x - matched_min_x) / (max_x - min_x).max(1.0));
    let vertical = f64::from((matched_max_y - matched_min_y) / (max_y - min_y).max(1.0));
    Ok((0.6 * quadrants + 0.2 * horizontal.min(1.0) + 0.2 * vertical.min(1.0)).clamp(0.0, 1.0))
}

fn static_estimate(
    source: &Vector<Point2f>,
    destination: &Vector<Point2f>,
    coverage: f64,
) -> Result<Option<Estimate>> {
    if coverage < 0.55 {
        return Ok(None);
    }
    let errors = correspondence_errors(source, destination, Mat3::IDENTITY)?;
    let inliers =
        errors.iter().filter(|error| **error <= 1.25).count() as f64 / errors.len().max(1) as f64;
    let error = median(errors);
    Ok((inliers >= 0.70 && error <= 0.75).then_some(Estimate {
        matrix: Mat3::IDENTITY,
        inlier_ratio: inliers * (0.55 + 0.45 * coverage),
        error,
        ecc: None,
        source: "static",
        static_model: true,
    }))
}

fn estimate_model(
    source: &Vector<Point2f>,
    destination: &Vector<Point2f>,
    model: MotionModel,
) -> Result<Estimate> {
    let mut inliers = Mat::default();
    let initial = match model {
        MotionModel::Similarity => geometry::estimate_affine_partial_2d(
            source,
            destination,
            &mut inliers,
            geometry::RANSAC,
            3.0,
            4000,
            0.995,
            20,
        )?,
        MotionModel::Affine => geometry::estimate_affine_2d(
            source,
            destination,
            &mut inliers,
            geometry::RANSAC,
            3.0,
            4000,
            0.995,
            20,
        )?,
        MotionModel::Projective => geometry::find_homography(
            source,
            destination,
            geometry::RANSAC,
            3.0,
            &mut inliers,
            4000,
            0.995,
        )?,
        MotionModel::Adaptive => unreachable!(),
    };
    if initial.empty() {
        bail!("robust transform estimation failed");
    }

    let matrix = mat_to_mat3(&initial)?;
    let error = symmetric_reprojection_error(source, destination, matrix)?;

    let inlier_ratio = if inliers.empty() {
        0.0
    } else {
        core::count_non_zero(&inliers)? as f64 / source.len().max(1) as f64
    };

    Ok(Estimate {
        matrix,
        inlier_ratio,
        error,
        ecc: None,
        source: "feature",
        static_model: false,
    })
}

fn grayscale(frame: &Mat) -> Result<Mat> {
    let mut gray = Mat::default();
    imgproc::cvt_color(
        frame,
        &mut gray,
        imgproc::COLOR_BGR2GRAY,
        0,
        core::AlgorithmHint::ALGO_HINT_DEFAULT,
    )?;
    Ok(gray)
}

#[cfg(test)]
fn plaque_feature_mask(width: i32, height: i32, plaque: RectF) -> Result<Mat> {
    plaque_feature_mask_for_transform(width, height, plaque, Mat3::IDENTITY)
}

/// Interpolates low-confidence temporal impulses without altering valid motion.
pub(crate) fn repair_outliers(samples: &mut [MotionSample], plaque: RectF) -> usize {
    if samples.len() < 5 {
        return 0;
    }

    let source_corners = plaque_corners(plaque);
    let projected: Vec<[PointF; 4]> = samples
        .iter()
        .map(|sample| source_corners.map(|point| sample.transform.transform(point)))
        .collect();

    let threshold = plaque
        .width
        .hypot(plaque.height)
        .mul_add(0.015, 0.0)
        .clamp(5.0, 14.0);
    let mut bad = Vec::new();
    for (index, sample) in samples.iter().enumerate() {
        let start = index.saturating_sub(3);
        let end = (index + 4).min(samples.len());
        if end - start < 4 {
            continue;
        }

        let mut deviation = 0.0;
        for corner in 0..4 {
            let mut xs = Vec::with_capacity(end - start - 1);
            let mut ys = Vec::with_capacity(end - start - 1);
            for (neighbor, points) in projected.iter().enumerate().take(end).skip(start) {
                if neighbor == index {
                    continue;
                }
                xs.push(points[corner].x);
                ys.push(points[corner].y);
            }
            let expected_x = median(xs);
            let expected_y = median(ys);
            deviation += (projected[index][corner].x - expected_x)
                .hypot(projected[index][corner].y - expected_y);
        }
        deviation /= 4.0;
        let trustworthy = sample.inlier_ratio >= 0.22 && sample.reprojection_error <= 5.0;
        if deviation > threshold && !trustworthy {
            bad.push((index, deviation));
        }
    }

    let mut cursor = 0;
    while cursor < bad.len() {
        let group_start = cursor;
        while cursor + 1 < bad.len() && bad[cursor + 1].0 == bad[cursor].0 + 1 {
            cursor += 1;
        }
        let first = bad[group_start].0;
        let last = bad[cursor].0;
        if first > 0 && last + 1 < samples.len() {
            let left = samples[first - 1].transform;
            let right = samples[last + 1].transform;
            let span = (last - first + 2) as f64;
            for (offset, sample) in samples[first..=last].iter_mut().enumerate() {
                let t = (offset + 1) as f64 / span;
                sample.transform =
                    interpolate_plaque_transform(plaque, left, right, t).unwrap_or(left);
                sample.inlier_ratio = 0.0;
                sample.reprojection_error =
                    sample.reprojection_error.max(bad[group_start + offset].1);
                sample.ecc = None;
            }
        }
        cursor += 1;
    }

    bad.len()
}

/// Bridges intervals where a foreground crossing can dominate plaque evidence.
pub(crate) fn stabilize_occluded_intervals(
    samples: &mut [MotionSample],
    plaque: RectF,
    reference_frame: usize,
) -> usize {
    if samples.len() < 5 {
        return 0;
    }
    let peak = samples
        .iter()
        .map(|sample| sample.occluder_coverage)
        .fold(0.0_f64, f64::max);
    if peak < 0.04 {
        return 0;
    }
    let threshold = (peak * 0.18).max(0.015);
    let active = samples
        .iter()
        .enumerate()
        .filter_map(|(frame, sample)| (sample.occluder_coverage >= threshold).then_some(frame))
        .collect::<Vec<_>>();
    let mut repaired = 0;
    let mut cursor = 0;
    while cursor < active.len() {
        let start = cursor;
        while cursor + 1 < active.len() && active[cursor + 1] <= active[cursor] + 2 {
            cursor += 1;
        }
        let active_frames = &active[start..=cursor];
        let integrated_coverage = active_frames
            .iter()
            .map(|&frame| samples[frame].occluder_coverage)
            .sum::<f64>();
        if active_frames.len() < 5 || integrated_coverage < 0.75 {
            cursor += 1;
            continue;
        }
        let first_active = active[start];
        let last_active = active[cursor];
        let first = first_active.saturating_sub(4);
        let last = (last_active + 18).min(samples.len() - 1);
        let mut anchors = Vec::with_capacity(3);
        if first > 0 {
            anchors.push((first - 1, samples[first - 1].transform));
        }
        if (first..=last).contains(&reference_frame) {
            anchors.push((reference_frame, samples[reference_frame].transform));
        }
        if last + 1 < samples.len() {
            anchors.push((last + 1, samples[last + 1].transform));
        }
        anchors.sort_by_key(|anchor| anchor.0);
        anchors.dedup_by_key(|anchor| anchor.0);
        for (frame, sample) in samples.iter_mut().enumerate().take(last + 1).skip(first) {
            if anchors.iter().any(|anchor| anchor.0 == frame) {
                continue;
            }
            let left = anchors.iter().rev().find(|anchor| anchor.0 < frame);
            let right = anchors.iter().find(|anchor| anchor.0 > frame);
            let transform = match (left, right) {
                (Some(&(left_frame, left)), Some(&(right_frame, right))) => {
                    let t = (frame - left_frame) as f64 / (right_frame - left_frame) as f64;
                    interpolate_plaque_transform(plaque, left, right, t)
                }
                (Some(&(_, transform)), None) | (None, Some(&(_, transform))) => Some(transform),
                (None, None) => None,
            };
            if let Some(transform) = transform {
                sample.transform = transform;
                sample.ecc = None;
                repaired += 1;
            }
        }
        cursor += 1;
    }
    repaired
}

fn interpolate_plaque_transform(plaque: RectF, left: Mat3, right: Mat3, t: f64) -> Option<Mat3> {
    let source = GeoQuad::from_rect(plaque.x, plaque.y, plaque.width, plaque.height);
    let target = transformed_plaque(plaque, left).lerp(transformed_plaque(plaque, right), t);
    target.validate("interpolated plaque").ok()?;
    let matrix = homography(source, target).ok()?;
    Some(Mat3 { values: matrix.m })
}

fn should_close_loop(capture: &mut VideoCapture, frame_count: usize) -> Result<bool> {
    capture.set(CAP_PROP_POS_FRAMES, 0.0)?;
    let mut first = Mat::default();
    capture.read(&mut first)?;
    capture.set(CAP_PROP_POS_FRAMES, frame_count.saturating_sub(1) as f64)?;
    let mut last = Mat::default();
    capture.read(&mut last)?;
    let first = grayscale(&first)?;
    let last = grayscale(&last)?;
    let mut difference = Mat::default();
    core::absdiff(&first, &last, &mut difference)?;
    let mean = core::mean(&difference, &core::no_array())?.0[0];
    Ok(mean < 18.0)
}

fn mat_to_mat3(matrix: &Mat) -> Result<Mat3> {
    let mut values = Mat3::IDENTITY.values;
    for row in 0..matrix.rows() {
        for column in 0..matrix.cols() {
            values[row as usize][column as usize] = if matrix.typ() == core::CV_32F {
                *matrix.at_2d::<f32>(row, column)? as f64
            } else {
                *matrix.at_2d::<f64>(row, column)?
            };
        }
    }
    Ok(Mat3 { values })
}

fn symmetric_reprojection_error(
    source: &Vector<Point2f>,
    destination: &Vector<Point2f>,
    matrix: Mat3,
) -> Result<f64> {
    let forward = correspondence_errors(source, destination, matrix)?;
    let inverse = matrix
        .inverse()
        .context("estimated transform is singular")?;
    let reverse = correspondence_errors(destination, source, inverse)?;
    Ok(median(
        forward
            .into_iter()
            .zip(reverse)
            .map(|(a, b)| (a + b) * 0.5)
            .collect(),
    ))
}

fn correspondence_errors(
    source: &Vector<Point2f>,
    destination: &Vector<Point2f>,
    matrix: Mat3,
) -> Result<Vec<f64>> {
    let count = source.len().min(destination.len());
    let mut errors = Vec::with_capacity(count);
    for index in 0..count {
        let source = source.get(index)?;
        let destination = destination.get(index)?;
        let projected = matrix.transform(PointF {
            x: source.x as f64,
            y: source.y as f64,
        });
        errors.push(
            ((projected.x - destination.x as f64).powi(2)
                + (projected.y - destination.y as f64).powi(2))
            .sqrt(),
        );
    }
    Ok(errors)
}

fn draw_diagnostic(mut frame: Mat, plaque: RectF, transform: Mat3) -> Result<Mat> {
    let transformed = plaque_corners(plaque).map(|point| transform.transform(point));
    let mut contour = Vector::<Point>::new();
    for point in transformed {
        contour.push(Point::new(point.x.round() as i32, point.y.round() as i32));
    }
    let mut contours = Vector::<Vector<Point>>::new();
    contours.push(contour);
    imgproc::polylines(
        &mut frame,
        &contours,
        true,
        Scalar::new(0.0, 255.0, 255.0, 0.0),
        3,
        imgproc::LINE_AA,
        0,
    )?;
    Ok(frame)
}

fn write_contact_sheet(frames: &[Mat], path: &Path) -> Result<()> {
    if frames.is_empty() {
        return Ok(());
    }
    let tile_width = 360;
    let tile_height =
        (frames[0].rows() as f64 * tile_width as f64 / frames[0].cols() as f64).round() as i32;
    let columns = 3;
    let rows = (frames.len() as i32 + columns - 1) / columns;
    let mut sheet = Mat::new_rows_cols_with_default(
        rows * tile_height,
        columns * tile_width,
        core::CV_8UC3,
        Scalar::all(0.0),
    )?;
    for (index, frame) in frames.iter().enumerate() {
        let mut tile = Mat::default();
        imgproc::resize(
            frame,
            &mut tile,
            core::Size::new(tile_width, tile_height),
            0.0,
            0.0,
            imgproc::INTER_AREA,
        )?;
        let x = index as i32 % columns * tile_width;
        let y = index as i32 / columns * tile_height;
        let mut target = Mat::roi_mut(&mut sheet, Rect::new(x, y, tile_width, tile_height))?;
        tile.copy_to(&mut target)?;
    }
    imgcodecs::imwrite(&path.to_string_lossy(), &sheet, &Vector::new())?;
    Ok(())
}

pub fn median(mut values: Vec<f64>) -> f64 {
    if values.is_empty() {
        return f64::INFINITY;
    }
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

#[cfg(test)]
mod tests {
    use super::{
        TrackingResult, apply_motion_refinement, apply_visibility_refinements, optimize_trajectory,
        oriented_rect_quad, plaque_feature_mask, plaque_transform_is_valid,
        reapply_locked_refinements, refined_loop_closed, select_masked_refinement,
        stabilize_occluded_intervals, static_estimate, transformed_quad,
    };
    use crate::{
        geometry::Quad,
        model::{Mat3, MotionSample, RectF},
        refinement::{CoordinateSystem, MotionKeyframe, MotionRefinement},
    };
    use opencv::{
        core::{Point2f, Vector},
        prelude::MatTraitConst,
    };

    #[test]
    fn masked_retracking_cannot_replace_a_more_confident_track() {
        let result = select_masked_refinement(
            tracking_result("baseline", 0.82, 1.0),
            tracking_result("masked", 0.48, 9.0),
        );

        assert_eq!(result.confidence, 0.82);
        assert_eq!(result.samples[0].transform.values[0][2], 1.0);
        assert!(result.model_name.ends_with("-masked-retrack-rejected"));
    }

    #[test]
    fn masked_retracking_replaces_a_weaker_track() {
        let result = select_masked_refinement(
            tracking_result("baseline", 0.48, 1.0),
            tracking_result("masked", 0.82, 9.0),
        );

        assert_eq!(result.confidence, 0.82);
        assert_eq!(result.samples[0].transform.values[0][2], 9.0);
        assert_eq!(result.model_name, "masked");
    }

    fn tracking_result(model: &str, confidence: f64, translation: f64) -> TrackingResult {
        TrackingResult {
            samples: vec![MotionSample {
                frame: 0,
                transform: Mat3 {
                    values: [[1.0, 0.0, translation], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                },
                inlier_ratio: confidence,
                reprojection_error: 0.0,
                ecc: Some(1.0),
                plaque_visibility: 1.0,
                occluder_coverage: 0.0,
            }],
            model_name: model.into(),
            reference_frame: 0,
            confidence,
            loop_closed: false,
        }
    }

    #[test]
    fn plaque_feature_mask_uses_border_not_background_or_cavity() {
        let mask = plaque_feature_mask(
            400,
            300,
            RectF {
                x: 100.0,
                y: 80.0,
                width: 200.0,
                height: 100.0,
            },
        )
        .unwrap();

        assert_eq!(*mask.at_2d::<u8>(10, 10).unwrap(), 0);
        assert_eq!(*mask.at_2d::<u8>(80, 100).unwrap(), 255);
        assert_eq!(*mask.at_2d::<u8>(130, 200).unwrap(), 0);
    }

    #[test]
    fn plaque_transform_rejects_mirrored_geometry() {
        let plaque = RectF {
            x: 100.0,
            y: 80.0,
            width: 200.0,
            height: 100.0,
        };
        let mirrored = Mat3 {
            values: [[-1.0, 0.0, 400.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        };

        assert!(plaque_transform_is_valid(Mat3::IDENTITY, plaque));
        assert!(!plaque_transform_is_valid(mirrored, plaque));
    }

    #[test]
    fn oriented_contour_quad_preserves_long_and_short_edges() {
        let quad = oriented_rect_quad(200.0, 100.0, 40.0, 160.0, -90.0);
        assert!((quad.orientation() - 6_400.0).abs() < 1e-6);
        assert!((quad.tl.x - 120.0).abs() < 1e-6);
        assert!((quad.br.x - 280.0).abs() < 1e-6);
    }

    #[test]
    fn static_model_requires_stationary_well_distributed_matches() {
        let source = Vector::<Point2f>::from_iter([
            Point2f::new(10.0, 10.0),
            Point2f::new(90.0, 10.0),
            Point2f::new(90.0, 50.0),
            Point2f::new(10.0, 50.0),
            Point2f::new(50.0, 10.0),
            Point2f::new(50.0, 50.0),
            Point2f::new(10.0, 30.0),
            Point2f::new(90.0, 30.0),
        ]);
        let stationary = source.clone();
        let moving = Vector::from_iter(
            source
                .iter()
                .map(|point| Point2f::new(point.x + 3.0, point.y)),
        );

        assert!(
            static_estimate(&source, &stationary, 1.0)
                .unwrap()
                .is_some()
        );
        assert!(static_estimate(&source, &moving, 1.0).unwrap().is_none());
    }

    #[test]
    fn trajectory_optimizer_reduces_an_isolated_jump() {
        let plaque = RectF {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        };
        let mut samples = (0..9)
            .map(|frame| MotionSample {
                frame,
                transform: Mat3::translation(if frame == 4 { 8.0 } else { 0.0 }, 0.0),
                inlier_ratio: if frame == 4 { 0.0 } else { 1.0 },
                reprojection_error: if frame == 4 { 12.0 } else { 0.0 },
                ecc: None,
                plaque_visibility: 1.0,
                occluder_coverage: 0.0,
            })
            .collect::<Vec<_>>();

        optimize_trajectory(&mut samples, plaque, 0, 0.35, false).unwrap();

        assert!(samples[4].transform.values[0][2] < 5.0);
        assert_eq!(samples[0].transform.values, Mat3::IDENTITY.values);
    }

    #[test]
    fn foreground_interval_is_bridged_from_clean_endpoints() {
        let plaque = RectF {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        };
        let mut samples = (0..50)
            .map(|frame| MotionSample {
                frame,
                transform: Mat3::translation(
                    if (10..=25).contains(&frame) {
                        40.0
                    } else {
                        0.0
                    },
                    0.0,
                ),
                inlier_ratio: 0.8,
                reprojection_error: 0.5,
                ecc: Some(1.0),
                plaque_visibility: 1.0,
                occluder_coverage: if (10..=15).contains(&frame) {
                    0.20
                } else {
                    0.0
                },
            })
            .collect::<Vec<_>>();

        let repaired = stabilize_occluded_intervals(&mut samples, plaque, 0);

        assert!(repaired >= 20);
        assert!(samples[15].transform.values[0][2].abs() < 1.0e-9);
        assert!(samples[25].transform.values[0][2].abs() < 1.0e-9);
    }

    #[test]
    fn brief_residual_does_not_replace_valid_motion() {
        let plaque = RectF {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 50.0,
        };
        let mut samples = (0..50)
            .map(|frame| MotionSample {
                frame,
                transform: Mat3::translation(frame as f64, 0.0),
                inlier_ratio: 0.8,
                reprojection_error: 0.5,
                ecc: Some(1.0),
                plaque_visibility: 1.0,
                occluder_coverage: if (20..=21).contains(&frame) {
                    0.15
                } else {
                    0.0
                },
            })
            .collect::<Vec<_>>();

        let repaired = stabilize_occluded_intervals(&mut samples, plaque, 0);

        assert_eq!(repaired, 0);
        assert_eq!(samples[20].transform.values[0][2], 20.0);
    }

    #[test]
    fn reference_inside_foreground_interval_remains_an_anchor() {
        let plaque = RectF {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        };
        let mut samples = (0..50)
            .map(|frame| MotionSample {
                frame,
                transform: Mat3::translation(
                    if (10..=35).contains(&frame) {
                        40.0
                    } else {
                        0.0
                    },
                    0.0,
                ),
                inlier_ratio: 0.8,
                reprojection_error: 0.5,
                ecc: Some(1.0),
                plaque_visibility: 1.0,
                occluder_coverage: if (10..=20).contains(&frame) {
                    0.20
                } else {
                    0.0
                },
            })
            .collect::<Vec<_>>();
        samples[25].transform = Mat3::IDENTITY;

        let repaired = stabilize_occluded_intervals(&mut samples, plaque, 25);

        assert!(repaired >= 30);
        assert_eq!(samples[25].transform.values, Mat3::IDENTITY.values);
        assert!(samples[20].transform.values[0][2].abs() < 1.0e-9);
        assert!(samples[35].transform.values[0][2].abs() < 1.0e-9);
    }

    #[test]
    fn trailing_foreground_interval_holds_the_last_clean_transform() {
        let plaque = RectF {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        };
        let mut samples = (0..40)
            .map(|frame| MotionSample {
                frame,
                transform: Mat3::translation(if frame >= 30 { 40.0 } else { 0.0 }, 0.0),
                inlier_ratio: 0.8,
                reprojection_error: 0.5,
                ecc: Some(1.0),
                plaque_visibility: 1.0,
                occluder_coverage: if frame >= 30 { 0.20 } else { 0.0 },
            })
            .collect::<Vec<_>>();

        let repaired = stabilize_occluded_intervals(&mut samples, plaque, 0);

        assert!(repaired >= 10);
        assert!(samples[39].transform.values[0][2].abs() < 1.0e-9);
    }

    #[test]
    fn sparse_refined_keyframe_constrains_an_all_frame_track() {
        let plaque = RectF {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 50.0,
        };
        let sample = |frame| MotionSample {
            frame,
            transform: Mat3::IDENTITY,
            inlier_ratio: 0.8,
            reprojection_error: 0.5,
            ecc: None,
            plaque_visibility: 1.0,
            occluder_coverage: 0.0,
        };
        let mut result = TrackingResult {
            samples: (0..3).map(sample).collect(),
            model_name: "automatic-inertia-0.35".into(),
            reference_frame: 0,
            confidence: 0.8,
            loop_closed: false,
        };
        let track = MotionRefinement {
            schema_version: 1,
            plaque: "main".into(),
            coordinates: CoordinateSystem::SourcePixels,
            source_sha256: None,
            keyframes: vec![MotionKeyframe {
                frame: 1,
                quad: [[15.0, 20.0], [115.0, 20.0], [115.0, 70.0], [15.0, 70.0]],
                locked: true,
                visibility: None,
            }],
        };

        apply_motion_refinement(&mut result, &track, plaque).unwrap();

        assert_eq!(result.samples.len(), 3);
        let source = Quad::from_rect(plaque.x, plaque.y, plaque.width, plaque.height);
        let constrained = transformed_quad(source, result.samples[1].transform);
        assert!((constrained.tl.x - 15.0).abs() < 1.0e-9);
        assert!(
            result
                .model_name
                .starts_with("refined-constrained-quad-track-")
        );
    }

    #[test]
    fn dense_locked_track_is_authoritative() {
        let plaque = RectF {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 5.0,
        };
        let mut result = TrackingResult {
            samples: (0..2)
                .map(|frame| MotionSample {
                    frame,
                    transform: Mat3::IDENTITY,
                    inlier_ratio: 0.5,
                    reprojection_error: 1.0,
                    ecc: None,
                    plaque_visibility: 1.0,
                    occluder_coverage: 0.0,
                })
                .collect(),
            model_name: "automatic-inertia-0.35".into(),
            reference_frame: 0,
            confidence: 0.5,
            loop_closed: false,
        };
        let keyframe = |frame| MotionKeyframe {
            frame,
            quad: [[0.0, 0.0], [10.0, 0.0], [10.0, 5.0], [0.0, 5.0]],
            locked: true,
            visibility: Some(1.0),
        };
        let track = MotionRefinement {
            schema_version: 1,
            plaque: "main".into(),
            coordinates: CoordinateSystem::SourcePixels,
            source_sha256: None,
            keyframes: vec![keyframe(0), keyframe(1)],
        };

        apply_motion_refinement(&mut result, &track, plaque).unwrap();

        assert!(
            result
                .model_name
                .starts_with("authoritative-refined-quad-track-")
        );
        assert_eq!(result.confidence, 0.99);
    }

    #[test]
    fn mixed_track_reapplies_only_locked_samples_after_refinement() {
        let plaque = RectF {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 5.0,
        };
        let sample = |frame| MotionSample {
            frame,
            transform: Mat3::IDENTITY,
            inlier_ratio: 0.8,
            reprojection_error: 0.5,
            ecc: None,
            plaque_visibility: 1.0,
            occluder_coverage: 0.0,
        };
        let track = MotionRefinement {
            schema_version: 1,
            plaque: "main".into(),
            coordinates: CoordinateSystem::SourcePixels,
            source_sha256: None,
            keyframes: vec![
                MotionKeyframe {
                    frame: 0,
                    quad: [[2.0, 0.0], [12.0, 0.0], [12.0, 5.0], [2.0, 5.0]],
                    locked: false,
                    visibility: None,
                },
                MotionKeyframe {
                    frame: 2,
                    quad: [[6.0, 0.0], [16.0, 0.0], [16.0, 5.0], [6.0, 5.0]],
                    locked: true,
                    visibility: None,
                },
            ],
        };
        let mut result = TrackingResult {
            samples: (0..3).map(sample).collect(),
            model_name: "automatic-inertia-0.35".into(),
            reference_frame: 0,
            confidence: 0.8,
            loop_closed: false,
        };

        apply_motion_refinement(&mut result, &track, plaque).unwrap();
        assert!(result.model_name.starts_with("refined-mixed-quad-track-"));
        result.samples.iter_mut().for_each(|sample| {
            sample.transform = Mat3::IDENTITY;
        });
        reapply_locked_refinements(&mut result.samples, &track, plaque).unwrap();

        let source = Quad::from_rect(plaque.x, plaque.y, plaque.width, plaque.height);
        let guided = transformed_quad(source, result.samples[0].transform);
        let locked = transformed_quad(source, result.samples[2].transform);
        assert!((guided.tl.x - 6.0).abs() < 1.0e-9);
        assert!((locked.tl.x - 6.0).abs() < 1.0e-9);
    }

    #[test]
    fn authored_visibility_is_applied_after_automatic_detection() {
        let sample = |frame, plaque_visibility| MotionSample {
            frame,
            transform: Mat3::IDENTITY,
            inlier_ratio: 0.8,
            reprojection_error: 0.5,
            ecc: None,
            plaque_visibility,
            occluder_coverage: 0.0,
        };
        let mut samples = vec![
            sample(0, 0.2),
            sample(1, 0.4),
            sample(2, 0.6),
            sample(3, 0.8),
            sample(4, 1.0),
        ];
        let keyframe = |frame, visibility| MotionKeyframe {
            frame,
            quad: [[0.0, 0.0], [10.0, 0.0], [10.0, 5.0], [0.0, 5.0]],
            locked: false,
            visibility,
        };
        let track = MotionRefinement {
            schema_version: 1,
            plaque: "main".into(),
            coordinates: CoordinateSystem::SourcePixels,
            source_sha256: None,
            keyframes: vec![
                keyframe(1, Some(0.3)),
                keyframe(2, None),
                keyframe(3, Some(0.4)),
            ],
        };

        apply_visibility_refinements(&mut samples, &track).unwrap();

        let expected = [0.1, 0.3, 0.6, 0.4, 0.6];
        for (sample, expected) in samples.iter().zip(expected) {
            assert!((sample.plaque_visibility - expected).abs() < 1.0e-9);
        }
    }

    #[test]
    fn loop_closure_uses_endpoint_inference() {
        let plaque = RectF {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 5.0,
        };
        let sample = |frame, translation| MotionSample {
            frame,
            transform: Mat3 {
                values: [[1.0, 0.0, translation], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            },
            inlier_ratio: 1.0,
            reprojection_error: 0.0,
            ecc: Some(1.0),
            plaque_visibility: 1.0,
            occluder_coverage: 0.0,
        };
        let open_samples = vec![sample(0, 0.0), sample(1, 10.0)];
        let closed_samples = vec![sample(0, 0.0), sample(1, 0.0)];

        assert!(!refined_loop_closed(&open_samples, plaque));
        assert!(refined_loop_closed(&closed_samples, plaque));
    }
}
