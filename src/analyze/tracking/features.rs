use std::path::Path;

use anyhow::{Context, Result, bail};
use opencv::{
    core::{
        self, AlgorithmHint, KeyPoint, Mat, Point, Point2f, Rect, Scalar, Size, TermCriteria,
        TermCriteria_Type, Vector,
    },
    features, features2d, geometry, imgcodecs, imgproc,
    prelude::*,
    video as cv_video,
    videoio::{CAP_PROP_POS_FRAMES, VideoCapture},
};

use crate::{
    geometry::{Point as GeoPoint, Quad as GeoQuad},
    model::{Mat3, MotionSample, PointF, RectF},
};

use super::solver::{Estimate, FeatureAnchor, estimate_persistent_transform};

pub(crate) struct PersistentPointTracker {
    pub(crate) previous_gray: Mat,
    pub(crate) canonical: Vector<Point2f>,
    pub(crate) current: Vector<Point2f>,
}

impl PersistentPointTracker {
    pub(crate) fn new(
        gray: &Mat,
        plaque: RectF,
        pose: Mat3,
        exclusion: Option<&Mat>,
    ) -> Result<Self> {
        let mut tracker = Self {
            previous_gray: gray.try_clone()?,
            canonical: Vector::new(),
            current: Vector::new(),
        };
        tracker.reset(gray, plaque, pose, exclusion)?;
        Ok(tracker)
    }

    pub(crate) fn advance(
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

    pub(crate) fn reset(
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

    pub(crate) fn add_corners(
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

    pub(crate) fn prune_to_pose(&mut self, pose: Mat3, maximum_error: f64) -> Result<()> {
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

pub(crate) fn persistent_corner_count(
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

pub(crate) fn point_is_excluded(exclusion: Option<&Mat>, point: Point2f) -> Result<bool> {
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

pub(crate) fn byte_point_is_excluded(
    exclusion: Option<&[u8]>,
    width: i32,
    height: i32,
    point: Point2f,
) -> bool {
    let Some(exclusion) = exclusion else {
        return false;
    };
    let x = point.x.round() as i32;
    let y = point.y.round() as i32;
    if x < 0 || y < 0 || x >= width || y >= height {
        return true;
    }
    let offset = (y * width + x) as usize;
    exclusion.get(offset).copied().unwrap_or(0) > 0
}

pub(crate) fn plaque_point_coverage(plaque: RectF, points: &Vector<Point2f>) -> f64 {
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

pub(crate) fn match_coverage(
    reference: &Vector<KeyPoint>,
    matched: &Vector<Point2f>,
) -> Result<f64> {
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

pub(crate) fn correspondence_inlier_coverage(
    source: &Vector<Point2f>,
    inliers: &Mat,
) -> Result<f64> {
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

pub(crate) fn plaque_feature_mask_for_transform(
    width: i32,
    height: i32,
    plaque: RectF,
    transform: Mat3,
) -> Result<Mat> {
    let mut mask = Mat::new_rows_cols_with_default(height, width, core::CV_8UC1, Scalar::all(0.0))?;
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

#[derive(Clone, Copy)]
pub(crate) struct PlaqueContour {
    pub(crate) quad: GeoQuad,
    pub(crate) confidence: f64,
}

pub(crate) fn detect_plaque_contour(
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
        AlgorithmHint::ALGO_HINT_DEFAULT,
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

pub(crate) fn oriented_rect_quad(
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

pub(crate) fn plaque_corners(plaque: RectF) -> [PointF; 4] {
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

pub(crate) fn plaque_transform_is_valid(transform: Mat3, plaque: RectF) -> bool {
    let quad = transformed_plaque(plaque, transform);
    if quad.validate("tracked plaque").is_err() || quad.orientation() <= 0.0 {
        return false;
    }
    let source_area = plaque.width * plaque.height;
    let area_ratio = quad.orientation() / source_area.max(1.0);
    if !(0.15..=6.0).contains(&area_ratio) {
        return false;
    }
    let top = edge_length(quad.tl, quad.tr);
    let bottom = edge_length(quad.bl, quad.br);
    let left = edge_length(quad.tl, quad.bl);
    let right = edge_length(quad.tr, quad.br);
    edge_ratio(top, bottom) <= 1.55 && edge_ratio(left, right) <= 1.55
}

pub(crate) fn edge_length(a: GeoPoint, b: GeoPoint) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

pub(crate) fn edge_ratio(a: f64, b: f64) -> f64 {
    a.max(b) / a.min(b).max(1.0)
}

pub(crate) fn transformed_plaque(plaque: RectF, transform: Mat3) -> GeoQuad {
    let points = plaque_corners(plaque).map(|point| {
        let mapped = transform.transform(point);
        GeoPoint::new(mapped.x, mapped.y)
    });
    GeoQuad::new(points[0], points[1], points[2], points[3])
}

pub(crate) fn make_anchor(
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

pub(crate) fn load_exclusion(
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
    let mask = imgcodecs::imread(&*path.to_string_lossy(), imgcodecs::IMREAD_GRAYSCALE)
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

pub(crate) fn apply_exclusion(mask: &mut Mat, exclusion: Option<&Mat>) -> Result<f64> {
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

pub(crate) fn clone_anchor(anchor: &FeatureAnchor) -> Result<FeatureAnchor> {
    Ok(FeatureAnchor {
        frame: anchor.frame,
        gray: anchor.gray.try_clone()?,
        keypoints: anchor.keypoints.clone(),
        descriptors: anchor.descriptors.try_clone()?,
        transform: anchor.transform,
    })
}

pub(crate) fn read_gray(capture: &mut VideoCapture, frame_index: usize) -> Result<Mat> {
    capture.set(CAP_PROP_POS_FRAMES, frame_index as f64)?;
    read_next_gray(capture, frame_index)
}

pub(crate) fn read_next_gray(capture: &mut VideoCapture, frame_index: usize) -> Result<Mat> {
    let mut frame = Mat::default();
    if !capture.read(&mut frame)? || frame.empty() {
        bail!("failed to decode frame {frame_index}");
    }
    grayscale(&frame)
}

pub(crate) fn grayscale(frame: &Mat) -> Result<Mat> {
    let mut gray = Mat::default();
    imgproc::cvt_color(
        frame,
        &mut gray,
        imgproc::COLOR_BGR2GRAY,
        0,
        AlgorithmHint::ALGO_HINT_DEFAULT,
    )?;
    Ok(gray)
}

pub(crate) fn draw_diagnostic(mut frame: Mat, plaque: RectF, transform: Mat3) -> Result<Mat> {
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

pub(crate) fn write_contact_sheet(frames: &[Mat], path: &Path) -> Result<()> {
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
    imgcodecs::imwrite(&*path.to_string_lossy(), &sheet, &Vector::new())?;
    Ok(())
}

pub(crate) fn write_tracking_diagnostics(
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

pub(crate) fn should_close_loop(capture: &mut VideoCapture, frame_count: usize) -> Result<bool> {
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
