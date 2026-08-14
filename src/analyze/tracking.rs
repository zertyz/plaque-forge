//! Plaque motion tracking across the source video.
//!
//! A tracker estimates a projective transform for each frame, then stabilizes that
//! trajectory and applies any locked or guiding motion scenes.

use std::{collections::VecDeque, path::Path};

use anyhow::{Context, Result, bail};
use opencv::{
    core::{
        self, DMatch, KeyPoint, Mat, Point, Point2f, Rect, Scalar, Size, TermCriteria,
        TermCriteria_Type, Vector,
    },
    features, features2d, geometry, imgcodecs, imgproc,
    prelude::*,
    video as cv_video,
    videoio::{CAP_PROP_FRAME_COUNT, CAP_PROP_POS_FRAMES, VideoCapture},
};

use crate::{
    cli::AnalyzeArgs,
    geometry::{Point as GeoPoint, Quad as GeoQuad, homography},
    model::{Mat3, MotionSample, PointF, RectF},
    progress::ProgressReporter,
    scene::SurfaceTrajectory,
    surface::Surface,
    video::VideoInfo,
};

#[derive(Debug, Clone, Copy)]
enum MotionModel {
    Adaptive,
    Similarity,
    Affine,
    Projective,
}

pub fn apply_motion_scene(
    result: &mut TrackingResult,
    track: &SurfaceTrajectory,
    plaque: RectF,
) -> Result<()> {
    apply_scene_constraints(&mut result.samples, track, plaque, ConstraintSelection::All)?;

    let dense = track.is_dense_locked(result.samples.len());
    let locked = track.locked_keyframes();
    let guides = track.guide_keyframes();
    let base_model = std::mem::take(&mut result.model_name);
    result.model_name = if dense {
        result.confidence = 0.99;
        for sample in &mut result.samples {
            sample.inlier_ratio = 1.0;
            sample.reprojection_error = 0.0;
            sample.ecc = Some(1.0);
        }
        format!("reviewed-dense-quad-track-{}-frames", track.keyframes.len())
    } else if locked > 0 && guides > 0 {
        format!("reviewed-mixed-quad-track-{locked}-locked-{guides}-guided+{base_model}")
    } else if locked > 0 {
        format!(
            "reviewed-constrained-quad-track-{}-keyframes+{base_model}",
            locked
        )
    } else {
        format!(
            "reviewed-guided-quad-track-{}-keyframes+{base_model}",
            track.keyframes.len()
        )
    };

    result.loop_closed = trajectory_loop_closed(&result.samples, plaque);
    Ok(())
}

pub fn reapply_locked_scenes(
    samples: &mut [MotionSample],
    track: &SurfaceTrajectory,
    plaque: RectF,
) -> Result<()> {
    if track.locked_keyframes() == 0 {
        return Ok(());
    }
    apply_scene_constraints(samples, track, plaque, ConstraintSelection::Locked)
}

#[derive(Clone, Copy)]
enum ConstraintSelection {
    All,
    Locked,
}

fn apply_scene_constraints(
    samples: &mut [MotionSample],
    track: &SurfaceTrajectory,
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
        let desired = scene_quad(keyframe.quad);
        desired.validate(&format!("trajectory anchor {}", keyframe.frame))?;
        corrections.push((keyframe.frame, quad_difference(desired, *current)));
    }
    if corrections.is_empty() {
        return Ok(());
    }

    for (frame, sample) in samples.iter_mut().enumerate() {
        let correction = correction_at(&corrections, frame);
        let corrected = quad_sum(automatic[frame], correction);
        corrected.validate(&format!("reviewed-constrained frame {frame}"))?;
        sample.transform = Mat3 {
            values: homography(source, corrected)?.m,
        };
    }

    Ok(())
}

pub fn apply_visibility_scenes(
    samples: &mut [MotionSample],
    track: &SurfaceTrajectory,
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

pub(crate) fn transformed_quad(source: GeoQuad, transform: Mat3) -> GeoQuad {
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

fn scene_quad(points: [[f64; 2]; 4]) -> GeoQuad {
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

fn trajectory_loop_closed(samples: &[MotionSample], plaque: RectF) -> bool {
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

pub fn screen_fixed(
    frame_count: usize,
    reference_frame: usize,
    confidence: f64,
    model_name: &str,
) -> TrackingResult {
    TrackingResult {
        samples: (0..frame_count)
            .map(|frame| MotionSample {
                frame,
                transform: Mat3::IDENTITY,
                measurement_valid: false,
                tracked_points: 0,
                spatial_coverage: 0.0,
                uncertainty_px: 0.0,
                measurement_source: "screen-canvas".into(),
                inlier_ratio: confidence,
                reprojection_error: 0.0,
                ecc: Some(1.0),
                plaque_visibility: 1.0,
                occluder_coverage: 0.0,
            })
            .collect(),
        model_name: model_name.to_string(),
        reference_frame: reference_frame.min(frame_count.saturating_sub(1)),
        confidence: confidence.clamp(0.0, 1.0),
        loop_closed: false,
    }
}

pub fn select_masked_scene(mut baseline: TrackingResult, scene: TrackingResult) -> TrackingResult {
    if scene.confidence >= baseline.confidence {
        scene
    } else {
        baseline.model_name.push_str("-masked-retrack-rejected");
        baseline
    }
}

/// Keeps the strongest absolute pose measurements, then refines that same global
/// trajectory using the final foreground-aware source flow. A fully masked
/// retrack is retained only when its own absolute evidence is genuinely stronger;
/// sparse masked frames must not discard a well-rooted baseline.
#[allow(clippy::too_many_arguments)]
pub fn refine_scene_with_masked_flow(
    args: &AnalyzeArgs,
    info: &VideoInfo,
    plaque: RectF,
    baseline: TrackingResult,
    masked: TrackingResult,
    exclusion_root: &Path,
    progress: &mut ProgressReporter,
) -> Result<TrackingResult> {
    let selected = select_masked_scene(baseline, masked);
    refine_selected_with_foreground_flow(
        args,
        info,
        plaque,
        selected,
        exclusion_root,
        progress,
        "foreground-aware-source-flow",
    )
}

/// Applies the final foreground-aware material-flow solve when the fully masked
/// absolute retracker cannot cover the complete shot. The original trajectory
/// remains the absolute-pose prior, but foreground pixels are excluded from the
/// noncausal relative-motion graph exactly as they are for a successful masked
/// retrack.
#[allow(clippy::too_many_arguments)]
pub fn refine_baseline_with_foreground_flow(
    args: &AnalyzeArgs,
    info: &VideoInfo,
    plaque: RectF,
    baseline: TrackingResult,
    exclusion_root: &Path,
    progress: &mut ProgressReporter,
) -> Result<TrackingResult> {
    refine_selected_with_foreground_flow(
        args,
        info,
        plaque,
        baseline,
        exclusion_root,
        progress,
        "masked-retrack-unusable-foreground-aware-source-flow",
    )
}

#[allow(clippy::too_many_arguments)]
fn refine_selected_with_foreground_flow(
    args: &AnalyzeArgs,
    info: &VideoInfo,
    plaque: RectF,
    mut selected: TrackingResult,
    exclusion_root: &Path,
    progress: &mut ProgressReporter,
    model_suffix: &str,
) -> Result<TrackingResult> {
    let summary = constrain_trajectory_to_source_flow(
        args,
        info,
        plaque,
        selected.reference_frame,
        selected.loop_closed,
        &mut selected.samples,
        Some(exclusion_root),
        progress,
    )?;
    selected.confidence = selected
        .confidence
        .min(summary.confidence.unwrap_or(selected.confidence));
    selected.model_name.push('-');
    selected.model_name.push_str(model_suffix);
    eprintln!(
        "foreground-aware trajectory constraint: source-flow p95 {:.2}px -> {:.2}px, p99 {:.2}px -> {:.2}px",
        summary.before_p95, summary.after_p95, summary.before_p99, summary.after_p99
    );
    Ok(selected)
}

pub fn load_dense_scene(
    args: &AnalyzeArgs,
    info: &VideoInfo,
    plaque: RectF,
    track: &SurfaceTrajectory,
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
            let keyframe = keyframe.with_context(|| format!("missing reviewed frame {frame}"))?;
            let matrix = homography(source, scene_quad(keyframe.quad))?;
            Ok(MotionSample {
                frame,
                transform: Mat3 { values: matrix.m },
                measurement_valid: false,
                tracked_points: 0,
                spatial_coverage: 0.0,
                uncertainty_px: 0.0,
                measurement_source: "declared-trajectory".into(),
                inlier_ratio: 1.0,
                reprojection_error: 0.0,
                ecc: Some(1.0),
                plaque_visibility: keyframe.visibility.unwrap_or(1.0),
                occluder_coverage: 0.0,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let mut capture = crate::video::open_capture(&args.input)?;
    progress.start(3, 7, "Load reviewed plaque trajectory", Some(info.frames));
    write_tracking_diagnostics(&mut capture, &samples, plaque, diagnostics, info.frames)?;
    progress.update(
        info.frames,
        format!("{} locked frames", track.keyframes.len()),
    );
    progress.finish("authoritative all-frame quadrilateral track");

    let loop_closed = trajectory_loop_closed(&samples, plaque);
    Ok(TrackingResult {
        samples,
        model_name: format!("reviewed-dense-quad-track-{}-frames", track.keyframes.len()),
        reference_frame: args.surface_frame.unwrap_or(0),
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
    tracking_exclusions: Option<&Path>,
) -> Result<TrackingResult> {
    track_with_exclusions(
        args,
        info,
        plaque,
        reference_frame,
        diagnostics,
        progress,
        tracking_exclusions,
    )
}

#[allow(clippy::too_many_arguments)]
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

    let mut capture = crate::video::open_capture(&args.input)?;
    let actual_frames = capture.get(CAP_PROP_FRAME_COUNT)?.round().max(1.0) as usize;
    let frame_count = info.frames.min(actual_frames).max(1);
    let reference_frame = reference_frame.min(frame_count.saturating_sub(1));

    // Destination features are searched across the full frame so a temporarily
    // wrong prediction cannot exclude the real surface from reacquisition. Keep
    // enough features for a narrow plaque to compete with a detailed background.
    let mut sift = features2d::SIFT::create(4_000, 3, 0.015, 12.0, 1.6, true)?;
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
    let root_persistent_points = persistent_corner_count(
        &root.gray,
        plaque,
        Mat3::IDENTITY,
        reference_exclusion.as_ref(),
    )?;
    // SIFT is the drift-closing reacquisition path, not the sole proof that a
    // physical plane is observable. Low-texture metal/cloud faces can carry few
    // descriptors after a precise support matte removes their detailed border,
    // yet still provide many well-distributed subpixel corners for persistent
    // forward/backward flow. Allow that primary material-point path to initialize;
    // all-frame source-flow verification remains an independent hard gate.
    if (root.descriptors.empty() || root.keypoints.len() < 8) && root_persistent_points < 12 {
        bail!(
            "insufficient stable writing-surface features at frame {reference_frame} \
             ({} SIFT descriptors, {root_persistent_points} subpixel corners); \
             inspect {} and correct the surface bounds/support in the scene if needed",
            root.keypoints.len(),
            diagnostics.join("candidate.png").display()
        );
    }

    let loop_closed = should_close_loop(&mut capture, frame_count)?;
    // A physical surface is measured throughout the clip even when a sparse
    // sample happens to look static. Subtle late motion must never be turned into
    // a screen-fixed title.
    let mut raw: Vec<Option<MotionSample>> = vec![None; frame_count];
    raw[reference_frame] = Some(MotionSample {
        frame: reference_frame,
        transform: Mat3::IDENTITY,
        measurement_valid: true,
        tracked_points: root.keypoints.len().max(root_persistent_points),
        spatial_coverage: 1.0,
        uncertainty_px: 0.25,
        measurement_source: "reference-frame".into(),
        inlier_ratio: 1.0,
        reprojection_error: 0.0,
        ecc: Some(1.0),
        plaque_visibility: 1.0,
        occluder_coverage: 0.0,
    });

    progress.start(
        3,
        7,
        "Bidirectional persistent scene tracking",
        Some(frame_count.saturating_mul(2)),
    );
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

    let reverse_anchor = find_reverse_anchor_from_measurements(
        &mut capture,
        &mut sift,
        &raw,
        plaque,
        reference_frame,
        occluder_masks,
    )?;
    if let Some(reverse_root) = reverse_anchor {
        let reverse_contour =
            detect_plaque_contour(&reverse_root.gray, plaque, reverse_root.transform)?;
        let mut reverse = vec![None; frame_count];
        reverse[reverse_root.frame] = Some(anchor_sample(&reverse_root));
        let reverse_indices = ((reference_frame + 1)..reverse_root.frame)
            .rev()
            .collect::<Vec<_>>();
        adaptive_anchor_count += process_direction(
            args,
            &mut capture,
            &mut sift,
            &reverse_root,
            reverse_contour.as_ref(),
            plaque,
            &reverse_indices,
            &mut reverse,
            &mut completed,
            progress,
            occluder_masks,
        )?;
        fuse_bidirectional_measurements(&mut raw, &reverse, plaque)?;
    }

    let mut samples: Vec<MotionSample> = raw
        .into_iter()
        .enumerate()
        .map(|(frame, sample)| {
            sample.unwrap_or(MotionSample {
                frame,
                transform: Mat3::IDENTITY,
                measurement_valid: false,
                tracked_points: 0,
                spatial_coverage: 0.0,
                uncertainty_px: f64::INFINITY,
                measurement_source: "missing".into(),
                inlier_ratio: 0.0,
                reprojection_error: f64::INFINITY,
                ecc: None,
                plaque_visibility: 1.0,
                occluder_coverage: 0.0,
            })
        })
        .collect();
    let repaired_frames = repair_outliers(&mut samples, plaque);
    let inferred_frames =
        solve_unobserved_intervals(&mut samples, plaque, info.width, info.height)?;
    optimize_trajectory(
        &mut samples,
        plaque,
        reference_frame,
        args.tracking_inertia,
        loop_closed,
    )?;
    let source_flow = if occluder_masks.is_some() {
        constrain_trajectory_to_source_flow(
            args,
            info,
            plaque,
            reference_frame,
            loop_closed,
            &mut samples,
            occluder_masks,
            progress,
        )?
    } else {
        SourceFlowConstraintSummary::not_evaluated()
    };
    let source_flow_status = if source_flow.confidence.is_some() {
        format!(
            "source-flow p95 {:.2}px -> {:.2}px, p99 {:.2}px -> {:.2}px",
            source_flow.before_p95,
            source_flow.after_p95,
            source_flow.before_p99,
            source_flow.after_p99,
        )
    } else {
        "source-flow not evaluated until foreground exclusions are available".to_string()
    };
    progress.finish(format!(
        "{adaptive_anchor_count} adaptive anchors, {inferred_frames} inferred, {repaired_frames} rejected measurements; {source_flow_status}"
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
    let measurement_confidence =
        (median_inliers * (-median_error / 4.0).exp() - repair_penalty).clamp(0.0, 0.99);
    let confidence = source_flow
        .confidence
        .map_or(measurement_confidence, |source| {
            measurement_confidence.min(source)
        });

    Ok(TrackingResult {
        samples,
        model_name: format!(
            "bidirectional-persistent-point-homography-sift-{:?}-regularization-{:.2}-global-source-flow",
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
    let mut flow = PersistentPointTracker::new(
        &root.gray,
        plaque,
        root.transform,
        load_exclusion(
            occluder_masks,
            root.frame,
            root.gray.cols(),
            root.gray.rows(),
        )?
        .as_ref(),
    )?;
    let mut last_transform = root.transform;
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
        // This pose is used only to limit the search region. It is never emitted
        // as a measurement; gaps are solved after both temporal directions have
        // been inspected.
        let predicted = last_transform;
        let mut plaque_mask =
            plaque_feature_mask_for_transform(gray.cols(), gray.rows(), plaque, predicted)?;
        let exclusion = load_exclusion(occluder_masks, frame_index, gray.cols(), gray.rows())?;
        let excluded = apply_exclusion(&mut plaque_mask, exclusion.as_ref())?;
        let persistent = flow
            .advance(&gray, plaque, exclusion.as_ref())?
            .filter(|estimate| plaque_transform_is_valid(estimate.matrix, plaque));
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
                                matrix: Mat3 { values: matrix.m }.multiply(root.transform),
                                inlier_ratio: current.confidence,
                                error: (1.0 - current.confidence) * 2.0,
                                tracked_points: 4,
                                spatial_coverage: 1.0,
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
        let mut search_mask = Mat::new_rows_cols_with_default(
            gray.rows(),
            gray.cols(),
            core::CV_8UC1,
            Scalar::all(255.0),
        )?;
        apply_exclusion(&mut search_mask, exclusion.as_ref())?;
        sift.detect_and_compute(&gray, &search_mask, &mut keypoints, &mut descriptors, false)?;

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
            estimate.anchored(root.transform, source)
        });

        let feature = choose_estimate(local, direct, plaque, predicted);
        let reacquired = choose_geometric_constraint(feature, contour, plaque);
        let trustworthy_reacquisition = reacquired
            .as_ref()
            .is_some_and(reacquisition_can_reseed_points);
        let estimate = choose_persistent_estimate(persistent, reacquired, plaque, predicted);
        let valid = estimate.as_ref().is_some_and(measurement_is_credible);
        let estimate = estimate.unwrap_or(Estimate {
            matrix: predicted,
            inlier_ratio: 0.0,
            error: f64::INFINITY,
            tracked_points: 0,
            spatial_coverage: 0.0,
            ecc: None,
            source: "unobserved",
            static_model: false,
        });
        output[frame_index] = Some(MotionSample {
            frame: frame_index,
            transform: estimate.matrix,
            measurement_valid: valid,
            tracked_points: estimate.tracked_points,
            spatial_coverage: estimate.spatial_coverage,
            uncertainty_px: measurement_uncertainty(&estimate),
            measurement_source: estimate.source.into(),
            inlier_ratio: estimate.inlier_ratio,
            reprojection_error: estimate.error,
            ecc: estimate.ecc,
            plaque_visibility: 1.0,
            occluder_coverage: 0.0,
        });
        if valid {
            last_transform = estimate.matrix;
            if estimate.source != "persistent-flow" && trustworthy_reacquisition {
                // A fixed-root or geometric reacquisition is an independent loop
                // closure. Rebind fresh image points to that canonical pose rather
                // than carrying any suspect rolling identities across the event.
                flow.reset(&gray, plaque, estimate.matrix, exclusion.as_ref())?;
            }
        }

        let trustworthy = valid;
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

fn anchor_sample(anchor: &FeatureAnchor) -> MotionSample {
    MotionSample {
        frame: anchor.frame,
        transform: anchor.transform,
        measurement_valid: true,
        tracked_points: anchor.keypoints.len(),
        spatial_coverage: 1.0,
        uncertainty_px: 0.35,
        measurement_source: "reverse-root".into(),
        inlier_ratio: 1.0,
        reprojection_error: 0.0,
        ecc: Some(1.0),
        plaque_visibility: 1.0,
        occluder_coverage: 0.0,
    }
}

fn find_reverse_anchor_from_measurements(
    capture: &mut VideoCapture,
    sift: &mut core::Ptr<features2d::SIFT>,
    measurements: &[Option<MotionSample>],
    plaque: RectF,
    reference_frame: usize,
    occluder_masks: Option<&Path>,
) -> Result<Option<FeatureAnchor>> {
    for frame in (reference_frame + 16..measurements.len()).rev() {
        let Some(sample) = measurements[frame].as_ref().filter(|sample| {
            sample.measurement_valid
                && matches!(
                    sample.measurement_source.as_str(),
                    "root" | "root-static" | "adaptive" | "geometry"
                )
                && sample.tracked_points >= 24
                && sample.spatial_coverage >= 0.55
                && sample.uncertainty_px <= 2.0
                && sample_confidence(sample) >= 0.28
                && plaque_transform_is_valid(sample.transform, plaque)
        }) else {
            continue;
        };
        let gray = read_gray(capture, frame)?;
        let exclusion = load_exclusion(occluder_masks, frame, gray.cols(), gray.rows())?;
        let anchor = make_anchor(
            sift,
            frame,
            gray,
            plaque,
            sample.transform,
            exclusion.as_ref(),
        )?;
        if anchor.keypoints.len() >= 8 && !anchor.descriptors.empty() {
            return Ok(Some(anchor));
        }
    }
    Ok(None)
}

fn fuse_bidirectional_measurements(
    forward: &mut [Option<MotionSample>],
    reverse: &[Option<MotionSample>],
    plaque: RectF,
) -> Result<()> {
    for (frame, candidate) in forward.iter_mut().zip(reverse) {
        let Some(candidate) = candidate.as_ref().filter(|sample| sample.measurement_valid) else {
            continue;
        };
        match frame {
            Some(existing) if existing.measurement_valid => {
                let disagreement =
                    mean_corner_distance(existing.transform, candidate.transform, plaque);
                if disagreement <= 2.5 {
                    let left = transformed_plaque(plaque, existing.transform);
                    let right = transformed_plaque(plaque, candidate.transform);
                    let left_weight = sample_confidence(existing).max(0.05);
                    let right_weight = sample_confidence(candidate).max(0.05);
                    let fused = left.lerp(right, right_weight / (left_weight + right_weight));
                    if inferred_quad_is_physical(fused, plaque) {
                        existing.transform = Mat3 {
                            values: homography(
                                GeoQuad::from_rect(plaque.x, plaque.y, plaque.width, plaque.height),
                                fused,
                            )?
                            .m,
                        };
                        existing.measurement_source = "bidirectional-fused".into();
                        existing.tracked_points += candidate.tracked_points;
                        existing.spatial_coverage =
                            existing.spatial_coverage.max(candidate.spatial_coverage);
                        existing.inlier_ratio = existing.inlier_ratio.max(candidate.inlier_ratio);
                        existing.reprojection_error = existing
                            .reprojection_error
                            .min(candidate.reprojection_error);
                        existing.uncertainty_px =
                            disagreement.max(existing.uncertainty_px.min(candidate.uncertainty_px));
                    }
                } else {
                    // The reverse pass starts from an independently reacquired
                    // pose, but its point track still depends on that single late
                    // anchor. It cannot veto or weaken a valid forward material
                    // observation; a reverse observation is authoritative only
                    // where it fills a forward gap.
                }
            }
            slot @ None => {
                let mut candidate = candidate.clone();
                candidate.measurement_source = "reverse-persistent".into();
                *slot = Some(candidate);
            }
            Some(existing) => {
                *existing = candidate.clone();
                existing.measurement_source = "reverse-persistent".into();
            }
        }
    }
    Ok(())
}

/// Tracks durable material points whose coordinates always live on the reference
/// plaque. Lucas-Kanade only advances their image observations; it never changes
/// their canonical identity. Points added after an occlusion are inverse-projected
/// through an independently accepted pose before they can participate.
struct PersistentPointTracker {
    previous_gray: Mat,
    canonical: Vector<Point2f>,
    current: Vector<Point2f>,
}

impl PersistentPointTracker {
    fn new(gray: &Mat, plaque: RectF, pose: Mat3, exclusion: Option<&Mat>) -> Result<Self> {
        let mut tracker = Self {
            previous_gray: gray.try_clone()?,
            canonical: Vector::new(),
            current: Vector::new(),
        };
        tracker.reset(gray, plaque, pose, exclusion)?;
        Ok(tracker)
    }

    fn advance(
        &mut self,
        gray: &Mat,
        plaque: RectF,
        exclusion: Option<&Mat>,
    ) -> Result<Option<Estimate>> {
        if self.current.len() < 8 {
            self.previous_gray = gray.try_clone()?;
            self.canonical.clear();
            self.current.clear();
            return Ok(None);
        }

        let criteria = TermCriteria::new(
            i32::from(TermCriteria_Type::COUNT) + i32::from(TermCriteria_Type::EPS),
            40,
            0.005,
        )?;
        let mut next = Vector::<Point2f>::new();
        let mut forward_status = Vector::<u8>::new();
        let mut forward_error = Vector::<f32>::new();
        cv_video::calc_optical_flow_pyr_lk(
            &self.previous_gray,
            gray,
            &self.current,
            &mut next,
            &mut forward_status,
            &mut forward_error,
            Size::new(31, 31),
            4,
            criteria,
            0,
            1.0e-4,
        )?;

        // A forward result is accepted only when tracking it back reaches the
        // prior observation. This cheaply rejects points captured by a crossing
        // spider, vine, web, highlight, or newly revealed background.
        let mut back = Vector::<Point2f>::new();
        let mut backward_status = Vector::<u8>::new();
        let mut backward_error = Vector::<f32>::new();
        cv_video::calc_optical_flow_pyr_lk(
            gray,
            &self.previous_gray,
            &next,
            &mut back,
            &mut backward_status,
            &mut backward_error,
            Size::new(31, 31),
            4,
            criteria,
            0,
            1.0e-4,
        )?;

        let mut canonical = Vector::<Point2f>::new();
        let mut observed = Vector::<Point2f>::new();
        for index in 0..self.current.len() {
            if forward_status.get(index)? == 0 || backward_status.get(index)? == 0 {
                continue;
            }
            let before = self.current.get(index)?;
            let after = next.get(index)?;
            let returned = back.get(index)?;
            let fb_error = (before.x - returned.x).hypot(before.y - returned.y);
            if !after.x.is_finite()
                || !after.y.is_finite()
                || fb_error > 1.25
                || after.x < 0.0
                || after.y < 0.0
                || after.x >= gray.cols() as f32
                || after.y >= gray.rows() as f32
                || point_is_excluded(exclusion, after)?
            {
                continue;
            }
            canonical.push(self.canonical.get(index)?);
            observed.push(after);
        }

        self.previous_gray = gray.try_clone()?;
        self.canonical = canonical;
        self.current = observed;
        let estimate = estimate_persistent_transform(&self.canonical, &self.current, plaque).ok();
        if let Some(estimate) = &estimate {
            self.prune_to_pose(estimate.matrix, 3.0)?;
        }
        Ok(estimate)
    }

    fn reset(
        &mut self,
        gray: &Mat,
        plaque: RectF,
        pose: Mat3,
        exclusion: Option<&Mat>,
    ) -> Result<()> {
        self.previous_gray = gray.try_clone()?;
        self.canonical.clear();
        self.current.clear();
        self.add_corners(gray, plaque, pose, exclusion, 1_200)
    }

    fn add_corners(
        &mut self,
        gray: &Mat,
        plaque: RectF,
        pose: Mat3,
        exclusion: Option<&Mat>,
        maximum: i32,
    ) -> Result<()> {
        let Some(inverse) = pose.inverse() else {
            return Ok(());
        };
        let mut mask = plaque_feature_mask_for_transform(gray.cols(), gray.rows(), plaque, pose)?;
        apply_exclusion(&mut mask, exclusion)?;
        for point in &self.current {
            imgproc::circle(
                &mut mask,
                Point::new(point.x.round() as i32, point.y.round() as i32),
                5,
                Scalar::all(0.0),
                -1,
                imgproc::LINE_8,
                0,
            )?;
        }
        let mut detected = Vector::<Point2f>::new();
        features::good_features_to_track(
            gray,
            &mut detected,
            maximum,
            0.005,
            4.0,
            &mask,
            5,
            false,
            0.04,
        )?;
        if !detected.is_empty() {
            imgproc::corner_sub_pix(
                gray,
                &mut detected,
                Size::new(3, 3),
                Size::new(-1, -1),
                TermCriteria::new(
                    i32::from(TermCriteria_Type::COUNT) + i32::from(TermCriteria_Type::EPS),
                    20,
                    0.01,
                )?,
            )?;
        }
        for point in detected {
            let mapped = inverse.transform(PointF {
                x: f64::from(point.x),
                y: f64::from(point.y),
            });
            if mapped.x.is_finite()
                && mapped.y.is_finite()
                && mapped.x >= plaque.x
                && mapped.y >= plaque.y
                && mapped.x <= plaque.x + plaque.width
                && mapped.y <= plaque.y + plaque.height
            {
                self.canonical
                    .push(Point2f::new(mapped.x as f32, mapped.y as f32));
                self.current.push(point);
            }
        }
        Ok(())
    }

    fn prune_to_pose(&mut self, pose: Mat3, maximum_error: f64) -> Result<()> {
        let mut canonical = Vector::<Point2f>::new();
        let mut current = Vector::<Point2f>::new();
        for index in 0..self.canonical.len() {
            let source = self.canonical.get(index)?;
            let observed = self.current.get(index)?;
            let expected = pose.transform(PointF {
                x: f64::from(source.x),
                y: f64::from(source.y),
            });
            if (expected.x - f64::from(observed.x)).hypot(expected.y - f64::from(observed.y))
                <= maximum_error
            {
                canonical.push(source);
                current.push(observed);
            }
        }
        self.canonical = canonical;
        self.current = current;
        Ok(())
    }
}

fn persistent_corner_count(
    gray: &Mat,
    plaque: RectF,
    pose: Mat3,
    exclusion: Option<&Mat>,
) -> Result<usize> {
    let mut tracker = PersistentPointTracker {
        previous_gray: gray.try_clone()?,
        canonical: Vector::new(),
        current: Vector::new(),
    };
    tracker.add_corners(gray, plaque, pose, exclusion, 1_200)?;
    Ok(tracker.current.len())
}

fn point_is_excluded(exclusion: Option<&Mat>, point: Point2f) -> Result<bool> {
    let Some(exclusion) = exclusion else {
        return Ok(false);
    };
    let x = point.x.round() as i32;
    let y = point.y.round() as i32;
    if x < 0 || y < 0 || x >= exclusion.cols() || y >= exclusion.rows() {
        return Ok(true);
    }
    Ok(*exclusion.at_2d::<u8>(y, x)? > 0)
}

/// Independent source-frame evidence used by render verification.
///
/// Unlike the analysis tracker, this starts fresh on every frame pair. It observes
/// material motion with forward/backward Lucas-Kanade flow, then asks how closely
/// the persisted four-corner trajectory predicts those observations. It therefore
/// detects a screen-fixed or lagging trajectory without registering the rendered
/// title or trusting the tracker's own residuals.
#[derive(Debug, Clone)]
pub(crate) struct SourceFlowConsistency {
    pub median_error_pixels: f64,
    pub inlier_fraction: f64,
    pub tracked_points: usize,
    pub spatial_coverage: f64,
    pub flow_model_inlier_fraction: f64,
    /// Independent image-space motion from the earlier source frame to the later
    /// one. This is estimated from fresh forward/backward optical flow, not from
    /// either stored surface pose.
    pub material_transform: Mat3,
    pub(crate) flow_model_error_pixels: f64,
    correspondences: Vec<(PointF, PointF)>,
}

impl SourceFlowConsistency {
    fn error_for_poses(&self, plaque: RectF, previous_pose: Mat3, current_pose: Mat3) -> f64 {
        let Some(previous_inverse) = previous_pose.inverse() else {
            return f64::INFINITY;
        };
        let mut errors = Vec::with_capacity(self.correspondences.len());
        for &(source, _observed) in &self.correspondences {
            let material = previous_inverse.transform(source);
            if !material.x.is_finite()
                || !material.y.is_finite()
                || material.x < plaque.x
                || material.y < plaque.y
                || material.x > plaque.x + plaque.width
                || material.y > plaque.y + plaque.height
            {
                continue;
            }
            let predicted = current_pose.transform(material);
            // Compare two projective motion models: the stored four-corner plane
            // and a fresh robust homography fitted directly to source pixels.
            // Comparing against each raw LK endpoint instead would charge the
            // trajectory for irreducible optical-flow noise and non-planar
            // animation (for example a softly billowing cloud plaque), even when
            // it exactly matches the independently observed best-fit plane.
            let independently_observed = self.material_transform.transform(source);
            errors.push(
                (predicted.x - independently_observed.x)
                    .hypot(predicted.y - independently_observed.y),
            );
        }
        median(errors)
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn measure_source_flow_consistency(
    previous: &Surface,
    current: &Surface,
    plaque: RectF,
    previous_pose: Mat3,
    current_pose: Mat3,
    previous_exclusion: Option<&[u8]>,
    current_exclusion: Option<&[u8]>,
) -> Result<Option<SourceFlowConsistency>> {
    if previous.width() != current.width() || previous.height() != current.height() {
        bail!("source-flow frames have different dimensions");
    }
    let width = previous.width() as i32;
    let height = previous.height() as i32;
    let previous_gray = surface_luma_mat(previous)?;
    let current_gray = surface_luma_mat(current)?;
    let mut feature_mask = plaque_feature_mask_for_transform(width, height, plaque, previous_pose)?;
    apply_byte_exclusion(&mut feature_mask, previous_exclusion, width, height)?;

    let mut before = Vector::<Point2f>::new();
    features::good_features_to_track(
        &previous_gray,
        &mut before,
        800,
        0.005,
        5.0,
        &feature_mask,
        7,
        false,
        0.04,
    )?;
    if before.len() < 16 {
        return Ok(None);
    }
    imgproc::corner_sub_pix(
        &previous_gray,
        &mut before,
        Size::new(3, 3),
        Size::new(-1, -1),
        TermCriteria::new(
            i32::from(TermCriteria_Type::COUNT) + i32::from(TermCriteria_Type::EPS),
            20,
            0.01,
        )?,
    )?;

    let criteria = TermCriteria::new(
        i32::from(TermCriteria_Type::COUNT) + i32::from(TermCriteria_Type::EPS),
        40,
        0.005,
    )?;
    let mut after = Vector::<Point2f>::new();
    let mut forward_status = Vector::<u8>::new();
    let mut forward_error = Vector::<f32>::new();
    cv_video::calc_optical_flow_pyr_lk(
        &previous_gray,
        &current_gray,
        &before,
        &mut after,
        &mut forward_status,
        &mut forward_error,
        Size::new(31, 31),
        4,
        criteria,
        0,
        1.0e-4,
    )?;
    let mut returned = Vector::<Point2f>::new();
    let mut backward_status = Vector::<u8>::new();
    let mut backward_error = Vector::<f32>::new();
    cv_video::calc_optical_flow_pyr_lk(
        &current_gray,
        &previous_gray,
        &after,
        &mut returned,
        &mut backward_status,
        &mut backward_error,
        Size::new(31, 31),
        4,
        criteria,
        0,
        1.0e-4,
    )?;

    let Some(previous_inverse) = previous_pose.inverse() else {
        return Ok(None);
    };
    let mut source_points = Vector::<Point2f>::new();
    let mut observed_points = Vector::<Point2f>::new();
    for index in 0..before.len() {
        if forward_status.get(index)? == 0 || backward_status.get(index)? == 0 {
            continue;
        }
        let source = before.get(index)?;
        let observed = after.get(index)?;
        let round_trip = returned.get(index)?;
        if !observed.x.is_finite()
            || !observed.y.is_finite()
            || (source.x - round_trip.x).hypot(source.y - round_trip.y) > 1.25
            || observed.x < 0.0
            || observed.y < 0.0
            || observed.x >= width as f32
            || observed.y >= height as f32
            || byte_point_is_excluded(current_exclusion, width, height, observed)
        {
            continue;
        }
        source_points.push(source);
        observed_points.push(observed);
    }
    if source_points.len() < 16 {
        return Ok(None);
    }
    // A fresh robust model rejects coherent foreground motion (a spider, vine,
    // web, bird, or arm) before the stored plaque trajectory is evaluated.
    let robust = estimate_model(
        &source_points,
        &observed_points,
        MotionModel::Projective,
        1.0,
    )
    .or_else(|_| estimate_model(&source_points, &observed_points, MotionModel::Affine, 1.0))?;
    if robust.inlier_ratio < 0.40 {
        return Ok(None);
    }

    let mut canonical = Vector::<Point2f>::new();
    let mut correspondences = Vec::new();
    let mut errors = Vec::new();
    for index in 0..source_points.len() {
        let source = source_points.get(index)?;
        let observed = observed_points.get(index)?;
        let robust_prediction = robust.matrix.transform(PointF {
            x: f64::from(source.x),
            y: f64::from(source.y),
        });
        if (robust_prediction.x - f64::from(observed.x))
            .hypot(robust_prediction.y - f64::from(observed.y))
            > 2.25
        {
            continue;
        }
        let material = previous_inverse.transform(PointF {
            x: f64::from(source.x),
            y: f64::from(source.y),
        });
        if !material.x.is_finite()
            || !material.y.is_finite()
            || material.x < plaque.x
            || material.y < plaque.y
            || material.x > plaque.x + plaque.width
            || material.y > plaque.y + plaque.height
        {
            continue;
        }
        let predicted = current_pose.transform(material);
        let independently_observed = robust.matrix.transform(PointF {
            x: f64::from(source.x),
            y: f64::from(source.y),
        });
        // The independent verifier evaluates rigid projective plane agreement.
        // Raw endpoint scatter remains represented by `robust.error` and gates
        // whether this observation is usable; it is not motion disagreement.
        errors.push(
            (predicted.x - independently_observed.x).hypot(predicted.y - independently_observed.y),
        );
        canonical.push(Point2f::new(material.x as f32, material.y as f32));
        correspondences.push((
            PointF {
                x: f64::from(source.x),
                y: f64::from(source.y),
            },
            PointF {
                x: f64::from(observed.x),
                y: f64::from(observed.y),
            },
        ));
    }
    if errors.len() < 16 {
        return Ok(None);
    }
    let spatial_coverage = plaque_point_coverage(plaque, &canonical);
    if spatial_coverage < 0.30 {
        return Ok(None);
    }
    let inlier_fraction =
        errors.iter().filter(|&&error| error <= 1.5).count() as f64 / errors.len() as f64;
    Ok(Some(SourceFlowConsistency {
        median_error_pixels: median(errors),
        inlier_fraction,
        tracked_points: canonical.len(),
        spatial_coverage,
        flow_model_inlier_fraction: robust.inlier_ratio,
        material_transform: robust.matrix,
        flow_model_error_pixels: robust.error,
        correspondences,
    }))
}

fn surface_luma_mat(surface: &Surface) -> Result<Mat> {
    let mut gray = Mat::new_rows_cols_with_default(
        surface.height() as i32,
        surface.width() as i32,
        core::CV_8UC1,
        Scalar::all(0.0),
    )?;
    for (destination, source) in gray
        .data_bytes_mut()?
        .iter_mut()
        .zip(surface.pixels().chunks_exact(4))
    {
        *destination = ((u32::from(source[0]) * 54
            + u32::from(source[1]) * 183
            + u32::from(source[2]) * 19
            + 128)
            / 256) as u8;
    }
    Ok(gray)
}

fn apply_byte_exclusion(
    mask: &mut Mat,
    exclusion: Option<&[u8]>,
    width: i32,
    height: i32,
) -> Result<()> {
    let Some(exclusion) = exclusion else {
        return Ok(());
    };
    if exclusion.is_empty() {
        return Ok(());
    }
    if exclusion.len() != width as usize * height as usize {
        bail!("source-flow exclusion dimensions do not match the frame");
    }
    for (allowed, &hidden) in mask.data_bytes_mut()?.iter_mut().zip(exclusion) {
        if hidden >= 16 {
            *allowed = 0;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct SourceFlowConstraintSummary {
    before_p95: f64,
    before_p99: f64,
    after_p95: f64,
    after_p99: f64,
    confidence: Option<f64>,
}

impl SourceFlowConstraintSummary {
    fn not_evaluated() -> Self {
        Self {
            before_p95: 0.0,
            before_p99: 0.0,
            after_p95: 0.0,
            after_p99: 0.0,
            confidence: None,
        }
    }
}

/// Constrains the absolute four-corner track with fresh adjacent-frame material
/// motion. The persistent tracker supplies globally rooted poses; this independent
/// pass supplies the relative motion that those poses must obey. Solving the whole
/// clip in both temporal directions prevents a foreground crossing from producing
/// a one-frame impulse and prevents a causal "notice, then catch up" fallback.
#[allow(clippy::too_many_arguments)]
fn constrain_trajectory_to_source_flow(
    args: &AnalyzeArgs,
    info: &VideoInfo,
    plaque: RectF,
    reference_frame: usize,
    loop_closed: bool,
    samples: &mut [MotionSample],
    exclusion_root: Option<&Path>,
    progress: &mut ProgressReporter,
) -> Result<SourceFlowConstraintSummary> {
    if samples.len() < 2 {
        return Ok(SourceFlowConstraintSummary {
            before_p95: 0.0,
            before_p99: 0.0,
            after_p95: 0.0,
            after_p99: 0.0,
            confidence: Some(1.0),
        });
    }

    let mut decoder = crate::video::Decoder::spawn(&args.ffmpeg, &args.input, info)?;
    let first = decoder
        .next_frame()?
        .context("source-flow solver could not decode frame 0")?;
    let first_exclusion =
        load_exclusion_bytes(exclusion_root, 0, info.width as usize, info.height as usize)?;
    let mut history = VecDeque::from([(0usize, first, first_exclusion)]);
    let mut observations = vec![Vec::<(usize, SourceFlowConsistency)>::new(); samples.len()];
    for frame in 1..samples.len() {
        let Some(current) = decoder.next_frame()? else {
            bail!("source-flow solver ended before frame {frame}");
        };
        let current_exclusion = load_exclusion_bytes(
            exclusion_root,
            frame,
            info.width as usize,
            info.height as usize,
        )?;
        for lag in [1usize, 6, 12] {
            let Some(previous_frame) = frame.checked_sub(lag) else {
                continue;
            };
            let Some((_, previous, previous_exclusion)) = history
                .iter()
                .find(|(index, _, _)| *index == previous_frame)
            else {
                continue;
            };
            if samples[previous_frame].plaque_visibility < 0.5
                || samples[frame].plaque_visibility < 0.5
                || surface_visible_fraction(
                    plaque,
                    samples[previous_frame].transform,
                    info.width,
                    info.height,
                ) < 0.30
                || surface_visible_fraction(
                    plaque,
                    samples[frame].transform,
                    info.width,
                    info.height,
                ) < 0.30
            {
                continue;
            }
            if let Some(observation) = measure_source_flow_consistency(
                previous,
                &current,
                plaque,
                samples[previous_frame].transform,
                samples[frame].transform,
                previous_exclusion.as_deref(),
                current_exclusion.as_deref(),
            )?
            .filter(source_flow_observation_is_usable)
            {
                observations[frame].push((previous_frame, observation));
            }
        }
        history.push_back((frame, current, current_exclusion));
        while history
            .front()
            .is_some_and(|(index, _, _)| frame.saturating_sub(*index) > 12)
        {
            history.pop_front();
        }
        progress.update(
            samples.len() + frame,
            format!(
                "independent source-flow constraint {frame}/{}",
                samples.len() - 1
            ),
        );
    }
    decoder.finish()?;

    let mut before = source_flow_errors_for_trajectory(&observations, samples, plaque);
    let before_p95 = percentile(&mut before, 0.95);
    let before_p99 = percentile(&mut before, 0.99);
    let original = samples.to_vec();
    let source = GeoQuad::from_rect(plaque.x, plaque.y, plaque.width, plaque.height);
    let original_quads = original
        .iter()
        .map(|sample| transformed_plaque(plaque, sample.transform))
        .collect::<Vec<_>>();
    let corrected = solve_global_source_flow_quads(
        &observations,
        &original,
        &original_quads,
        reference_frame,
        plaque,
    );
    assign_quads(samples, source, &corrected)?;
    let soft_errors = source_flow_errors_for_trajectory(&observations, samples, plaque);
    let soft_quality = source_flow_candidate_quality(soft_errors);

    let integrated = solve_source_flow_by_reference_integration(
        &observations,
        &original_quads,
        reference_frame,
        plaque,
    );
    assign_quads(samples, source, &integrated)?;
    let integrated_errors = source_flow_errors_for_trajectory(&observations, samples, plaque);
    let integrated_quality = source_flow_candidate_quality(integrated_errors);

    let before_quality = (before_p95, before_p99);
    eprintln!(
        "source-flow candidates: baseline p95 {:.2}px p99 {:.2}px; global p95 {:.2}px p99 {:.2}px; exact integration p95 {:.2}px p99 {:.2}px",
        before_quality.0,
        before_quality.1,
        soft_quality.0,
        soft_quality.1,
        integrated_quality.0,
        integrated_quality.1,
    );
    let (after_p95, after_p99) = if source_flow_quality_is_better(integrated_quality, soft_quality)
        && source_flow_quality_is_better(integrated_quality, before_quality)
    {
        eprintln!("source-flow candidate selected: exact integration");
        integrated_quality
    } else if source_flow_quality_is_better(soft_quality, before_quality) {
        assign_quads(samples, source, &corrected)?;
        eprintln!("source-flow candidate selected: global pose/flow graph");
        soft_quality
    } else {
        samples.clone_from_slice(&original);
        eprintln!("source-flow candidate selected: baseline absolute trajectory");
        before_quality
    };
    // The first graph changes where the plaque material is sampled. Re-observe
    // every scale in that aligned geometry, then solve once more. This is the
    // standard coarse-to-fine step that prevents an initially displaced track
    // from measuring a coherent background patch instead of the plaque.
    if (after_p95, after_p99) != before_quality {
        observations = collect_source_flow_observations(
            args,
            info,
            plaque,
            samples,
            exclusion_root,
            progress,
        )?;
        let aligned_quads = samples
            .iter()
            .map(|sample| transformed_plaque(plaque, sample.transform))
            .collect::<Vec<_>>();
        let polished = solve_global_source_flow_quads(
            &observations,
            samples,
            &aligned_quads,
            reference_frame,
            plaque,
        );
        let current = samples.to_vec();
        assign_quads(samples, source, &polished)?;
        let polished_quality = source_flow_candidate_quality(source_flow_errors_for_trajectory(
            &observations,
            samples,
            plaque,
        ));
        eprintln!(
            "source-flow coarse-to-fine candidate: p95 {:.2}px p99 {:.2}px",
            polished_quality.0, polished_quality.1
        );
        if !source_flow_quality_is_better(polished_quality, (after_p95, after_p99)) {
            samples.clone_from_slice(&current);
        }
    }
    let graph_anchors = samples
        .iter()
        .map(|sample| transformed_plaque(plaque, sample.transform))
        .collect::<Vec<_>>();
    let mut best = samples.to_vec();
    let mut best_quality = source_flow_candidate_quality(source_flow_errors_for_trajectory(
        &observations,
        &best,
        plaque,
    ));
    let flow_ceiling = (best_quality.0 * 1.20 + 0.08, best_quality.1 * 1.20 + 0.15);
    let mut best_dynamics = trajectory_dynamics(&best, plaque, loop_closed);
    let mut best_objective = trajectory_candidate_objective(best_quality, best_dynamics);
    for acceleration_weight in [0.30, 0.75, 1.50, 3.0, 6.0, 12.0, 24.0, 48.0, 96.0, 192.0] {
        let quads = solve_global_source_flow_quads_with_inertia(
            &observations,
            samples,
            &graph_anchors,
            reference_frame,
            plaque,
            acceleration_weight,
        );
        let mut candidate = samples.to_vec();
        assign_quads(&mut candidate, source, &quads)?;
        let quality = source_flow_candidate_quality(source_flow_errors_for_trajectory(
            &observations,
            &candidate,
            plaque,
        ));
        let dynamics = trajectory_dynamics(&candidate, plaque, loop_closed);
        let objective = trajectory_candidate_objective(quality, dynamics);
        eprintln!(
            "source-flow inertia candidate {:.2}: flow p95 {:.2}px p99 {:.2}px; dynamics {:.5}, p95 {:.2}px, max {:.2}px; objective {:.2}",
            acceleration_weight,
            quality.0,
            quality.1,
            dynamics.temporal_score,
            dynamics.p95_residual,
            dynamics.maximum_residual,
            objective
        );
        if quality.0 <= flow_ceiling.0
            && quality.1 <= flow_ceiling.1
            && trajectory_candidate_is_better(dynamics, objective, best_dynamics, best_objective)
        {
            best = candidate;
            best_quality = quality;
            best_dynamics = dynamics;
            best_objective = objective;
        }
    }
    eprintln!(
        "source-flow physical solution selected: temporal {:.5}, dynamics p95 {:.2}px, max {:.2}px at frame {}",
        best_dynamics.temporal_score,
        best_dynamics.p95_residual,
        best_dynamics.maximum_residual,
        best_dynamics.worst_frame,
    );
    samples.clone_from_slice(&best);
    let (after_p95, after_p99) = best_quality;
    let confidence = source_flow_confidence(after_p95, after_p99);
    Ok(SourceFlowConstraintSummary {
        before_p95,
        before_p99,
        after_p95,
        after_p99,
        confidence: Some(confidence),
    })
}

#[allow(clippy::too_many_arguments)]
fn collect_source_flow_observations(
    args: &AnalyzeArgs,
    info: &VideoInfo,
    plaque: RectF,
    samples: &[MotionSample],
    exclusion_root: Option<&Path>,
    progress: &mut ProgressReporter,
) -> Result<Vec<Vec<(usize, SourceFlowConsistency)>>> {
    let mut decoder = crate::video::Decoder::spawn(&args.ffmpeg, &args.input, info)?;
    let first = decoder
        .next_frame()?
        .context("source-flow solver could not decode frame 0")?;
    let first_exclusion =
        load_exclusion_bytes(exclusion_root, 0, info.width as usize, info.height as usize)?;
    let mut history = VecDeque::from([(0usize, first, first_exclusion)]);
    let mut observations = vec![Vec::new(); samples.len()];
    for frame in 1..samples.len() {
        let Some(current) = decoder.next_frame()? else {
            bail!("source-flow solver ended before frame {frame}");
        };
        let current_exclusion = load_exclusion_bytes(
            exclusion_root,
            frame,
            info.width as usize,
            info.height as usize,
        )?;
        for lag in [1usize, 6, 12] {
            let Some(previous_frame) = frame.checked_sub(lag) else {
                continue;
            };
            let Some((_, previous, previous_exclusion)) = history
                .iter()
                .find(|(index, _, _)| *index == previous_frame)
            else {
                continue;
            };
            if samples[previous_frame].plaque_visibility < 0.5
                || samples[frame].plaque_visibility < 0.5
                || surface_visible_fraction(
                    plaque,
                    samples[previous_frame].transform,
                    info.width,
                    info.height,
                ) < 0.30
                || surface_visible_fraction(
                    plaque,
                    samples[frame].transform,
                    info.width,
                    info.height,
                ) < 0.30
            {
                continue;
            }
            if let Some(observation) = measure_source_flow_consistency(
                previous,
                &current,
                plaque,
                samples[previous_frame].transform,
                samples[frame].transform,
                previous_exclusion.as_deref(),
                current_exclusion.as_deref(),
            )?
            .filter(source_flow_observation_is_usable)
            {
                observations[frame].push((previous_frame, observation));
            }
        }
        history.push_back((frame, current, current_exclusion));
        while history
            .front()
            .is_some_and(|(index, _, _)| frame.saturating_sub(*index) > 12)
        {
            history.pop_front();
        }
        progress.update(
            samples.len() + frame,
            format!(
                "coarse-to-fine source-flow pass {frame}/{}",
                samples.len() - 1
            ),
        );
    }
    decoder.finish()?;
    Ok(observations)
}

fn source_flow_candidate_quality(mut errors: Vec<f64>) -> (f64, f64) {
    let mut tail = errors.clone();
    (percentile(&mut errors, 0.95), percentile(&mut tail, 0.99))
}

fn source_flow_quality_is_better(candidate: (f64, f64), current: (f64, f64)) -> bool {
    candidate.0.is_finite()
        && candidate.1.is_finite()
        && (candidate.1 + 0.05 < current.1 || candidate.0 + 0.05 < current.0)
        && candidate.0 <= current.0 * 1.05 + 0.05
        && candidate.1 <= current.1 * 1.05 + 0.05
}

const MINIMUM_PHYSICAL_TEMPORAL_SCORE: f64 = 0.95;
const MAXIMUM_PHYSICAL_FRAME_RESIDUAL: f64 = 4.0;

fn trajectory_candidate_objective(
    source_flow_quality: (f64, f64),
    dynamics: TrajectoryDynamics,
) -> f64 {
    source_flow_quality.0
        + 0.60 * source_flow_quality.1
        + 0.20 * dynamics.p95_residual
        + 0.05 * dynamics.maximum_residual
        + 25.0 * (MINIMUM_PHYSICAL_TEMPORAL_SCORE - dynamics.temporal_score).max(0.0)
        + 2.0 * (dynamics.maximum_residual - MAXIMUM_PHYSICAL_FRAME_RESIDUAL).max(0.0)
}

fn trajectory_candidate_is_better(
    candidate: TrajectoryDynamics,
    candidate_objective: f64,
    current: TrajectoryDynamics,
    current_objective: f64,
) -> bool {
    let candidate_is_physical = candidate.is_physical();
    let current_is_physical = current.is_physical();
    (candidate_is_physical && !current_is_physical)
        || (candidate_is_physical == current_is_physical
            && candidate_objective + 1.0e-6 < current_objective)
}

fn assign_quads(samples: &mut [MotionSample], source: GeoQuad, quads: &[GeoQuad]) -> Result<()> {
    for (sample, quad) in samples.iter_mut().zip(quads) {
        sample.transform = Mat3 {
            values: homography(source, *quad)?.m,
        };
    }
    Ok(())
}

/// Exact relative integration is the fallback when the two-ended soft solve
/// cannot improve the independent residual. It is rooted at the strongest global
/// pose and propagates toward both past and future. This preserves every measured
/// inter-frame projective motion; drift remains bounded by later absolute review
/// diagnostics rather than being hidden by a locally smooth but detached title.
fn solve_source_flow_by_reference_integration(
    observations: &[Vec<(usize, SourceFlowConsistency)>],
    anchors: &[GeoQuad],
    reference_frame: usize,
    plaque: RectF,
) -> Vec<GeoQuad> {
    let mut solved = anchors.to_vec();
    for frame in reference_frame + 1..solved.len() {
        if let Some(observation) = adjacent_source_flow(observations, frame - 1, frame) {
            let candidate = transform_quad(solved[frame - 1], observation.material_transform);
            if inferred_quad_is_physical(candidate, plaque) {
                solved[frame] = candidate;
            }
        }
    }
    for frame in (0..reference_frame).rev() {
        if let Some(inverse) = adjacent_source_flow(observations, frame, frame + 1)
            .and_then(|observation| observation.material_transform.inverse())
        {
            let candidate = transform_quad(solved[frame + 1], inverse);
            if inferred_quad_is_physical(candidate, plaque) {
                solved[frame] = candidate;
            }
        }
    }
    solved
}

fn source_flow_observation_is_usable(observation: &SourceFlowConsistency) -> bool {
    observation.tracked_points >= 24
        && observation.spatial_coverage >= 0.42
        && observation.flow_model_inlier_fraction >= 0.68
        && observation.flow_model_error_pixels <= 1.5
}

fn load_exclusion_bytes(
    root: Option<&Path>,
    frame: usize,
    width: usize,
    height: usize,
) -> Result<Option<Vec<u8>>> {
    let Some(root) = root else {
        return Ok(None);
    };
    let path = root.join(format!("{frame:06}.png"));
    if !path.is_file() {
        return Ok(None);
    }
    let mask = image::open(&path)
        .with_context(|| format!("failed to read source-flow exclusion {}", path.display()))?
        .into_luma8();
    if mask.width() as usize != width || mask.height() as usize != height {
        bail!("source-flow exclusion dimensions differ from source video");
    }
    Ok(Some(mask.into_raw()))
}

fn source_flow_errors_for_trajectory(
    observations: &[Vec<(usize, SourceFlowConsistency)>],
    samples: &[MotionSample],
    plaque: RectF,
) -> Vec<f64> {
    observations
        .iter()
        .enumerate()
        .flat_map(|(frame, observations)| {
            observations.iter().map(move |(previous, observation)| {
                observation.error_for_poses(
                    plaque,
                    samples[*previous].transform,
                    samples[frame].transform,
                )
            })
        })
        .filter(|error| error.is_finite())
        .collect()
}

fn solve_global_source_flow_quads(
    observations: &[Vec<(usize, SourceFlowConsistency)>],
    samples: &[MotionSample],
    anchors: &[GeoQuad],
    reference_frame: usize,
    plaque: RectF,
) -> Vec<GeoQuad> {
    solve_global_source_flow_quads_with_inertia(
        observations,
        samples,
        anchors,
        reference_frame,
        plaque,
        0.12,
    )
}

fn solve_global_source_flow_quads_with_inertia(
    observations: &[Vec<(usize, SourceFlowConsistency)>],
    samples: &[MotionSample],
    anchors: &[GeoQuad],
    reference_frame: usize,
    plaque: RectF,
    acceleration_weight: f64,
) -> Vec<GeoQuad> {
    let mut solved = anchors.to_vec();
    for _ in 0..72 {
        let previous = solved.clone();
        for frame in 0..solved.len() {
            let mut accumulator = WeightedQuad::default();
            let absolute_weight = if frame == reference_frame {
                8.0
            } else {
                absolute_pose_weight(&samples[frame])
            };
            accumulator.add(anchors[frame], absolute_weight);

            for (source_frame, observation) in &observations[frame] {
                let proposal =
                    transform_quad(previous[*source_frame], observation.material_transform);
                accumulator.add(
                    proposal,
                    source_flow_edge_weight(frame - *source_frame, observation),
                );
            }
            for (future_frame, future_observations) in
                observations.iter().enumerate().skip(frame + 1)
            {
                for (_, observation) in future_observations
                    .iter()
                    .filter(|(source_frame, _)| *source_frame == frame)
                {
                    if let Some(inverse) = observation.material_transform.inverse() {
                        let proposal = transform_quad(previous[future_frame], inverse);
                        accumulator.add(
                            proposal,
                            source_flow_edge_weight(future_frame - frame, observation),
                        );
                    }
                }
            }
            if frame > 0 && frame + 1 < solved.len() {
                // Low-weight acceleration prior, evaluated non-causally.
                accumulator.add(
                    previous[frame - 1].lerp(previous[frame + 1], 0.5),
                    acceleration_weight,
                );
            }
            if let Some(candidate) = accumulator.finish()
                && inferred_quad_is_physical(candidate, plaque)
            {
                solved[frame] = candidate;
            }
        }
    }
    solved
}

#[derive(Default)]
struct WeightedQuad {
    coordinates: [[f64; 2]; 4],
    total: f64,
}

impl WeightedQuad {
    fn add(&mut self, quad: GeoQuad, weight: f64) {
        if !weight.is_finite() || weight <= 0.0 {
            return;
        }
        for (corner, point) in quad.points().into_iter().enumerate() {
            self.coordinates[corner][0] += point.x * weight;
            self.coordinates[corner][1] += point.y * weight;
        }
        self.total += weight;
    }

    fn finish(self) -> Option<GeoQuad> {
        if self.total <= f64::EPSILON {
            return None;
        }
        let points: [GeoPoint; 4] = std::array::from_fn(|corner| {
            GeoPoint::new(
                self.coordinates[corner][0] / self.total,
                self.coordinates[corner][1] / self.total,
            )
        });
        Some(GeoQuad::new(points[0], points[1], points[2], points[3]))
    }
}

fn absolute_pose_weight(sample: &MotionSample) -> f64 {
    if !sample.measurement_valid {
        return 0.0;
    }
    let source_weight = match sample.measurement_source.as_str() {
        "reference-frame" | "reverse-root" => 8.0,
        "root" | "root-static" => 2.5,
        "reverse-persistent" | "bidirectional-fused" => 1.5,
        "adaptive" => 0.60,
        "persistent-flow" => 0.12,
        _ => 0.08,
    };
    source_weight * (0.25 + 0.75 * sample_confidence(sample))
}

fn source_flow_edge_weight(lag: usize, observation: &SourceFlowConsistency) -> f64 {
    let base = match lag {
        1 => 18.0,
        6 => 10.0,
        12 => 7.0,
        _ => 2.0,
    };
    let support =
        (observation.flow_model_inlier_fraction * observation.spatial_coverage).clamp(0.45, 1.0);
    let residual = (-observation.flow_model_error_pixels.min(4.0) / 2.0).exp();
    base * support * (0.60 + 0.40 * residual)
}

fn adjacent_source_flow(
    observations: &[Vec<(usize, SourceFlowConsistency)>],
    previous: usize,
    current: usize,
) -> Option<&SourceFlowConsistency> {
    observations
        .get(current)?
        .iter()
        .find_map(|(source, observation)| (*source == previous).then_some(observation))
}

fn transform_quad(quad: GeoQuad, transform: Mat3) -> GeoQuad {
    let points = quad.points().map(|point| {
        let mapped = transform.transform(PointF {
            x: point.x,
            y: point.y,
        });
        GeoPoint::new(mapped.x, mapped.y)
    });
    GeoQuad::new(points[0], points[1], points[2], points[3])
}

fn percentile(values: &mut [f64], quantile: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    if values.is_empty() {
        return f64::INFINITY;
    }
    let index = ((values.len() - 1) as f64 * quantile.clamp(0.0, 1.0)).round() as usize;
    values[index]
}

fn source_flow_confidence(p95: f64, p99: f64) -> f64 {
    if !p95.is_finite() || !p99.is_finite() {
        return 0.0;
    }
    let p95_excess = (p95 - 0.85).max(0.0);
    let p99_excess = (p99 - 1.50).max(0.0);
    (-((p95_excess / 1.6).powi(2) + (p99_excess / 2.4).powi(2)))
        .exp()
        .clamp(0.0, 0.99)
}

fn byte_point_is_excluded(
    exclusion: Option<&[u8]>,
    width: i32,
    height: i32,
    point: Point2f,
) -> bool {
    let Some(exclusion) = exclusion.filter(|mask| !mask.is_empty()) else {
        return false;
    };
    let x = point.x.round() as i32;
    let y = point.y.round() as i32;
    x < 0
        || y < 0
        || x >= width
        || y >= height
        || exclusion
            .get(y as usize * width as usize + x as usize)
            .is_none_or(|&alpha| alpha >= 16)
}

fn estimate_persistent_transform(
    canonical: &Vector<Point2f>,
    observed: &Vector<Point2f>,
    plaque: RectF,
) -> Result<Estimate> {
    if canonical.len() < 8 || canonical.len() != observed.len() {
        bail!("insufficient persistent surface points");
    }
    let coverage = plaque_point_coverage(plaque, canonical);
    let model = if canonical.len() >= 16 && coverage >= 0.42 {
        MotionModel::Projective
    } else {
        MotionModel::Affine
    };
    let mut estimate = estimate_model(canonical, observed, model, coverage)?;
    estimate.source = "persistent-flow";
    estimate.static_model = false;
    Ok(estimate)
}

fn plaque_point_coverage(plaque: RectF, points: &Vector<Point2f>) -> f64 {
    if points.is_empty() || plaque.width <= 0.0 || plaque.height <= 0.0 {
        return 0.0;
    }
    let mut occupied = [false; 12];
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for point in points {
        let x = ((f64::from(point.x) - plaque.x) / plaque.width).clamp(0.0, 1.0);
        let y = ((f64::from(point.y) - plaque.y) / plaque.height).clamp(0.0, 1.0);
        let column = (x * 4.0).floor().clamp(0.0, 3.0) as usize;
        let row = (y * 3.0).floor().clamp(0.0, 2.0) as usize;
        occupied[row * 4 + column] = true;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    let sectors = occupied.into_iter().filter(|value| *value).count() as f64 / 12.0;
    let horizontal = (max_x - min_x).clamp(0.0, 1.0);
    let vertical = (max_y - min_y).clamp(0.0, 1.0);
    (0.65 * sectors + 0.175 * horizontal + 0.175 * vertical).clamp(0.0, 1.0)
}

fn measurement_is_credible(estimate: &Estimate) -> bool {
    if estimate.source == "persistent-flow" {
        // Persistent points already carry canonical material identities and pass
        // a forward/backward flow check. During a crossing, requiring the same
        // whole-plaque coverage as descriptor reacquisition needlessly discards
        // trustworthy one-sided support. `estimate_persistent_transform` falls
        // back to affine before coverage becomes too narrow for a homography.
        estimate.tracked_points >= 12
            && estimate.spatial_coverage >= 0.26
            && estimate.inlier_ratio >= 0.32
            && estimate.error.is_finite()
            && estimate.error <= 3.0
    } else {
        // A rolling adaptive anchor is useful for bridging appearance change, but
        // eight matches are not enough to establish a new four-corner plane. A
        // coherent foreground web/branch can easily supply that many matches and
        // then become a false root for every subsequent frame. Root descriptors
        // are globally anchored and geometry has four explicit boundary points;
        // adaptive reacquisition therefore has a deliberately stronger floor.
        let support_is_sufficient = if estimate.source == "adaptive" {
            estimate.tracked_points >= 20
                && estimate.spatial_coverage >= 0.55
                && estimate.inlier_ratio >= 0.38
                && estimate.error <= 2.5
        } else {
            estimate.tracked_points >= 8 || estimate.source == "geometry"
        };
        support_is_sufficient
            && estimate.spatial_coverage >= 0.42
            && estimate.inlier_ratio >= 0.24
            && estimate.error.is_finite()
            && estimate.error <= 4.0
    }
}

fn measurement_uncertainty(estimate: &Estimate) -> f64 {
    if !estimate.error.is_finite() {
        return f64::INFINITY;
    }
    let support_penalty = (12.0 / estimate.tracked_points.max(1) as f64)
        .sqrt()
        .max(1.0);
    let coverage_penalty = 1.0 / estimate.spatial_coverage.max(0.10).sqrt();
    (estimate.error.max(0.20) * support_penalty * coverage_penalty).clamp(0.20, 24.0)
}

#[derive(Clone, Copy)]
struct Estimate {
    matrix: Mat3,
    inlier_ratio: f64,
    error: f64,
    tracked_points: usize,
    spatial_coverage: f64,
    ecc: Option<f64>,
    source: &'static str,
    static_model: bool,
}

fn reacquisition_can_reseed_points(estimate: &Estimate) -> bool {
    measurement_is_credible(estimate)
        && estimate.tracked_points >= 24
        && estimate.spatial_coverage >= 0.60
        && estimate.inlier_ratio >= 0.45
        && estimate.error <= 2.0
        && matches!(estimate.source, "root" | "root-static")
}

impl Estimate {
    fn anchored(mut self, anchor: Mat3, source: &'static str) -> Self {
        self.matrix = self.matrix.multiply(anchor);
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
            // A fixed-reference match closes the drift loop. Prefer it whenever
            // it is independently credible; rolling anchors exist only to bridge
            // appearance changes where the fixed reference has insufficient
            // support. Comparing two small residuals and choosing the rolling one
            // allowed subpixel errors to accumulate into a visibly detached plane.
            if measurement_is_credible(&direct) {
                return Some(direct);
            }
            let local_score = local.error
                + (1.0 - local.inlier_ratio) * 3.0
                + continuity_penalty(local.matrix, predicted, plaque);
            let direct_score = direct.error
                + (1.0 - direct.inlier_ratio) * 3.0
                + continuity_penalty(direct.matrix, predicted, plaque);
            if direct_score <= local_score + 0.75 {
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

fn choose_persistent_estimate(
    persistent: Option<Estimate>,
    reacquired: Option<Estimate>,
    plaque: RectF,
    predicted: Mat3,
) -> Option<Estimate> {
    let persistent = persistent.filter(|estimate| {
        plaque_transform_is_valid(estimate.matrix, plaque) && measurement_is_credible(estimate)
    });
    let reacquired = reacquired.filter(|estimate| {
        plaque_transform_is_valid(estimate.matrix, plaque) && measurement_is_credible(estimate)
    });
    match (persistent, reacquired) {
        (Some(persistent), Some(reacquired)) => {
            let disagreement = mean_corner_distance(persistent.matrix, reacquired.matrix, plaque);
            // Optical flow preserves point identity from one frame to the next.
            // An independent detector is useful as a loop closure only when it
            // agrees with that identity-preserving path; otherwise a foreground
            // object's sharper texture must not be allowed to steal the plane.
            let reacquisition_score = reacquired.error
                + (1.0 - reacquired.inlier_ratio) * 3.0
                + continuity_penalty(reacquired.matrix, predicted, plaque);
            let persistent_score = persistent.error
                + (1.0 - persistent.inlier_ratio) * 3.0
                + continuity_penalty(persistent.matrix, predicted, plaque);
            let strong_root_loop_closure = matches!(reacquired.source, "root" | "root-static")
                && reacquired.tracked_points >= 32
                && reacquired.spatial_coverage >= 0.65
                && reacquired.inlier_ratio >= 0.55
                && reacquired.error <= 1.25;
            if strong_root_loop_closure
                || (disagreement <= 3.0 && reacquisition_score + 0.20 < persistent_score)
            {
                Some(reacquired)
            } else {
                Some(persistent)
            }
        }
        (Some(persistent), None) => Some(persistent),
        (None, Some(reacquired)) => Some(reacquired),
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
    if !(0.15..=6.0).contains(&area_ratio) {
        return false;
    }
    // A planar projective transform may taper opposite edges, but a near-frontal
    // title plaque cannot collapse one edge while expanding its opposite edge by
    // several times. Such bow-ties are sparse-match degeneracies, not perspective.
    let top = edge_length(quad.tl, quad.tr);
    let bottom = edge_length(quad.bl, quad.br);
    let left = edge_length(quad.tl, quad.bl);
    let right = edge_length(quad.tr, quad.br);
    edge_ratio(top, bottom) <= 1.55 && edge_ratio(left, right) <= 1.55
}

fn edge_length(a: GeoPoint, b: GeoPoint) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

fn edge_ratio(a: f64, b: f64) -> f64 {
    a.max(b) / a.min(b).max(1.0)
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
    // The material face is the tracked plane.  Restricting evidence to a thin
    // border discarded the richest stable texture and made foliage or background
    // just outside the plaque dominate the homography.
    let inset = (plaque.width.min(plaque.height) * 0.008).clamp(1.0, 4.0);
    let face = RectF {
        x: plaque.x + inset,
        y: plaque.y + inset,
        width: (plaque.width - inset * 2.0).max(1.0),
        height: (plaque.height - inset * 2.0).max(1.0),
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
        &polygon(face),
        Scalar::all(255.0),
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

/// Solves every interval without visual support from observations on both sides.
///
/// Internal gaps use cubic Hermite boundary conditions estimated from observations
/// before *and* after the gap. Consequently an approaching turn starts changing
/// velocity before the turn rather than waiting for a causal detector to notice it.
/// Short unsupported clip tails use a robust velocity from the nearest measured
/// poses. Longer tails are accepted only when the same four-corner velocity carries
/// the surface monotonically out of the viewport; an unsupported on-screen tail is
/// rejected instead of freezing or inventing motion.
fn solve_unobserved_intervals(
    samples: &mut [MotionSample],
    plaque: RectF,
    frame_width: u32,
    frame_height: u32,
) -> Result<usize> {
    let valid = samples
        .iter()
        .enumerate()
        .filter_map(|(frame, sample)| sample.measurement_valid.then_some(frame))
        .collect::<Vec<_>>();
    if valid.is_empty() {
        bail!("surface trajectory has no visual measurements");
    }

    let source = GeoQuad::from_rect(plaque.x, plaque.y, plaque.width, plaque.height);
    let measured = samples
        .iter()
        .map(|sample| transformed_plaque(plaque, sample.transform))
        .collect::<Vec<_>>();
    let mut inferred = 0usize;

    for frame in 0..samples.len() {
        if samples[frame].measurement_valid {
            continue;
        }
        let left_position = valid.partition_point(|candidate| *candidate < frame);
        let left = left_position.checked_sub(1).map(|index| valid[index]);
        let right = valid.get(left_position).copied();
        let (candidate, source_name) = match (left, right) {
            (Some(left), Some(right)) if left < right => {
                let previous = valid
                    .get(left_position.saturating_sub(2))
                    .copied()
                    .unwrap_or(left);
                let following = valid.get(left_position + 1).copied().unwrap_or(right);
                let t = (frame - left) as f64 / (right - left) as f64;
                let duration = (right - left) as f64;
                let hermite = hermite_quad(
                    measured[left],
                    measured[right],
                    quad_velocity(measured[previous], measured[left], previous, left),
                    quad_velocity(measured[right], measured[following], right, following),
                    t,
                    duration,
                );
                let candidate = if inferred_quad_is_physical(hermite, plaque) {
                    hermite
                } else {
                    // Boundary velocities can be contaminated immediately beside a
                    // long occlusion. The two measured endpoint poses are still
                    // authoritative; smoothstep interpolation keeps zero endpoint
                    // acceleration without allowing a concave four-corner path.
                    measured[left].lerp(measured[right], t * t * (3.0 - 2.0 * t))
                };
                (candidate, "bidirectional-temporal-solve")
            }
            (Some(left), None) => {
                let gap = frame - left;
                let previous = valid
                    .get(left_position.saturating_sub(2))
                    .copied()
                    .unwrap_or(left);
                let velocity = quad_velocity(measured[previous], measured[left], previous, left);
                let tail = samples.len() - 1 - left;
                if tail > 12
                    && !offscreen_tail_is_physical(
                        measured[left],
                        velocity,
                        tail as f64,
                        plaque,
                        frame_width,
                        frame_height,
                    )
                {
                    bail!(
                        "surface tracking loses all visual support after frame {left}; \
                         the unsupported {tail}-frame tail does not carry the surface out of view"
                    );
                }
                (
                    translate_quad(measured[left], velocity, gap as f64),
                    if tail > 12 {
                        "offscreen-trajectory-solve"
                    } else {
                        "one-sided-temporal-solve"
                    },
                )
            }
            (None, Some(right)) => {
                let gap = right - frame;
                let following = valid.get(1).copied().unwrap_or(right);
                let velocity =
                    quad_velocity(measured[right], measured[following], right, following);
                if right > 12
                    && !offscreen_tail_is_physical(
                        measured[right],
                        velocity,
                        -(right as f64),
                        plaque,
                        frame_width,
                        frame_height,
                    )
                {
                    bail!(
                        "surface tracking has no visual support before frame {right}; \
                         the unsupported {right}-frame lead-in does not originate out of view"
                    );
                }
                (
                    translate_quad(measured[right], velocity, -(gap as f64)),
                    if right > 12 {
                        "offscreen-trajectory-solve"
                    } else {
                        "one-sided-temporal-solve"
                    },
                )
            }
            _ => (measured[frame], "unobserved"),
        };
        candidate.validate("physically inferred surface pose")?;
        if candidate.orientation() <= 0.0 {
            bail!("physically inferred surface pose is mirrored at frame {frame}");
        }
        samples[frame].transform = Mat3 {
            values: homography(source, candidate)?.m,
        };
        samples[frame].measurement_valid = false;
        samples[frame].tracked_points = 0;
        samples[frame].spatial_coverage = 0.0;
        samples[frame].measurement_source = source_name.into();
        let support_distance = nearest_measurement_distance(frame, left, right) as f64;
        samples[frame].uncertainty_px = support_distance;
        // JSON has no representation for infinity. Keep the absence of evidence in
        // `measurement_valid`; use a finite conservative residual so every freshly
        // written trajectory can be reopened byte-for-byte.
        samples[frame].reprojection_error = support_distance.clamp(4.0, 24.0);
        samples[frame].ecc = None;
        inferred += 1;
    }
    Ok(inferred)
}

fn inferred_quad_is_physical(candidate: GeoQuad, plaque: RectF) -> bool {
    if candidate
        .validate("physically inferred surface pose")
        .is_err()
        || candidate.orientation() <= 0.0
    {
        return false;
    }
    let area_ratio = candidate.orientation() / (plaque.width * plaque.height).max(1.0);
    let top = edge_length(candidate.tl, candidate.tr);
    let bottom = edge_length(candidate.bl, candidate.br);
    let left = edge_length(candidate.tl, candidate.bl);
    let right = edge_length(candidate.tr, candidate.br);
    (0.15..=6.0).contains(&area_ratio)
        && edge_ratio(top, bottom) <= 1.55
        && edge_ratio(left, right) <= 1.55
}

fn offscreen_tail_is_physical(
    measured: GeoQuad,
    velocity: [[f64; 2]; 4],
    duration: f64,
    plaque: RectF,
    frame_width: u32,
    frame_height: u32,
) -> bool {
    let steps = duration.abs().round().max(1.0) as usize;
    let mut previous_fraction = surface_quad_visible_fraction(measured, frame_width, frame_height);
    for step in 1..=steps {
        let t = duration * step as f64 / steps as f64;
        let candidate = translate_quad(measured, velocity, t);
        let area_ratio = candidate.orientation() / (plaque.width * plaque.height).max(1.0);
        if candidate.validate("off-screen trajectory").is_err()
            || candidate.orientation() <= 0.0
            || !(0.15..=6.0).contains(&area_ratio)
        {
            return false;
        }
        let visible = surface_quad_visible_fraction(candidate, frame_width, frame_height);
        if visible > previous_fraction + 0.08 {
            return false;
        }
        previous_fraction = visible;
    }
    previous_fraction <= 0.06
}

pub(crate) fn surface_visible_fraction(
    plaque: RectF,
    transform: Mat3,
    frame_width: u32,
    frame_height: u32,
) -> f64 {
    surface_quad_visible_fraction(
        transformed_plaque(plaque, transform),
        frame_width,
        frame_height,
    )
}

fn surface_quad_visible_fraction(quad: GeoQuad, width: u32, height: u32) -> f64 {
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

fn nearest_measurement_distance(frame: usize, left: Option<usize>, right: Option<usize>) -> usize {
    left.map(|value| frame - value)
        .into_iter()
        .chain(right.map(|value| value - frame))
        .min()
        .unwrap_or(usize::MAX)
}

fn quad_velocity(a: GeoQuad, b: GeoQuad, a_frame: usize, b_frame: usize) -> [[f64; 2]; 4] {
    let duration = b_frame.abs_diff(a_frame).max(1) as f64;
    let a = a.points();
    let b = b.points();
    std::array::from_fn(|corner| {
        [
            (b[corner].x - a[corner].x) / duration,
            (b[corner].y - a[corner].y) / duration,
        ]
    })
}

fn hermite_quad(
    left: GeoQuad,
    right: GeoQuad,
    left_velocity: [[f64; 2]; 4],
    right_velocity: [[f64; 2]; 4],
    t: f64,
    duration: f64,
) -> GeoQuad {
    let left = left.points();
    let right = right.points();
    let t2 = t * t;
    let t3 = t2 * t;
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;
    let points: [GeoPoint; 4] = std::array::from_fn(|corner| {
        GeoPoint::new(
            h00 * left[corner].x
                + h10 * duration * left_velocity[corner][0]
                + h01 * right[corner].x
                + h11 * duration * right_velocity[corner][0],
            h00 * left[corner].y
                + h10 * duration * left_velocity[corner][1]
                + h01 * right[corner].y
                + h11 * duration * right_velocity[corner][1],
        )
    });
    GeoQuad::new(points[0], points[1], points[2], points[3])
}

fn translate_quad(quad: GeoQuad, velocity: [[f64; 2]; 4], duration: f64) -> GeoQuad {
    let points = quad.points();
    let moved: [GeoPoint; 4] = std::array::from_fn(|corner| {
        GeoPoint::new(
            points[corner].x + velocity[corner][0] * duration,
            points[corner].y + velocity[corner][1] * duration,
        )
    });
    GeoQuad::new(moved[0], moved[1], moved[2], moved[3])
}

pub(crate) fn mean_quad_distance(left: GeoQuad, right: GeoQuad) -> f64 {
    left.points()
        .into_iter()
        .zip(right.points())
        .map(|(a, b)| (a.x - b.x).hypot(a.y - b.y))
        .sum::<f64>()
        / 4.0
}

/// A four-corner, image-space dynamics measurement shared by analysis and
/// verification. Each residual is the distance between the observed pose and
/// the constant-velocity pose predicted from both neighboring frames. Looking
/// at all four corners detects perspective/rotation impulses that a center-only
/// test cannot see.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TrajectoryDynamics {
    pub(crate) temporal_score: f64,
    pub(crate) p95_residual: f64,
    pub(crate) maximum_residual: f64,
    pub(crate) worst_frame: usize,
    pub(crate) loop_score: f64,
    pub(crate) loop_residual: f64,
}

impl TrajectoryDynamics {
    fn is_physical(self) -> bool {
        self.temporal_score >= MINIMUM_PHYSICAL_TEMPORAL_SCORE
            && self.maximum_residual <= MAXIMUM_PHYSICAL_FRAME_RESIDUAL
    }
}

pub(crate) fn trajectory_dynamics(
    samples: &[MotionSample],
    plaque: RectF,
    loop_closed: bool,
) -> TrajectoryDynamics {
    if samples.len() < 3 {
        return TrajectoryDynamics {
            temporal_score: 0.0,
            p95_residual: f64::INFINITY,
            maximum_residual: f64::INFINITY,
            worst_frame: 0,
            loop_score: if loop_closed { 0.0 } else { 1.0 },
            loop_residual: if loop_closed { f64::INFINITY } else { 0.0 },
        };
    }

    let quads = samples
        .iter()
        .map(|sample| transformed_plaque(plaque, sample.transform))
        .collect::<Vec<_>>();
    let mut residuals = Vec::with_capacity(samples.len());
    let mut maximum_residual = 0.0_f64;
    let mut worst_frame = 0usize;
    let first = if loop_closed { 0 } else { 1 };
    let end = if loop_closed {
        samples.len()
    } else {
        samples.len() - 1
    };
    for frame in first..end {
        let previous = if frame == 0 {
            samples.len() - 1
        } else {
            frame - 1
        };
        let next = if frame + 1 == samples.len() {
            0
        } else {
            frame + 1
        };
        let mut residual = mean_quad_distance(quads[frame], quads[previous].lerp(quads[next], 0.5));

        // Visibility is part of the rendered title trajectory. Express abrupt
        // opacity curvature as a small pixel-equivalent penalty while leaving a
        // smooth authored fade unpenalized.
        let expected_visibility =
            (samples[previous].plaque_visibility + samples[next].plaque_visibility) * 0.5;
        residual =
            residual.hypot((samples[frame].plaque_visibility - expected_visibility).abs() * 8.0);
        if residual > maximum_residual {
            maximum_residual = residual;
            worst_frame = frame;
        }
        residuals.push((frame, residual));
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
    let mut residual_values = residuals
        .iter()
        .map(|(_, residual)| *residual)
        .collect::<Vec<_>>();
    let p95_residual = percentile(&mut residual_values, 0.95);
    let loop_residual = if loop_closed {
        residuals
            .iter()
            .filter(|(frame, _)| *frame == 0 || *frame + 1 == samples.len())
            .map(|(_, residual)| *residual)
            .fold(0.0, f64::max)
    } else {
        0.0
    };

    TrajectoryDynamics {
        temporal_score,
        p95_residual,
        maximum_residual,
        worst_frame,
        loop_score: if loop_closed {
            score_for(loop_residual)
        } else {
            1.0
        },
        loop_residual,
    }
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
    write_contact_sheet(&frames, &diagnostics.join("tracking-contact-sheet.png"))?;
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
    if reference_descriptors.empty()
        || reference_keypoints.len() < 8
        || descriptors.empty()
        || keypoints.len() < 8
    {
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
        let mut estimate = estimate_model(&source, &destination, model, coverage)?;
        estimate.inlier_ratio *= 0.55 + 0.45 * coverage;
        let complexity_penalty = match model {
            MotionModel::Similarity => 0.0,
            MotionModel::Affine => 0.08,
            MotionModel::Projective => 0.12,
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
    let width = (max_x - min_x).max(1.0);
    let height = (max_y - min_y).max(1.0);
    let mut occupied = [false; 9];
    let mut matched_min_x = f32::INFINITY;
    let mut matched_min_y = f32::INFINITY;
    let mut matched_max_x = f32::NEG_INFINITY;
    let mut matched_max_y = f32::NEG_INFINITY;
    for point in matched {
        let column = (((point.x - min_x) / width) * 3.0).floor().clamp(0.0, 2.0) as usize;
        let row = (((point.y - min_y) / height) * 3.0).floor().clamp(0.0, 2.0) as usize;
        let index = row * 3 + column;
        occupied[index] = true;
        matched_min_x = matched_min_x.min(point.x);
        matched_min_y = matched_min_y.min(point.y);
        matched_max_x = matched_max_x.max(point.x);
        matched_max_y = matched_max_y.max(point.y);
    }
    let sectors = occupied.into_iter().filter(|value| *value).count() as f64 / 9.0;
    let horizontal = f64::from((matched_max_x - matched_min_x) / width);
    let vertical = f64::from((matched_max_y - matched_min_y) / height);
    Ok((0.65 * sectors + 0.175 * horizontal.min(1.0) + 0.175 * vertical.min(1.0)).clamp(0.0, 1.0))
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
        tracked_points: source.len(),
        spatial_coverage: coverage,
        ecc: None,
        source: "static",
        static_model: true,
    }))
}

fn estimate_model(
    source: &Vector<Point2f>,
    destination: &Vector<Point2f>,
    model: MotionModel,
    coverage: f64,
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
            geometry::USAC_MAGSAC,
            2.25,
            &mut inliers,
            10_000,
            0.999,
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
    let inlier_coverage = correspondence_inlier_coverage(source, &inliers)?;

    Ok(Estimate {
        matrix,
        inlier_ratio,
        error,
        tracked_points: core::count_non_zero(&inliers).unwrap_or(0).max(0) as usize,
        // Coverage of all tentative matches is not evidence for the selected
        // robust model. A small foreground cluster can win MAGSAC while rejected
        // plaque matches make the tentative set look spatially excellent. Report
        // only the selected model's inlier distribution.
        spatial_coverage: coverage.min(inlier_coverage),
        ecc: None,
        source: "feature",
        static_model: false,
    })
}

fn correspondence_inlier_coverage(source: &Vector<Point2f>, inliers: &Mat) -> Result<f64> {
    if source.is_empty() || inliers.empty() {
        return Ok(0.0);
    }
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for point in source {
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }
    let width = (max_x - min_x).max(1.0);
    let height = (max_y - min_y).max(1.0);
    let mut occupied = [false; 12];
    let mut selected_min_x = f32::INFINITY;
    let mut selected_min_y = f32::INFINITY;
    let mut selected_max_x = f32::NEG_INFINITY;
    let mut selected_max_y = f32::NEG_INFINITY;
    let mut selected_count = 0usize;
    for index in 0..source.len() {
        let is_inlier = if inliers.rows() as usize == source.len() {
            *inliers.at_2d::<u8>(index as i32, 0)? != 0
        } else if inliers.cols() as usize == source.len() {
            *inliers.at_2d::<u8>(0, index as i32)? != 0
        } else {
            false
        };
        if !is_inlier {
            continue;
        }
        let point = source.get(index)?;
        let column = (((point.x - min_x) / width) * 4.0).floor().clamp(0.0, 3.0) as usize;
        let row = (((point.y - min_y) / height) * 3.0).floor().clamp(0.0, 2.0) as usize;
        occupied[row * 4 + column] = true;
        selected_min_x = selected_min_x.min(point.x);
        selected_min_y = selected_min_y.min(point.y);
        selected_max_x = selected_max_x.max(point.x);
        selected_max_y = selected_max_y.max(point.y);
        selected_count += 1;
    }
    if selected_count == 0 {
        return Ok(0.0);
    }
    let sectors = occupied.into_iter().filter(|value| *value).count() as f64 / 12.0;
    let horizontal = f64::from((selected_max_x - selected_min_x) / width).clamp(0.0, 1.0);
    let vertical = f64::from((selected_max_y - selected_min_y) / height).clamp(0.0, 1.0);
    Ok((0.65 * sectors + 0.175 * horizontal + 0.175 * vertical).clamp(0.0, 1.0))
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

/// Rejects physically impossible one-frame pose impulses before the noncausal
/// interpolation pass.
///
/// Feature estimates with strong, spatially distributed inliers may legitimately
/// disagree with a short-window median during fast motion. Low-support estimates
/// do not get that exemption and are bridged from evidence on both sides.
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
            for (offset, sample) in samples[first..=last].iter_mut().enumerate() {
                sample.measurement_valid = false;
                sample.measurement_source = "rejected-outlier".into();
                sample.uncertainty_px = sample.uncertainty_px.max(bad[group_start + offset].1);
                sample.inlier_ratio = 0.0;
                sample.ecc = None;
            }
        }
        cursor += 1;
    }

    bad.len()
}

fn should_close_loop(capture: &mut VideoCapture, frame_count: usize) -> Result<bool> {
    // Loop closure is an optimization hint, never a prerequisite for tracking. Some
    // OpenCV/codec combinations report N frames but cannot seek-decode exactly N-1.
    // Search backward for a decodable tail frame instead of aborting the whole analysis
    // on that optional heuristic. The video probe normally removes stale metadata tails;
    // this remains a defensive fallback for codec-specific seek behavior.
    let first = match read_gray(capture, 0) {
        Ok(frame) => frame,
        Err(error) => {
            eprintln!(
                "warning: loop detection disabled because frame 0 could not be decoded: {error:#}"
            );
            return Ok(false);
        }
    };
    let nominal_last = frame_count.saturating_sub(1);
    let mut decoded_last = None;
    let tail_window = frame_count.min(256);
    for offset in 0..tail_window {
        let index = nominal_last.saturating_sub(offset);
        if let Ok(frame) = read_gray(capture, index) {
            decoded_last = Some((index, frame));
            break;
        }
    }
    let Some((actual_last, last)) = decoded_last else {
        eprintln!(
            "warning: loop detection disabled because none of the final {tail_window} frame positions could be seek-decoded"
        );
        return Ok(false);
    };
    if actual_last != nominal_last {
        eprintln!(
            "warning: loop detection used frame {actual_last} because nominal final frame {nominal_last} was not seek-decodable"
        );
    }
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
        SourceFlowConsistency, TrackingResult, apply_motion_scene, apply_visibility_scenes,
        measure_source_flow_consistency, optimize_trajectory, oriented_rect_quad,
        plaque_feature_mask, plaque_transform_is_valid, reapply_locked_scenes, select_masked_scene,
        solve_unobserved_intervals, static_estimate, surface_visible_fraction, trajectory_dynamics,
        trajectory_loop_closed, transformed_quad,
    };
    use crate::{
        color::Rgba,
        geometry::Quad,
        model::{Mat3, MotionSample, PointF, RectF},
        scene::{CoordinateSystem, MotionKeyframe, SurfaceTrajectory, TRAJECTORY_FORMAT},
        surface::Surface,
    };
    use opencv::{
        core::{Point2f, Vector},
        prelude::MatTraitConst,
    };

    #[test]
    fn masked_retracking_cannot_replace_a_more_confident_track() {
        let result = select_masked_scene(
            tracking_result("baseline", 0.82, 1.0),
            tracking_result("masked", 0.48, 9.0),
        );

        assert_eq!(result.confidence, 0.82);
        assert_eq!(result.samples[0].transform.values[0][2], 1.0);
        assert!(result.model_name.ends_with("-masked-retrack-rejected"));
    }

    #[test]
    fn masked_retracking_replaces_a_weaker_track() {
        let result = select_masked_scene(
            tracking_result("baseline", 0.48, 1.0),
            tracking_result("masked", 0.82, 9.0),
        );

        assert_eq!(result.confidence, 0.82);
        assert_eq!(result.samples[0].transform.values[0][2], 9.0);
        assert_eq!(result.model_name, "masked");
    }

    #[test]
    fn four_corner_dynamics_exposes_a_projective_impulse() {
        let plaque = RectF {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 50.0,
        };
        let mut samples = (0..9)
            .map(|frame| MotionSample {
                frame,
                transform: Mat3::translation(frame as f64, 0.0),
                measurement_valid: true,
                plaque_visibility: 1.0,
                ..MotionSample::default()
            })
            .collect::<Vec<_>>();
        samples[4].transform.values[0][1] = 0.12;

        let dynamics = trajectory_dynamics(&samples, plaque, false);
        assert_eq!(dynamics.worst_frame, 4);
        assert!(dynamics.maximum_residual > 5.0);
        assert!(dynamics.temporal_score < 0.90);
    }

    fn tracking_result(model: &str, confidence: f64, translation: f64) -> TrackingResult {
        TrackingResult {
            samples: vec![MotionSample {
                frame: 0,
                transform: Mat3 {
                    values: [[1.0, 0.0, translation], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                },
                measurement_valid: true,
                tracked_points: 20,
                spatial_coverage: 1.0,
                uncertainty_px: 0.25,
                measurement_source: "test".into(),
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
    fn plaque_feature_mask_uses_the_material_face_not_background() {
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
        assert_eq!(*mask.at_2d::<u8>(82, 102).unwrap(), 255);
        assert_eq!(*mask.at_2d::<u8>(130, 200).unwrap(), 255);
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
    fn independent_source_flow_rejects_a_screen_fixed_trajectory() {
        let width = 192;
        let height = 128;
        let plaque = RectF {
            x: 32.0,
            y: 24.0,
            width: 112.0,
            height: 72.0,
        };
        let mut previous = Surface::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let checker = ((x / 13 + y / 11) % 2) as u8;
                let detail = ((x * 19 + y * 31 + x * y * 3) % 61) as u8;
                let value = 45 + checker * 120 + detail;
                previous.set_pixel(x, y, Rgba::new(value, 255 - value / 2, value / 2, 255));
            }
        }
        let dx = 4_u32;
        let dy = 3_u32;
        let shift = Mat3::translation(f64::from(dx), f64::from(dy));
        let mut current = Surface::new(width, height);
        for y in 0..height - dy {
            for x in 0..width - dx {
                current.set_pixel(x + dx, y + dy, previous.pixel(x, y));
            }
        }

        let correct = measure_source_flow_consistency(
            &previous,
            &current,
            plaque,
            Mat3::IDENTITY,
            shift,
            None,
            None,
        )
        .unwrap()
        .unwrap();
        let screen_fixed = measure_source_flow_consistency(
            &previous,
            &current,
            plaque,
            Mat3::IDENTITY,
            Mat3::IDENTITY,
            None,
            None,
        )
        .unwrap()
        .unwrap();

        assert!(
            correct.median_error_pixels < 0.75,
            "correct trajectory error was {}px",
            correct.median_error_pixels
        );
        assert!(correct.inlier_fraction > 0.80);
        assert!(
            screen_fixed.median_error_pixels > correct.median_error_pixels + 3.5,
            "screen-fixed error {}px did not separate from correct error {}px",
            screen_fixed.median_error_pixels,
            correct.median_error_pixels
        );
    }

    #[test]
    fn source_flow_scores_the_fitted_plane_not_nonrigid_endpoint_scatter() {
        let plaque = RectF {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        };
        let shift = Mat3::translation(5.0, -2.0);
        let correspondences = (0_u32..32)
            .map(|index| {
                let source = PointF {
                    x: 10.0 + f64::from(index % 8) * 10.0,
                    y: 8.0 + f64::from(index / 8) * 10.0,
                };
                let expected = shift.transform(source);
                let endpoint_noise = if index.is_multiple_of(2) { 1.25 } else { -1.25 };
                (
                    source,
                    PointF {
                        x: expected.x + endpoint_noise,
                        y: expected.y - endpoint_noise * 0.5,
                    },
                )
            })
            .collect();
        let observation = SourceFlowConsistency {
            median_error_pixels: 0.0,
            inlier_fraction: 1.0,
            tracked_points: 32,
            spatial_coverage: 1.0,
            flow_model_inlier_fraction: 1.0,
            material_transform: shift,
            flow_model_error_pixels: 1.4,
            correspondences,
        };

        let correct = observation.error_for_poses(plaque, Mat3::IDENTITY, shift);
        let screen_fixed = observation.error_for_poses(plaque, Mat3::IDENTITY, Mat3::IDENTITY);
        assert!(correct < 1.0e-9, "fitted plane residual was {correct}px");
        assert!(
            screen_fixed > 5.0,
            "screen-fixed residual was {screen_fixed}px"
        );
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
                measurement_valid: frame != 4,
                inlier_ratio: if frame == 4 { 0.0 } else { 1.0 },
                reprojection_error: if frame == 4 { 12.0 } else { 0.0 },
                measurement_source: "test".into(),
                ..MotionSample::default()
            })
            .collect::<Vec<_>>();

        optimize_trajectory(&mut samples, plaque, 0, 0.35, false).unwrap();

        assert!(samples[4].transform.values[0][2] < 5.0);
        assert_eq!(samples[0].transform.values, Mat3::IDENTITY.values);
    }

    #[test]
    fn bidirectional_solver_uses_future_motion_without_relabelling_it_as_evidence() {
        let plaque = RectF {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        };
        let measured = |frame, x| MotionSample {
            frame,
            transform: Mat3::translation(x, 0.0),
            measurement_valid: true,
            tracked_points: 24,
            spatial_coverage: 0.9,
            uncertainty_px: 0.5,
            measurement_source: "test-measurement".into(),
            inlier_ratio: 0.9,
            reprojection_error: 0.5,
            ..MotionSample::default()
        };
        let mut samples = vec![
            measured(0, 0.0),
            measured(1, 0.0),
            MotionSample {
                frame: 2,
                ..MotionSample::default()
            },
            MotionSample {
                frame: 3,
                ..MotionSample::default()
            },
            measured(4, 8.0),
            measured(5, 14.0),
        ];

        assert_eq!(
            solve_unobserved_intervals(&mut samples, plaque, 1920, 1080).unwrap(),
            2
        );

        assert!(samples[2].transform.values[0][2] > 0.0);
        assert!(samples[3].transform.values[0][2] > samples[2].transform.values[0][2]);
        assert!(!samples[2].measurement_valid);
        assert_eq!(
            samples[2].measurement_source,
            "bidirectional-temporal-solve"
        );
    }

    #[test]
    fn long_unsupported_tail_is_allowed_only_when_motion_carries_surface_offscreen() {
        let plaque = RectF {
            x: 80.0,
            y: 40.0,
            width: 100.0,
            height: 50.0,
        };
        let measured = |frame, x| MotionSample {
            frame,
            transform: Mat3::translation(x, 0.0),
            measurement_valid: true,
            tracked_points: 24,
            spatial_coverage: 0.9,
            uncertainty_px: 0.5,
            measurement_source: "test-measurement".into(),
            inlier_ratio: 0.9,
            reprojection_error: 0.5,
            ..MotionSample::default()
        };
        let mut exiting = (0..30)
            .map(|frame| MotionSample {
                frame,
                ..MotionSample::default()
            })
            .collect::<Vec<_>>();
        exiting[0] = measured(0, 0.0);
        exiting[1] = measured(1, 12.0);
        assert_eq!(
            solve_unobserved_intervals(&mut exiting, plaque, 240, 160).unwrap(),
            28
        );
        assert_eq!(exiting[29].measurement_source, "offscreen-trajectory-solve");
        assert!(surface_visible_fraction(plaque, exiting[29].transform, 240, 160) <= 0.06);

        let mut frozen = (0..30)
            .map(|frame| MotionSample {
                frame,
                ..MotionSample::default()
            })
            .collect::<Vec<_>>();
        frozen[0] = measured(0, 0.0);
        frozen[1] = measured(1, 0.0);
        assert!(solve_unobserved_intervals(&mut frozen, plaque, 240, 160).is_err());
    }

    #[test]
    fn sparse_reviewed_keyframe_constrains_an_all_frame_track() {
        let plaque = RectF {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 50.0,
        };
        let sample = |frame| MotionSample {
            frame,
            transform: Mat3::IDENTITY,
            measurement_valid: true,
            inlier_ratio: 0.8,
            reprojection_error: 0.5,
            ..MotionSample::default()
        };
        let mut result = TrackingResult {
            samples: (0..3).map(sample).collect(),
            model_name: "automatic-inertia-0.35".into(),
            reference_frame: 0,
            confidence: 0.8,
            loop_closed: false,
        };
        let track = SurfaceTrajectory {
            format: TRAJECTORY_FORMAT.into(),
            surface: "main".into(),
            coordinates: CoordinateSystem::SourcePixels,
            source_sha256: None,
            keyframes: vec![MotionKeyframe {
                frame: 1,
                quad: [[15.0, 20.0], [115.0, 20.0], [115.0, 70.0], [15.0, 70.0]],
                locked: true,
                visibility: None,
            }],
        };

        apply_motion_scene(&mut result, &track, plaque).unwrap();

        assert_eq!(result.samples.len(), 3);
        let source = Quad::from_rect(plaque.x, plaque.y, plaque.width, plaque.height);
        let constrained = transformed_quad(source, result.samples[1].transform);
        assert!((constrained.tl.x - 15.0).abs() < 1.0e-9);
        assert!(
            result
                .model_name
                .starts_with("reviewed-constrained-quad-track-")
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
                    measurement_valid: true,
                    inlier_ratio: 0.5,
                    reprojection_error: 1.0,
                    ..MotionSample::default()
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
        let track = SurfaceTrajectory {
            format: TRAJECTORY_FORMAT.into(),
            surface: "main".into(),
            coordinates: CoordinateSystem::SourcePixels,
            source_sha256: None,
            keyframes: vec![keyframe(0), keyframe(1)],
        };

        apply_motion_scene(&mut result, &track, plaque).unwrap();

        assert!(result.model_name.starts_with("reviewed-dense-quad-track-"));
        assert_eq!(result.confidence, 0.99);
    }

    #[test]
    fn mixed_track_reapplies_only_locked_samples_after_scene() {
        let plaque = RectF {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 5.0,
        };
        let sample = |frame| MotionSample {
            frame,
            transform: Mat3::IDENTITY,
            measurement_valid: true,
            inlier_ratio: 0.8,
            reprojection_error: 0.5,
            ..MotionSample::default()
        };
        let track = SurfaceTrajectory {
            format: TRAJECTORY_FORMAT.into(),
            surface: "main".into(),
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

        apply_motion_scene(&mut result, &track, plaque).unwrap();
        assert!(result.model_name.starts_with("reviewed-mixed-quad-track-"));
        result.samples.iter_mut().for_each(|sample| {
            sample.transform = Mat3::IDENTITY;
        });
        reapply_locked_scenes(&mut result.samples, &track, plaque).unwrap();

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
            measurement_valid: true,
            inlier_ratio: 0.8,
            reprojection_error: 0.5,
            plaque_visibility,
            ..MotionSample::default()
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
        let track = SurfaceTrajectory {
            format: TRAJECTORY_FORMAT.into(),
            surface: "main".into(),
            coordinates: CoordinateSystem::SourcePixels,
            source_sha256: None,
            keyframes: vec![
                keyframe(1, Some(0.3)),
                keyframe(2, None),
                keyframe(3, Some(0.4)),
            ],
        };

        apply_visibility_scenes(&mut samples, &track).unwrap();

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
            measurement_valid: true,
            inlier_ratio: 1.0,
            reprojection_error: 0.0,
            ..MotionSample::default()
        };
        let open_samples = vec![sample(0, 0.0), sample(1, 10.0)];
        let closed_samples = vec![sample(0, 0.0), sample(1, 0.0)];

        assert!(!trajectory_loop_closed(&open_samples, plaque));
        assert!(trajectory_loop_closed(&closed_samples, plaque));
    }
}
