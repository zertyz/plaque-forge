//! Plaque motion tracking across the source video.
//!
//! A tracker estimates a projective transform for each frame, then stabilizes that
//! trajectory and applies any locked or guiding motion scenes.

use std::path::Path;

use anyhow::{Context, Result, bail};
use opencv::{
    core::{self, Scalar, Vector},
    features2d,
    prelude::*,
    videoio::{CAP_PROP_FRAME_COUNT, CAP_PROP_POS_FRAMES, VideoCapture},
};

use crate::{
    application::AnalyzeRequest,
    geometry::{Quad as GeoQuad, homography},
    model::{Mat3, MotionSample, RectF},
    progress::ProgressReporter,
    scene::SurfaceTrajectory,
    video::VideoInfo,
};

pub mod constraints;
pub mod features;
pub mod solver;
pub mod trajectory;
pub mod types;

pub use constraints::*;
pub(crate) use features::*;
pub(crate) use solver::*;
pub(crate) use trajectory::*;
pub use types::*;

/// Keeps the strongest absolute pose measurements, then refines that same global
/// trajectory using the final foreground-aware source flow. A fully masked
/// retrack is retained only when its own absolute evidence is genuinely stronger;
/// sparse masked frames must not discard a well-rooted baseline.
#[allow(clippy::too_many_arguments)]
pub fn refine_scene_with_masked_flow(
    args: &AnalyzeRequest,
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
    args: &AnalyzeRequest,
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
    args: &AnalyzeRequest,
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
        &mut selected,
        exclusion_root,
        progress,
    )?;

    if summary.observations >= 3 && summary.confidence >= 0.20 {
        selected.model_name = format!("{}+{model_suffix}", selected.model_name);
        progress.finish(format!(
            "trajectory refined with {} source-flow observations (median {:.2}px, p95 {:.2}px, confidence {:.2})",
            summary.observations, summary.median_error, summary.p95_error, summary.confidence
        ));
    } else {
        progress.finish(
            "source-flow refinement retained baseline trajectory due to insufficient reliable optical flow",
        );
    }
    Ok(selected)
}

#[allow(clippy::too_many_arguments)]
pub fn load_dense_scene(
    _args: &AnalyzeRequest,
    info: &VideoInfo,
    plaque: RectF,
    trajectory: &SurfaceTrajectory,
    _diagnostics: &Path,
    _progress: &mut ProgressReporter,
) -> Result<TrackingResult> {
    let mut samples = Vec::with_capacity(info.frames);
    let source = GeoQuad::from_rect(plaque.x, plaque.y, plaque.width, plaque.height);
    for frame in 0..info.frames {
        let keyframe = trajectory
            .keyframes
            .iter()
            .find(|keyframe| keyframe.frame == frame)
            .with_context(|| {
                format!(
                    "dense scene trajectory missing frame {frame} (expected 0..{})",
                    info.frames
                )
            })?;
        let quad = scene_quad(keyframe.quad);
        quad.validate("dense scene keyframe")?;
        let matrix = homography(source, quad)?;
        samples.push(MotionSample {
            frame,
            transform: Mat3 { values: matrix.m },
            measurement_valid: true,
            tracked_points: 4,
            spatial_coverage: 1.0,
            uncertainty_px: 0.1,
            measurement_source: "reviewed-dense-quad".into(),
            inlier_ratio: 1.0,
            reprojection_error: 0.0,
            ecc: Some(1.0),
            plaque_visibility: keyframe.visibility.unwrap_or(1.0).clamp(0.0, 1.0),
            occluder_coverage: 0.0,
        });
    }

    Ok(TrackingResult {
        samples,
        model_name: format!("reviewed-dense-quad-track-{}-frames", info.frames),
        screen_fixed: false,
        reference_frame: 0,
        confidence: 0.99,
        loop_closed: trajectory_loop_closed(
            &[
                MotionSample {
                    frame: 0,
                    transform: Mat3 {
                        values: homography(source, scene_quad(trajectory.keyframes[0].quad))?.m,
                    },
                    ..MotionSample::default()
                },
                MotionSample {
                    frame: info.frames - 1,
                    transform: Mat3 {
                        values: homography(
                            source,
                            scene_quad(trajectory.keyframes[info.frames - 1].quad),
                        )?
                        .m,
                    },
                    ..MotionSample::default()
                },
            ],
            plaque,
        ),
    })
}

#[allow(clippy::too_many_arguments)]
pub fn track(
    args: &AnalyzeRequest,
    info: &VideoInfo,
    plaque: RectF,
    reference_frame: usize,
    diagnostics: &Path,
    progress: &mut ProgressReporter,
    occluder_masks: Option<&Path>,
) -> Result<TrackingResult> {
    track_with_exclusions(
        args,
        info,
        plaque,
        reference_frame,
        diagnostics,
        progress,
        occluder_masks,
    )
}

pub fn retrack_masked(
    args: &AnalyzeRequest,
    info: &VideoInfo,
    plaque: RectF,
    reference_frame: usize,
    diagnostics: &Path,
    progress: &mut ProgressReporter,
    occluder_masks: &Path,
) -> Result<TrackingResult> {
    let result = track_with_exclusions(
        args,
        info,
        plaque,
        reference_frame,
        diagnostics,
        progress,
        Some(occluder_masks),
    )?;
    Ok(TrackingResult {
        model_name: format!("{}-masked-retrack", result.model_name),
        ..result
    })
}

#[allow(clippy::too_many_arguments)]
fn track_with_exclusions(
    args: &AnalyzeRequest,
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

    progress.finish(format!(
        "{adaptive_anchor_count} adaptive anchors, {inferred_frames} inferred, {repaired_frames} rejected measurements"
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
            "bidirectional-persistent-point-homography-sift-{:?}-regularization-{:.2}-global-source-flow",
            MotionModel::Adaptive,
            args.tracking_inertia
        )
        .to_lowercase(),
        screen_fixed: false,
        reference_frame,
        confidence,
        loop_closed,
    })
}

#[allow(clippy::too_many_arguments)]
fn process_direction(
    args: &AnalyzeRequest,
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
        let mut keypoints = Vector::<core::KeyPoint>::new();
        let mut descriptors = opencv::core::Mat::default();
        let mut search_mask = opencv::core::Mat::new_rows_cols_with_default(
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

#[cfg(test)]
mod tests {
    use super::{
        SourceFlowConsistency, TrackingResult, apply_motion_scene, apply_visibility_scenes,
        constraints::transformed_quad, measure_source_flow_consistency, optimize_trajectory,
        oriented_rect_quad, plaque_transform_is_valid, reapply_locked_scenes, select_masked_scene,
        solve_unobserved_intervals, static_estimate, surface_visible_fraction, trajectory_dynamics,
        trajectory_loop_closed,
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
        prelude::*,
    };

    fn plaque_feature_mask(
        width: i32,
        height: i32,
        plaque: RectF,
    ) -> anyhow::Result<opencv::core::Mat> {
        super::features::plaque_feature_mask_for_transform(width, height, plaque, Mat3::IDENTITY)
    }

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
            screen_fixed: model.contains("screen-fixed"),
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
            screen_fixed: false,
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
            screen_fixed: false,
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
            screen_fixed: false,
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
