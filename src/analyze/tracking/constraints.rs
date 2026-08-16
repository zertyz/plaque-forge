use anyhow::{Context, Result, bail};

use crate::{
    geometry::{Point as GeoPoint, Quad as GeoQuad, homography},
    model::{Mat3, MotionSample, PointF, RectF},
    scene::SurfaceTrajectory,
};

use super::types::TrackingResult;

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
pub(crate) enum ConstraintSelection {
    All,
    Locked,
}

pub(crate) fn apply_scene_constraints(
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

pub fn transformed_quad(source: GeoQuad, transform: Mat3) -> GeoQuad {
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

pub fn scene_quad(points: [[f64; 2]; 4]) -> GeoQuad {
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

pub(crate) fn trajectory_loop_closed(samples: &[MotionSample], plaque: RectF) -> bool {
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
