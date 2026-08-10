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
    pub inlier_ratio: f64,
    pub reprojection_error: f64,
    pub ecc: Option<f64>,
    #[serde(default = "fully_visible")]
    pub plaque_visibility: f64,
    #[serde(default)]
    pub occluder_coverage: f64,
}

fn fully_visible() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisConfidence {
    pub plaque_detection: f64,
    pub motion: f64,
    pub extraction: f64,
    pub occlusion: f64,
    pub overall: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}
