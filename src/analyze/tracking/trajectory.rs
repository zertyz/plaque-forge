use std::path::Path;

use anyhow::{Context, Result, bail};
use opencv::{
    core::{
        self, AlgorithmHint, Mat, Point2f, Scalar, Size, TermCriteria, TermCriteria_Type, Vector,
    },
    features, imgcodecs, imgproc,
    prelude::*,
    video as cv_video,
};

use crate::{
    application::AnalyzeRequest,
    geometry::{Point as GeoPoint, Quad as GeoQuad, homography},
    model::{Mat3, MotionSample, PointF, RectF},
    progress::ProgressReporter,
    stats::percentile,
    surface::Surface,
    video::VideoInfo,
};

use super::{
    features::{
        byte_point_is_excluded, edge_length, edge_ratio, plaque_feature_mask_for_transform,
        transformed_plaque,
    },
    solver::{estimate_model, median, sample_confidence},
    types::{MotionModel, TrackingResult},
};

const MINIMUM_PHYSICAL_TEMPORAL_SCORE: f64 = 0.95;
const MAXIMUM_PHYSICAL_FRAME_RESIDUAL: f64 = 4.0;

#[derive(Debug, Clone)]
pub(crate) struct SourceFlowConsistency {
    pub median_error_pixels: f64,
    pub inlier_fraction: f64,
    pub tracked_points: usize,
    pub spatial_coverage: f64,
    pub flow_model_inlier_fraction: f64,
    pub material_transform: Mat3,
    pub(crate) flow_model_error_pixels: f64,
    pub(crate) correspondences: Vec<(PointF, PointF)>,
}

impl SourceFlowConsistency {
    pub(crate) fn error_for_poses(
        &self,
        plaque: RectF,
        previous_pose: Mat3,
        current_pose: Mat3,
    ) -> f64 {
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
            let independently_observed = self.material_transform.transform(source);
            errors.push(
                (predicted.x - independently_observed.x)
                    .hypot(predicted.y - independently_observed.y),
            );
        }
        median(errors)
    }
}

pub(crate) fn surface_luma_mat(surface: &Surface) -> Result<Mat> {
    let width = surface.width() as i32;
    let height = surface.height() as i32;
    let mut rgba = Mat::new_rows_cols_with_default(height, width, core::CV_8UC4, Scalar::all(0.0))?;
    rgba.data_bytes_mut()?.copy_from_slice(surface.pixels());
    let mut gray = Mat::default();
    imgproc::cvt_color(
        &rgba,
        &mut gray,
        imgproc::COLOR_RGBA2GRAY,
        0,
        AlgorithmHint::ALGO_HINT_DEFAULT,
    )?;
    Ok(gray)
}

pub(crate) fn apply_byte_exclusion(
    mask: &mut Mat,
    exclusion: Option<&[u8]>,
    width: i32,
    height: i32,
) -> Result<()> {
    let Some(exclusion) = exclusion else {
        return Ok(());
    };
    if let Ok(bytes) = mask.data_bytes_mut() {
        for (dst, &exc) in bytes.iter_mut().zip(exclusion.iter()) {
            if exc > 0 {
                *dst = 0;
            }
        }
    } else {
        for y in 0..height {
            for x in 0..width {
                let offset = (y * width + x) as usize;
                if exclusion.get(offset).copied().unwrap_or(0) > 0 {
                    *mask.at_2d_mut::<u8>(y, x)? = 0;
                }
            }
        }
    }
    Ok(())
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
        errors.push(
            (predicted.x - independently_observed.x).hypot(predicted.y - independently_observed.y),
        );
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
    if correspondences.len() < 12 {
        return Ok(None);
    }
    let inliers = errors.iter().filter(|error| **error <= 1.5).count() as f64 / errors.len() as f64;
    let median_error_pixels = median(errors);

    Ok(Some(SourceFlowConsistency {
        median_error_pixels,
        inlier_fraction: inliers,
        tracked_points: correspondences.len(),
        spatial_coverage: robust.spatial_coverage,
        flow_model_inlier_fraction: robust.inlier_ratio,
        material_transform: robust.matrix,
        flow_model_error_pixels: robust.error,
        correspondences,
    }))
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SourceFlowConstraintSummary {
    pub(crate) observations: usize,
    pub(crate) median_error: f64,
    pub(crate) p95_error: f64,
    #[allow(dead_code)]
    pub(crate) p99_error: f64,
    pub(crate) confidence: f64,
}

impl SourceFlowConstraintSummary {
    pub(crate) fn not_evaluated() -> Self {
        Self {
            observations: 0,
            median_error: f64::INFINITY,
            p95_error: f64::INFINITY,
            p99_error: f64::INFINITY,
            confidence: 0.0,
        }
    }
}

pub(crate) fn constrain_trajectory_to_source_flow(
    args: &AnalyzeRequest,
    info: &VideoInfo,
    plaque: RectF,
    result: &mut TrackingResult,
    exclusion_root: &Path,
    progress: &mut ProgressReporter,
) -> Result<SourceFlowConstraintSummary> {
    if result.samples.len() < 3 {
        return Ok(SourceFlowConstraintSummary::not_evaluated());
    }

    progress.start(
        6,
        7,
        "Refining trajectory with noncausal optical flow",
        Some(result.samples.len()),
    );
    let observations =
        collect_source_flow_observations(args, info, plaque, result, exclusion_root, progress)?;
    let usable_observations = observations
        .iter()
        .filter(|obs| source_flow_observation_is_usable(obs))
        .count();
    if usable_observations < 3 {
        return Ok(SourceFlowConstraintSummary::not_evaluated());
    }

    let source = GeoQuad::from_rect(plaque.x, plaque.y, plaque.width, plaque.height);
    let current_quads: Vec<GeoQuad> = result
        .samples
        .iter()
        .map(|sample| transformed_plaque(plaque, sample.transform))
        .collect();
    let current_errors = source_flow_errors_for_trajectory(&current_quads, plaque, &observations);
    let (mut best_median, mut best_p95) = source_flow_candidate_quality(current_errors.clone());
    let mut best_dynamics = trajectory_dynamics(&result.samples, plaque, result.loop_closed);
    let mut best_objective = trajectory_candidate_objective(best_median, best_p95, best_dynamics);
    let mut best_quads = current_quads.clone();
    let mut best_confidence = result.confidence;

    for inertia in [0.0_f64, 0.08, 0.16, 0.28, 0.42] {
        let candidate_quads =
            solve_global_source_flow_quads_with_inertia(source, result, &observations, inertia)?;
        let mut candidate_samples = result.samples.clone();
        assign_quads(&mut candidate_samples, source, &candidate_quads)?;
        let candidate_dynamics =
            trajectory_dynamics(&candidate_samples, plaque, result.loop_closed);
        let candidate_errors =
            source_flow_errors_for_trajectory(&candidate_quads, plaque, &observations);
        let (candidate_median, candidate_p95) =
            source_flow_candidate_quality(candidate_errors.clone());
        let candidate_objective =
            trajectory_candidate_objective(candidate_median, candidate_p95, candidate_dynamics);
        if trajectory_candidate_is_better(
            candidate_objective,
            candidate_median,
            candidate_p95,
            candidate_dynamics,
            best_objective,
            best_median,
            best_p95,
            best_dynamics,
        ) {
            best_objective = candidate_objective;
            best_median = candidate_median;
            best_p95 = candidate_p95;
            best_dynamics = candidate_dynamics;
            best_quads = candidate_quads;
            let mut sorted = candidate_errors;
            let p99 = percentile(&mut sorted, 0.99);
            best_confidence = (result.confidence * 0.40
                + source_flow_confidence(best_p95, p99) * 0.60)
                .clamp(0.10, 0.99);
        }
    }

    assign_quads(&mut result.samples, source, &best_quads)?;
    result.confidence = best_confidence;
    let mut final_errors = source_flow_errors_for_trajectory(&best_quads, plaque, &observations);
    let median_error = percentile(&mut final_errors, 0.50);
    let p95_error = percentile(&mut final_errors, 0.95);
    let p99_error = percentile(&mut final_errors, 0.99);
    Ok(SourceFlowConstraintSummary {
        observations: usable_observations,
        median_error,
        p95_error,
        p99_error,
        confidence: best_confidence,
    })
}

pub(crate) fn collect_source_flow_observations(
    args: &AnalyzeRequest,
    info: &VideoInfo,
    plaque: RectF,
    result: &TrackingResult,
    exclusion_root: &Path,
    progress: &mut ProgressReporter,
) -> Result<Vec<SourceFlowConsistency>> {
    let mut observations = Vec::with_capacity(result.samples.len().saturating_sub(1));
    let mut decoder = crate::video::Decoder::spawn(&args.ffmpeg, &args.input, info)?;
    let mut previous_surface = match decoder.next_frame()? {
        Some(surface) => surface,
        None => return Ok(observations),
    };
    let mut previous_exclusion = load_exclusion_bytes(exclusion_root, 0, info.width, info.height)?;

    for current_index in 1..result.samples.len() {
        let current_surface = match decoder.next_frame()? {
            Some(surface) => surface,
            None => break,
        };
        let current_exclusion =
            load_exclusion_bytes(exclusion_root, current_index, info.width, info.height)?;
        let previous_pose = result.samples[current_index - 1].transform;
        let current_pose = result.samples[current_index].transform;

        if let Some(obs) = measure_source_flow_consistency(
            &previous_surface,
            &current_surface,
            plaque,
            previous_pose,
            current_pose,
            previous_exclusion.as_deref(),
            current_exclusion.as_deref(),
        )? {
            observations.push(obs);
        } else {
            let fallback_transform = if let Some(prev_inv) = previous_pose.inverse() {
                current_pose.multiply(prev_inv)
            } else {
                Mat3::IDENTITY
            };
            observations.push(SourceFlowConsistency {
                median_error_pixels: f64::INFINITY,
                inlier_fraction: 0.0,
                tracked_points: 0,
                spatial_coverage: 0.0,
                flow_model_inlier_fraction: 0.0,
                material_transform: fallback_transform,
                flow_model_error_pixels: f64::INFINITY,
                correspondences: Vec::new(),
            });
        }
        previous_surface = current_surface;
        previous_exclusion = current_exclusion;
        progress.update(
            current_index,
            format!("source-flow frame {current_index}/{}", result.samples.len()),
        );
    }
    decoder.finish()?;
    Ok(observations)
}

pub(crate) fn source_flow_candidate_quality(mut errors: Vec<f64>) -> (f64, f64) {
    (percentile(&mut errors, 0.50), percentile(&mut errors, 0.95))
}

pub(crate) fn source_flow_quality_is_better(candidate: (f64, f64), current: (f64, f64)) -> bool {
    candidate.1 + candidate.0 * 0.5 < current.1 + current.0 * 0.5 - 0.02
}

pub(crate) fn trajectory_candidate_objective(
    median: f64,
    p95: f64,
    dynamics: TrajectoryDynamics,
) -> f64 {
    p95 + median * 0.50 + (1.0 - dynamics.temporal_score) * 4.0 + dynamics.maximum_residual * 0.08
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn trajectory_candidate_is_better(
    candidate_objective: f64,
    candidate_median: f64,
    candidate_p95: f64,
    candidate_dynamics: TrajectoryDynamics,
    current_objective: f64,
    current_median: f64,
    current_p95: f64,
    current_dynamics: TrajectoryDynamics,
) -> bool {
    if !candidate_dynamics.is_physical() && current_dynamics.is_physical() {
        return false;
    }
    if candidate_dynamics.is_physical() && !current_dynamics.is_physical() {
        return true;
    }
    candidate_objective < current_objective - 0.02
        || source_flow_quality_is_better(
            (candidate_median, candidate_p95),
            (current_median, current_p95),
        )
}

pub(crate) fn assign_quads(
    samples: &mut [MotionSample],
    source: GeoQuad,
    quads: &[GeoQuad],
) -> Result<()> {
    for (sample, quad) in samples.iter_mut().zip(quads) {
        quad.validate("trajectory quad")?;
        sample.transform = Mat3 {
            values: homography(source, *quad)?.m,
        };
    }
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn solve_source_flow_by_reference_integration(
    source: GeoQuad,
    result: &TrackingResult,
    observations: &[SourceFlowConsistency],
) -> Result<Vec<GeoQuad>> {
    let mut quads = vec![source; result.samples.len()];
    let reference_frame = result
        .samples
        .iter()
        .position(|s| s.measurement_source == "reference")
        .unwrap_or(0);
    quads[reference_frame] = transformed_plaque(
        RectF {
            x: source.tl.x,
            y: source.tl.y,
            width: (source.tr.x - source.tl.x).abs().max(1.0),
            height: (source.bl.y - source.tl.y).abs().max(1.0),
        },
        result.samples[reference_frame].transform,
    );

    for index in (0..reference_frame).rev() {
        if index < observations.len() {
            let flow = &observations[index].material_transform;
            if let Some(inv_flow) = flow.inverse() {
                quads[index] = transform_quad(quads[index + 1], inv_flow);
            }
        }
    }
    for index in reference_frame + 1..result.samples.len() {
        if index - 1 < observations.len() {
            let flow = &observations[index - 1].material_transform;
            quads[index] = transform_quad(quads[index - 1], *flow);
        }
    }
    Ok(quads)
}

pub(crate) fn source_flow_observation_is_usable(observation: &SourceFlowConsistency) -> bool {
    observation.median_error_pixels.is_finite()
        && observation.median_error_pixels <= 3.0
        && observation.inlier_fraction >= 0.25
        && observation.tracked_points >= 12
}

pub(crate) fn load_exclusion_bytes(
    root: &Path,
    frame: usize,
    width: u32,
    height: u32,
) -> Result<Option<Vec<u8>>> {
    let path = root.join(format!("{frame:06}.png"));
    if !path.is_file() {
        return Ok(None);
    }
    let mask = imgcodecs::imread(&*path.to_string_lossy(), imgcodecs::IMREAD_GRAYSCALE)
        .with_context(|| format!("failed to read occluder mask {}", path.display()))?;
    if mask.cols() as u32 != width || mask.rows() as u32 != height {
        bail!("occluder mask dimensions differ from video");
    }
    let mut bytes = vec![0u8; (width * height) as usize];
    if let Ok(data) = mask.data_bytes() {
        bytes.copy_from_slice(data);
    } else {
        for y in 0..mask.rows() {
            for x in 0..mask.cols() {
                bytes[(y * mask.cols() + x) as usize] = *mask.at_2d::<u8>(y, x)?;
            }
        }
    }
    Ok(Some(bytes))
}

pub(crate) fn source_flow_errors_for_trajectory(
    quads: &[GeoQuad],
    plaque: RectF,
    observations: &[SourceFlowConsistency],
) -> Vec<f64> {
    let source = GeoQuad::from_rect(plaque.x, plaque.y, plaque.width, plaque.height);
    let mut errors = Vec::with_capacity(observations.len());
    for (index, observation) in observations.iter().enumerate() {
        if !source_flow_observation_is_usable(observation) || index + 1 >= quads.len() {
            continue;
        }
        let prev_mat = homography(source, quads[index]).map(|h| Mat3 { values: h.m });
        let curr_mat = homography(source, quads[index + 1]).map(|h| Mat3 { values: h.m });
        if let (Ok(prev), Ok(curr)) = (prev_mat, curr_mat) {
            errors.push(observation.error_for_poses(plaque, prev, curr));
        }
    }
    errors
}

#[allow(dead_code)]
pub(crate) fn solve_global_source_flow_quads(
    source: GeoQuad,
    result: &TrackingResult,
    observations: &[SourceFlowConsistency],
) -> Result<Vec<GeoQuad>> {
    solve_global_source_flow_quads_with_inertia(source, result, observations, 0.0)
}

pub(crate) fn solve_global_source_flow_quads_with_inertia(
    source: GeoQuad,
    result: &TrackingResult,
    observations: &[SourceFlowConsistency],
    inertia: f64,
) -> Result<Vec<GeoQuad>> {
    let raw_quads: Vec<GeoQuad> = result
        .samples
        .iter()
        .map(|sample| {
            let points = source.points().map(|p| {
                let mapped = sample.transform.transform(PointF { x: p.x, y: p.y });
                GeoPoint::new(mapped.x, mapped.y)
            });
            GeoQuad::new(points[0], points[1], points[2], points[3])
        })
        .collect();

    let mut current = raw_quads.clone();
    for _ in 0..8 {
        let previous = current.clone();
        for index in 0..result.samples.len() {
            let mut accumulator = WeightedQuad::default();
            let abs_weight = absolute_pose_weight(&result.samples[index]);
            accumulator.add(raw_quads[index], abs_weight);

            if index > 0 && index - 1 < observations.len() {
                let obs = &observations[index - 1];
                if source_flow_observation_is_usable(obs) {
                    let flow_quad = transform_quad(previous[index - 1], obs.material_transform);
                    accumulator.add(flow_quad, source_flow_edge_weight(1, obs));
                }
            }
            if index + 1 < result.samples.len()
                && index < observations.len()
                && source_flow_observation_is_usable(&observations[index])
                && let Some(inv_flow) = observations[index].material_transform.inverse()
            {
                let flow_quad = transform_quad(previous[index + 1], inv_flow);
                accumulator.add(flow_quad, source_flow_edge_weight(1, &observations[index]));
            }

            if inertia > 0.0 && index > 0 && index + 1 < result.samples.len() {
                let neighbor_smooth = previous[index - 1].lerp(previous[index + 1], 0.5);
                accumulator.add(neighbor_smooth, inertia * 0.5);
            }

            if let Some(updated) = accumulator.finish()
                && updated.validate("source flow quad").is_ok()
                && updated.orientation() > 0.0
            {
                current[index] = updated;
            }
        }
    }
    Ok(current)
}

#[derive(Default)]
pub(crate) struct WeightedQuad {
    pub(crate) points: [[f64; 2]; 4],
    pub(crate) total_weight: f64,
}

impl WeightedQuad {
    pub(crate) fn add(&mut self, quad: GeoQuad, weight: f64) {
        if weight <= 0.0 || !weight.is_finite() {
            return;
        }
        for (corner, point) in quad.points().into_iter().enumerate() {
            self.points[corner][0] += point.x * weight;
            self.points[corner][1] += point.y * weight;
        }
        self.total_weight += weight;
    }

    pub(crate) fn finish(self) -> Option<GeoQuad> {
        if self.total_weight <= 0.0 {
            return None;
        }
        let inv = 1.0 / self.total_weight;
        Some(GeoQuad::new(
            GeoPoint::new(self.points[0][0] * inv, self.points[0][1] * inv),
            GeoPoint::new(self.points[1][0] * inv, self.points[1][1] * inv),
            GeoPoint::new(self.points[2][0] * inv, self.points[2][1] * inv),
            GeoPoint::new(self.points[3][0] * inv, self.points[3][1] * inv),
        ))
    }
}

pub(crate) fn absolute_pose_weight(sample: &MotionSample) -> f64 {
    if !sample.measurement_valid {
        return 0.02;
    }
    let conf = sample_confidence(sample);
    (conf * 2.0).clamp(0.05, 3.0)
}

pub(crate) fn source_flow_edge_weight(_lag: usize, observation: &SourceFlowConsistency) -> f64 {
    if !source_flow_observation_is_usable(observation) {
        return 0.0;
    }
    (observation.inlier_fraction * (3.0 / observation.median_error_pixels.max(0.5))).clamp(0.1, 4.0)
}

#[allow(dead_code)]
pub(crate) fn adjacent_source_flow(
    observations: &[SourceFlowConsistency],
    from: usize,
    to: usize,
) -> Option<Mat3> {
    if to == from + 1 && from < observations.len() {
        Some(observations[from].material_transform)
    } else if from == to + 1 && to < observations.len() {
        observations[to].material_transform.inverse()
    } else {
        None
    }
}

pub(crate) fn transform_quad(quad: GeoQuad, transform: Mat3) -> GeoQuad {
    let points = quad.points().map(|p| {
        let mapped = transform.transform(PointF { x: p.x, y: p.y });
        GeoPoint::new(mapped.x, mapped.y)
    });
    GeoQuad::new(points[0], points[1], points[2], points[3])
}

pub(crate) fn source_flow_confidence(p95: f64, p99: f64) -> f64 {
    let p95_score = (-p95 / 2.0).exp();
    let p99_score = (-p99 / 4.0).exp();
    (p95_score * 0.65 + p99_score * 0.35).clamp(0.05, 0.99)
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

pub(crate) fn repair_outliers(samples: &mut [MotionSample], plaque: RectF) -> usize {
    if samples.len() < 5 {
        return 0;
    }

    let source_corners = super::features::plaque_corners(plaque);
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

pub(crate) fn solve_unobserved_intervals(
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
        samples[frame].reprojection_error = support_distance.clamp(4.0, 24.0);
        samples[frame].ecc = None;
        inferred += 1;
    }
    Ok(inferred)
}

pub(crate) fn inferred_quad_is_physical(candidate: GeoQuad, plaque: RectF) -> bool {
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

pub(crate) fn offscreen_tail_is_physical(
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

pub(crate) fn surface_quad_visible_fraction(quad: GeoQuad, width: u32, height: u32) -> f64 {
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

pub(crate) fn nearest_measurement_distance(
    frame: usize,
    left: Option<usize>,
    right: Option<usize>,
) -> usize {
    left.map(|value| frame - value)
        .into_iter()
        .chain(right.map(|value| value - frame))
        .min()
        .unwrap_or(usize::MAX)
}

pub(crate) fn quad_velocity(
    a: GeoQuad,
    b: GeoQuad,
    a_frame: usize,
    b_frame: usize,
) -> [[f64; 2]; 4] {
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

pub(crate) fn hermite_quad(
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

pub(crate) fn translate_quad(quad: GeoQuad, velocity: [[f64; 2]; 4], duration: f64) -> GeoQuad {
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

#[derive(Debug, Clone, Copy)]
pub(crate) struct TrajectoryDynamics {
    pub(crate) temporal_score: f64,
    #[allow(dead_code)]
    pub(crate) p95_residual: f64,
    pub(crate) maximum_residual: f64,
    pub(crate) worst_frame: usize,
    pub(crate) loop_score: f64,
    pub(crate) loop_residual: f64,
}

impl TrajectoryDynamics {
    pub(crate) fn is_physical(self) -> bool {
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
    let last = if loop_closed {
        samples.len()
    } else {
        samples.len() - 1
    };

    for index in first..last {
        let left = if index == 0 {
            samples.len() - 1
        } else {
            index - 1
        };
        let right = if index + 1 == samples.len() {
            0
        } else {
            index + 1
        };
        let predicted = quads[left].lerp(quads[right], 0.5);
        let residual = mean_quad_distance(quads[index], predicted);
        residuals.push(residual);
        if residual > maximum_residual {
            maximum_residual = residual;
            worst_frame = index;
        }
    }

    let p95_residual = percentile(&mut residuals, 0.95);
    let loop_residual = if loop_closed {
        mean_quad_distance(quads[0], quads[samples.len() - 1])
    } else {
        0.0
    };
    let score_for = |r: f64| (-r / 3.0).exp().clamp(0.0, 1.0);

    TrajectoryDynamics {
        temporal_score: score_for(p95_residual),
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
