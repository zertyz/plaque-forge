use anyhow::{Context, Result, bail};
use opencv::{
    core::{self, DMatch, KeyPoint, Mat, Point2f, Vector},
    features2d, geometry,
    prelude::*,
};

use crate::model::{Mat3, MotionSample, PointF, RectF};

use super::{
    features::{
        correspondence_inlier_coverage, match_coverage, plaque_corners, plaque_point_coverage,
        plaque_transform_is_valid,
    },
    types::MotionModel,
};

#[derive(Clone)]
pub(crate) struct FeatureAnchor {
    pub(crate) frame: usize,
    pub(crate) gray: Mat,
    pub(crate) keypoints: Vector<KeyPoint>,
    pub(crate) descriptors: Mat,
    pub(crate) transform: Mat3,
}

#[derive(Clone, Copy)]
pub(crate) struct Estimate {
    pub(crate) matrix: Mat3,
    pub(crate) inlier_ratio: f64,
    pub(crate) error: f64,
    pub(crate) tracked_points: usize,
    pub(crate) spatial_coverage: f64,
    pub(crate) ecc: Option<f64>,
    pub(crate) source: &'static str,
    pub(crate) static_model: bool,
}

impl Estimate {
    pub(crate) fn anchored(mut self, anchor: Mat3, source: &'static str) -> Self {
        self.matrix = self.matrix.multiply(anchor);
        self.source = source;
        self
    }
}

pub(crate) fn estimate_persistent_transform(
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

pub(crate) fn measurement_is_credible(estimate: &Estimate) -> bool {
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

pub(crate) fn measurement_uncertainty(estimate: &Estimate) -> f64 {
    if !estimate.error.is_finite() {
        return f64::INFINITY;
    }
    let support_penalty = (12.0 / estimate.tracked_points.max(1) as f64)
        .sqrt()
        .max(1.0);
    let coverage_penalty = 1.0 / estimate.spatial_coverage.max(0.10).sqrt();
    (estimate.error.max(0.20) * support_penalty * coverage_penalty).clamp(0.20, 24.0)
}

pub(crate) fn reacquisition_can_reseed_points(estimate: &Estimate) -> bool {
    measurement_is_credible(estimate)
        && estimate.tracked_points >= 24
        && estimate.spatial_coverage >= 0.60
        && estimate.inlier_ratio >= 0.45
        && estimate.error <= 2.0
        && matches!(estimate.source, "root" | "root-static")
}

pub(crate) fn choose_estimate(
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

pub(crate) fn choose_persistent_estimate(
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

pub(crate) fn continuity_penalty(candidate: Mat3, predicted: Mat3, plaque: RectF) -> f64 {
    let mean = mean_corner_distance(candidate, predicted, plaque);
    (mean - 6.0).max(0.0) * 0.20
}

pub(crate) fn mean_corner_distance(left: Mat3, right: Mat3, plaque: RectF) -> f64 {
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

pub(crate) fn choose_geometric_constraint(
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

pub(crate) fn anchor_sample(anchor: &FeatureAnchor) -> MotionSample {
    MotionSample {
        frame: anchor.frame,
        transform: anchor.transform,
        measurement_valid: true,
        tracked_points: anchor.keypoints.len(),
        spatial_coverage: 1.0,
        uncertainty_px: 0.20,
        measurement_source: "reference".into(),
        inlier_ratio: 1.0,
        reprojection_error: 0.0,
        ecc: Some(1.0),
        plaque_visibility: 1.0,
        occluder_coverage: 0.0,
    }
}

pub(crate) fn estimate_reference_transform(
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

pub(crate) fn estimate_objective(estimate: &Estimate, complexity_penalty: f64) -> f64 {
    estimate.error + complexity_penalty + (1.0 - estimate.inlier_ratio) * 2.0
}

pub(crate) fn mutual_correspondences(
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

pub(crate) fn static_estimate(
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

pub(crate) fn estimate_model(
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
        spatial_coverage: coverage.min(inlier_coverage),
        ecc: None,
        source: "feature",
        static_model: false,
    })
}

pub(crate) fn sample_confidence(sample: &MotionSample) -> f64 {
    (sample.inlier_ratio * (-sample.reprojection_error.min(20.0) / 5.0).exp()).clamp(0.0, 1.0)
}

pub(crate) fn mat_to_mat3(matrix: &Mat) -> Result<Mat3> {
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

pub(crate) fn symmetric_reprojection_error(
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

pub(crate) fn correspondence_errors(
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

pub fn median(mut values: Vec<f64>) -> f64 {
    crate::stats::median(&mut values)
}
