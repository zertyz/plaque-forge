use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RectF {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PointF {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Mat3 {
    pub values: [[f64; 3]; 3],
}

impl Mat3 {
    pub const IDENTITY: Self = Self {
        values: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    };

    pub fn transform(self, point: PointF) -> PointF {
        let m = self.values;
        let w = m[2][0] * point.x + m[2][1] * point.y + m[2][2];
        PointF {
            x: (m[0][0] * point.x + m[0][1] * point.y + m[0][2]) / w,
            y: (m[1][0] * point.x + m[1][1] * point.y + m[1][2]) / w,
        }
    }

    pub fn multiply(self, rhs: Self) -> Self {
        let mut out = [[0.0; 3]; 3];
        for (r, row) in out.iter_mut().enumerate() {
            for (c, cell) in row.iter_mut().enumerate() {
                *cell = (0..3).map(|k| self.values[r][k] * rhs.values[k][c]).sum();
            }
        }
        Self { values: out }
    }

    pub fn inverse(self) -> Option<Self> {
        let m = self.values;
        let determinant = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        if determinant.abs() < 1.0e-12 {
            return None;
        }
        let scale = determinant.recip();
        Some(Self {
            values: [
                [
                    (m[1][1] * m[2][2] - m[1][2] * m[2][1]) * scale,
                    (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * scale,
                    (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * scale,
                ],
                [
                    (m[1][2] * m[2][0] - m[1][0] * m[2][2]) * scale,
                    (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * scale,
                    (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * scale,
                ],
                [
                    (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * scale,
                    (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * scale,
                    (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * scale,
                ],
            ],
        })
    }

    pub fn translation(x: f64, y: f64) -> Self {
        Self {
            values: [[1.0, 0.0, x], [0.0, 1.0, y], [0.0, 0.0, 1.0]],
        }
    }

    pub fn scale(x: f64, y: f64) -> Self {
        Self {
            values: [[x, 0.0, 0.0], [0.0, y, 0.0], [0.0, 0.0, 1.0]],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MotionSample {
    pub frame: usize,
    pub transform: Mat3,
    /// True only when this frame has independent visual evidence for its pose.
    /// A temporally solved pose is never relabelled as a measurement.
    #[serde(default)]
    pub measurement_valid: bool,
    /// Number of robust point correspondences supporting the selected model.
    #[serde(default)]
    pub tracked_points: usize,
    /// Spatial distribution of the observations over the surface, in [0, 1].
    #[serde(default)]
    pub spatial_coverage: f64,
    /// Estimated one-sigma image-space uncertainty in pixels.
    #[serde(default = "unknown_uncertainty")]
    pub uncertainty_px: f64,
    /// Measurement backend, or `temporal-solve` for an inferred pose.
    #[serde(default)]
    pub measurement_source: String,
    pub inlier_ratio: f64,
    pub reprojection_error: f64,
    pub ecc: Option<f64>,
    #[serde(default = "fully_visible")]
    pub plaque_visibility: f64,
    #[serde(default)]
    pub occluder_coverage: f64,
}

impl Default for MotionSample {
    fn default() -> Self {
        Self {
            frame: 0,
            transform: Mat3::IDENTITY,
            measurement_valid: false,
            tracked_points: 0,
            spatial_coverage: 0.0,
            uncertainty_px: f64::INFINITY,
            measurement_source: String::new(),
            inlier_ratio: 0.0,
            reprojection_error: f64::INFINITY,
            ecc: None,
            plaque_visibility: 1.0,
            occluder_coverage: 0.0,
        }
    }
}

fn unknown_uncertainty() -> f64 {
    f64::INFINITY
}

fn fully_visible() -> f64 {
    1.0
}

impl MotionSample {
    pub fn validate(&self) -> anyhow::Result<()> {
        let finite_transform = self
            .transform
            .values
            .iter()
            .flatten()
            .all(|value| value.is_finite());
        anyhow::ensure!(
            finite_transform,
            "motion transform contains a non-finite value"
        );
        anyhow::ensure!(
            self.inlier_ratio.is_finite()
                && self.reprojection_error.is_finite()
                && self.spatial_coverage.is_finite()
                && self.uncertainty_px.is_finite()
                && self.plaque_visibility.is_finite()
                && self.occluder_coverage.is_finite(),
            "motion sample {} contains a non-finite metric",
            self.frame
        );
        anyhow::ensure!(
            self.ecc.is_none_or(f64::is_finite),
            "motion sample {} contains a non-finite ECC value",
            self.frame
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisConfidence {
    pub plaque_detection: f64,
    pub motion: f64,
    pub extraction: f64,
    pub occlusion: f64,
    pub overall: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TypographyMetrics {
    pub fit_mode: String,
    pub font_size: f32,
    pub maximum_safe_font_size: f32,
    pub lines: usize,
    pub fill_ratio: f64,
    pub minimum_padding_ratio: f64,
    pub clipped_pixels: u64,
    pub missing_glyphs: usize,
    pub fallback_glyphs: usize,
    pub explicit_newlines: usize,
    /// Text after renderer-selected line breaks. Equal to the input outside artistic fitting.
    #[serde(default)]
    pub resolved_text: String,
}
