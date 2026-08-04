use anyhow::{Context, Result, bail};
use std::{cmp::Ordering, fs, path::Path};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn lerp(self, other: Self, t: f64) -> Self {
        Self::new(
            self.x + (other.x - self.x) * t,
            self.y + (other.y - self.y) * t,
        )
    }

    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y)
    }

    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y)
    }

    fn scale(self, factor: f64) -> Self {
        Self::new(self.x * factor, self.y * factor)
    }

    fn hermite(
        start: Self,
        end: Self,
        start_slope: Self,
        end_slope: Self,
        t: f64,
        span: f64,
    ) -> Self {
        let t2 = t * t;
        let t3 = t2 * t;
        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;
        start
            .scale(h00)
            .add(start_slope.scale(h10 * span))
            .add(end.scale(h01))
            .add(end_slope.scale(h11 * span))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quad {
    pub tl: Point,
    pub tr: Point,
    pub br: Point,
    pub bl: Point,
}

impl Quad {
    pub const fn new(tl: Point, tr: Point, br: Point, bl: Point) -> Self {
        Self { tl, tr, br, bl }
    }

    pub fn from_rect(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self::new(
            Point::new(x, y),
            Point::new(x + w, y),
            Point::new(x + w, y + h),
            Point::new(x, y + h),
        )
    }

    pub fn points(self) -> [Point; 4] {
        [self.tl, self.tr, self.br, self.bl]
    }

    pub fn lerp(self, other: Self, t: f64) -> Self {
        Self::new(
            self.tl.lerp(other.tl, t),
            self.tr.lerp(other.tr, t),
            self.br.lerp(other.br, t),
            self.bl.lerp(other.bl, t),
        )
    }

    fn sub(self, other: Self) -> Self {
        Self::new(
            self.tl.sub(other.tl),
            self.tr.sub(other.tr),
            self.br.sub(other.br),
            self.bl.sub(other.bl),
        )
    }

    fn scale(self, factor: f64) -> Self {
        Self::new(
            self.tl.scale(factor),
            self.tr.scale(factor),
            self.br.scale(factor),
            self.bl.scale(factor),
        )
    }

    fn hermite(
        start: Self,
        end: Self,
        start_slope: Self,
        end_slope: Self,
        t: f64,
        span: f64,
    ) -> Self {
        Self::new(
            Point::hermite(start.tl, end.tl, start_slope.tl, end_slope.tl, t, span),
            Point::hermite(start.tr, end.tr, start_slope.tr, end_slope.tr, t, span),
            Point::hermite(start.br, end.br, start_slope.br, end_slope.br, t, span),
            Point::hermite(start.bl, end.bl, start_slope.bl, end_slope.bl, t, span),
        )
    }

    pub fn bounds(self) -> (f64, f64, f64, f64) {
        let points = self.points();
        let min_x = points.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
        let min_y = points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
        let max_x = points.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
        let max_y = points.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
        (min_x, min_y, max_x, max_y)
    }

    pub fn orientation(self) -> f64 {
        let points = self.points();
        points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .take(4)
            .map(|(a, b)| a.x * b.y - b.x * a.y)
            .sum::<f64>()
            * 0.5
    }

    pub fn validate(self, description: &str) -> Result<()> {
        let points = self.points();
        if points
            .iter()
            .any(|point| !point.x.is_finite() || !point.y.is_finite())
        {
            bail!("{description} contains a non-finite coordinate");
        }

        let mut cross_sign = 0.0_f64;
        for index in 0..4 {
            let a = points[index];
            let b = points[(index + 1) % 4];
            let c = points[(index + 2) % 4];
            let ab_x = b.x - a.x;
            let ab_y = b.y - a.y;
            let bc_x = c.x - b.x;
            let bc_y = c.y - b.y;
            if ab_x.hypot(ab_y) < 1e-9 {
                bail!("{description} has a zero-length edge");
            }
            let cross = ab_x * bc_y - ab_y * bc_x;
            if cross.abs() < 1e-12 {
                bail!("{description} has three collinear consecutive corners");
            }
            if cross_sign == 0.0 {
                cross_sign = cross.signum();
            } else if cross.signum() != cross_sign {
                bail!("{description} is concave or self-intersecting");
            }
        }

        if self.orientation().abs() < 1e-12 {
            bail!("{description} has zero area");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Mat3 {
    pub m: [[f64; 3]; 3],
}

impl Mat3 {
    pub fn transform(self, p: Point) -> Option<Point> {
        let z = self.m[2][0] * p.x + self.m[2][1] * p.y + self.m[2][2];
        if z.abs() < 1e-12 {
            return None;
        }
        Some(Point::new(
            (self.m[0][0] * p.x + self.m[0][1] * p.y + self.m[0][2]) / z,
            (self.m[1][0] * p.x + self.m[1][1] * p.y + self.m[1][2]) / z,
        ))
    }

    pub fn inverse(self) -> Result<Self> {
        let m = self.m;
        let a = m[0][0];
        let b = m[0][1];
        let c = m[0][2];
        let d = m[1][0];
        let e = m[1][1];
        let f = m[1][2];
        let g = m[2][0];
        let h = m[2][1];
        let i = m[2][2];

        let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
        if det.abs() < 1e-12 {
            bail!("homography is singular");
        }
        let inv = 1.0 / det;
        Ok(Self {
            m: [
                [
                    (e * i - f * h) * inv,
                    (c * h - b * i) * inv,
                    (b * f - c * e) * inv,
                ],
                [
                    (f * g - d * i) * inv,
                    (a * i - c * g) * inv,
                    (c * d - a * f) * inv,
                ],
                [
                    (d * h - e * g) * inv,
                    (b * g - a * h) * inv,
                    (a * e - b * d) * inv,
                ],
            ],
        })
    }
}

/// Returns the projective transform mapping `source` to `destination`.
#[allow(clippy::needless_range_loop)]
pub fn homography(source: Quad, destination: Quad) -> Result<Mat3> {
    let src = source.points();
    let dst = destination.points();
    let mut augmented = [[0.0_f64; 9]; 8];

    for row in 0..4 {
        let x = src[row].x;
        let y = src[row].y;
        let u = dst[row].x;
        let v = dst[row].y;

        augmented[row * 2] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
        augmented[row * 2 + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
    }

    for col in 0..8 {
        let pivot = (col..8)
            .max_by(|&a, &b| {
                augmented[a][col]
                    .abs()
                    .partial_cmp(&augmented[b][col].abs())
                    .unwrap_or(Ordering::Equal)
            })
            .context("empty homography system")?;

        if augmented[pivot][col].abs() < 1e-12 {
            bail!("degenerate quadrilateral cannot define a homography");
        }
        if pivot != col {
            augmented.swap(pivot, col);
        }

        let divisor = augmented[col][col];
        for j in col..9 {
            augmented[col][j] /= divisor;
        }

        for row in 0..8 {
            if row == col {
                continue;
            }
            let factor = augmented[row][col];
            for j in col..9 {
                augmented[row][j] -= factor * augmented[col][j];
            }
        }
    }

    let h = [
        augmented[0][8],
        augmented[1][8],
        augmented[2][8],
        augmented[3][8],
        augmented[4][8],
        augmented[5][8],
        augmented[6][8],
        augmented[7][8],
    ];
    Ok(Mat3 {
        m: [[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], 1.0]],
    })
}

#[derive(Debug, Clone)]
pub struct TrackKeyframe {
    pub frame: f64,
    pub quad: Quad,
}

#[derive(Debug, Clone)]
pub struct QuadTrack {
    keyframes: Vec<TrackKeyframe>,
}

impl QuadTrack {
    pub fn new(mut keyframes: Vec<TrackKeyframe>) -> Result<Self> {
        if keyframes.is_empty() {
            bail!("quad track contains no keyframes");
        }
        keyframes.sort_by(|a, b| a.frame.total_cmp(&b.frame));
        let mut expected_orientation = 0.0_f64;
        for (index, keyframe) in keyframes.iter().enumerate() {
            if !keyframe.frame.is_finite() {
                bail!("quad track keyframe {index} has a non-finite frame number");
            }
            keyframe
                .quad
                .validate(&format!("quad track keyframe {index}"))?;
            let orientation = keyframe.quad.orientation().signum();
            if expected_orientation == 0.0 {
                expected_orientation = orientation;
            } else if orientation != expected_orientation {
                bail!("quad track changes corner winding at keyframe {index}");
            }
        }
        for pair in keyframes.windows(2) {
            if pair[0].frame == pair[1].frame {
                bail!("duplicate track frame {}", pair[0].frame);
            }
        }
        Ok(Self { keyframes })
    }

    pub fn load_csv(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read track {}", path.display()))?;
        let mut keyframes = Vec::new();

        for (line_no, line) in text.lines().enumerate() {
            if line_no == 0 && line.trim_start().starts_with("frame,") {
                continue;
            }
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<_> = line.split(',').map(str::trim).collect();
            if fields.len() != 9 {
                bail!(
                    "{}:{}: expected 9 comma-separated fields, found {}",
                    path.display(),
                    line_no + 1,
                    fields.len()
                );
            }
            let mut values = [0.0_f64; 9];
            for (index, field) in fields.iter().enumerate() {
                values[index] = field.parse().with_context(|| {
                    format!(
                        "{}:{}: invalid number {:?}",
                        path.display(),
                        line_no + 1,
                        field
                    )
                })?;
            }
            keyframes.push(TrackKeyframe {
                frame: values[0],
                quad: Quad::new(
                    Point::new(values[1], values[2]),
                    Point::new(values[3], values[4]),
                    Point::new(values[5], values[6]),
                    Point::new(values[7], values[8]),
                ),
            });
        }
        Self::new(keyframes)
    }

    pub fn at(&self, frame: f64) -> Quad {
        if frame <= self.keyframes[0].frame {
            return self.keyframes[0].quad;
        }
        let last = &self.keyframes[self.keyframes.len() - 1];
        if frame >= last.frame {
            return last.quad;
        }

        let upper = self
            .keyframes
            .partition_point(|keyframe| keyframe.frame <= frame);
        let lower = upper - 1;
        let a = &self.keyframes[lower];
        let b = &self.keyframes[upper];
        let span = b.frame - a.frame;
        let t = (frame - a.frame) / span;

        let start_slope = if lower > 0 {
            let previous = &self.keyframes[lower - 1];
            b.quad
                .sub(previous.quad)
                .scale(1.0 / (b.frame - previous.frame))
        } else {
            b.quad.sub(a.quad).scale(1.0 / span)
        };
        let end_slope = if upper + 1 < self.keyframes.len() {
            let next = &self.keyframes[upper + 1];
            next.quad.sub(a.quad).scale(1.0 / (next.frame - a.frame))
        } else {
            b.quad.sub(a.quad).scale(1.0 / span)
        };

        let candidate = Quad::hermite(a.quad, b.quad, start_slope, end_slope, t, span);
        if candidate.validate("interpolated quad").is_ok() {
            candidate
        } else {
            a.quad.lerp(b.quad, t)
        }
    }

    pub fn len(&self) -> usize {
        self.keyframes.len()
    }

    pub fn first_frame(&self) -> f64 {
        self.keyframes[0].frame
    }

    pub fn last_frame(&self) -> f64 {
        self.keyframes[self.keyframes.len() - 1].frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-7, "{a} != {b}");
    }

    #[test]
    fn homography_maps_all_four_corners() {
        let source = Quad::from_rect(0.0, 0.0, 100.0, 60.0);
        let target = Quad::new(
            Point::new(10.0, 20.0),
            Point::new(220.0, 15.0),
            Point::new(200.0, 170.0),
            Point::new(30.0, 150.0),
        );
        let h = homography(source, target).unwrap();
        for (from, to) in source.points().into_iter().zip(target.points()) {
            let mapped = h.transform(from).unwrap();
            assert_close(mapped.x, to.x);
            assert_close(mapped.y, to.y);
        }
    }

    #[test]
    fn rejects_self_intersecting_quad() {
        let bow_tie = Quad::new(
            Point::new(0.0, 0.0),
            Point::new(1.0, 1.0),
            Point::new(0.0, 1.0),
            Point::new(1.0, 0.0),
        );
        assert!(bow_tie.validate("bow tie").is_err());
    }

    #[test]
    fn sparse_track_interpolation_hits_keyframes_exactly() {
        let first = Quad::from_rect(0.1, 0.1, 0.4, 0.2);
        let second = Quad::from_rect(0.2, 0.15, 0.5, 0.25);
        let third = Quad::from_rect(0.15, 0.2, 0.45, 0.3);
        let track = QuadTrack::new(vec![
            TrackKeyframe {
                frame: 0.0,
                quad: first,
            },
            TrackKeyframe {
                frame: 10.0,
                quad: second,
            },
            TrackKeyframe {
                frame: 20.0,
                quad: third,
            },
        ])
        .unwrap();
        assert_eq!(track.at(0.0), first);
        assert_eq!(track.at(10.0), second);
        assert_eq!(track.at(20.0), third);
        track.at(5.0).validate("midpoint").unwrap();
        track.at(15.0).validate("midpoint").unwrap();
    }

    #[test]
    fn inverse_round_trip() {
        let source = Quad::from_rect(0.0, 0.0, 100.0, 60.0);
        let target = Quad::new(
            Point::new(10.0, 20.0),
            Point::new(220.0, 15.0),
            Point::new(200.0, 170.0),
            Point::new(30.0, 150.0),
        );
        let h = homography(source, target).unwrap();
        let inv = h.inverse().unwrap();
        let p = Point::new(42.5, 31.25);
        let round_trip = inv.transform(h.transform(p).unwrap()).unwrap();
        assert_close(round_trip.x, p.x);
        assert_close(round_trip.y, p.y);
    }
}
