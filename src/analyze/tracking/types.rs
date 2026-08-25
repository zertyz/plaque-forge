use crate::model::{Mat3, MotionSample};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionModel {
    Adaptive,
    Similarity,
    Affine,
    Projective,
}

pub struct TrackingResult {
    pub samples: Vec<MotionSample>,
    pub model_name: String,
    /// Whether the trajectory pins the surface to the screen instead of the scene.
    pub screen_fixed: bool,
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
        screen_fixed: true,
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
