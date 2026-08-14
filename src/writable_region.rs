//! Human- and machine-declared writable-region geometry.
//!
//! Tracking still operates on a planar enclosing rectangle. The writable region is a
//! separate mask inside that rectangle, so circles, rounded signs, polygons, and
//! arbitrary masks do not require separate tracking implementations.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::model::RectF;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "kebab-case", deny_unknown_fields)]
pub enum WritableRegion {
    Rect {
        bounds: [f64; 4],
    },
    RoundedRect {
        bounds: [f64; 4],
        radius: f64,
    },
    Ellipse {
        center: [f64; 2],
        radii: [f64; 2],
        #[serde(default)]
        rotation_degrees: f64,
    },
    Polygon {
        points: Vec<[f64; 2]>,
    },
    Mask {
        bounds: [f64; 4],
        path: PathBuf,
    },
}

#[derive(Debug, Clone)]
pub enum ResolvedWritableRegion {
    Rect {
        bounds: [f64; 4],
    },
    RoundedRect {
        bounds: [f64; 4],
        radius: f64,
    },
    Ellipse {
        center: [f64; 2],
        radii: [f64; 2],
        rotation_degrees: f64,
    },
    Polygon {
        points: Vec<[f64; 2]>,
    },
    Mask {
        bounds: [f64; 4],
        path: PathBuf,
    },
}

impl WritableRegion {
    pub fn validate(&self, description: &str) -> Result<()> {
        match self {
            Self::Rect { bounds } => validate_rect(*bounds, description),
            Self::RoundedRect { bounds, radius } => {
                validate_rect(*bounds, description)?;
                if !radius.is_finite() || *radius < 0.0 {
                    bail!("{description} radius must be finite and non-negative");
                }
                if *radius > bounds[2].min(bounds[3]) * 0.5 {
                    bail!("{description} radius cannot exceed half the shorter side");
                }
                Ok(())
            }
            Self::Ellipse {
                center,
                radii,
                rotation_degrees,
            } => {
                validate_point(*center, &format!("{description} center"))?;
                if radii
                    .iter()
                    .any(|value| !value.is_finite() || *value <= 0.0)
                {
                    bail!("{description} radii must be finite and positive");
                }
                if !rotation_degrees.is_finite() {
                    bail!("{description} rotation_degrees must be finite");
                }
                Ok(())
            }
            Self::Polygon { points } => {
                if points.len() < 3 {
                    bail!("{description} polygon must contain at least three points");
                }
                for (index, point) in points.iter().enumerate() {
                    validate_point(*point, &format!("{description} points[{index}]"))?;
                }
                let area = polygon_area(points);
                if area.abs() < 1.0e-6 {
                    bail!("{description} polygon has zero area");
                }
                Ok(())
            }
            Self::Mask { bounds, path } => {
                validate_rect(*bounds, description)?;
                if path.is_absolute() {
                    bail!("{description} mask path must be relative to scene.toml");
                }
                Ok(())
            }
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Rect { .. } => "rect",
            Self::RoundedRect { .. } => "rounded-rect",
            Self::Ellipse { .. } => "ellipse",
            Self::Polygon { .. } => "polygon",
            Self::Mask { .. } => "mask",
        }
    }

    pub fn bounds(&self) -> [f64; 4] {
        match self {
            Self::Rect { bounds }
            | Self::RoundedRect { bounds, .. }
            | Self::Mask { bounds, .. } => *bounds,
            Self::Ellipse {
                center,
                radii,
                rotation_degrees,
            } => ellipse_bounds(*center, *radii, *rotation_degrees),
            Self::Polygon { points } => polygon_bounds(points),
        }
    }

    pub fn resolve(&self, scene_path: &Path) -> ResolvedWritableRegion {
        match self {
            Self::Rect { bounds } => ResolvedWritableRegion::Rect { bounds: *bounds },
            Self::RoundedRect { bounds, radius } => ResolvedWritableRegion::RoundedRect {
                bounds: *bounds,
                radius: *radius,
            },
            Self::Ellipse {
                center,
                radii,
                rotation_degrees,
            } => ResolvedWritableRegion::Ellipse {
                center: *center,
                radii: *radii,
                rotation_degrees: *rotation_degrees,
            },
            Self::Polygon { points } => ResolvedWritableRegion::Polygon {
                points: points.clone(),
            },
            Self::Mask { bounds, path } => ResolvedWritableRegion::Mask {
                bounds: *bounds,
                path: scene_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(path),
            },
        }
    }
}

impl ResolvedWritableRegion {
    pub fn bounds(&self) -> [f64; 4] {
        match self {
            Self::Rect { bounds }
            | Self::RoundedRect { bounds, .. }
            | Self::Mask { bounds, .. } => *bounds,
            Self::Ellipse {
                center,
                radii,
                rotation_degrees,
            } => ellipse_bounds(*center, *radii, *rotation_degrees),
            Self::Polygon { points } => polygon_bounds(points),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Rect { .. } => "rect",
            Self::RoundedRect { .. } => "rounded-rect",
            Self::Ellipse { .. } => "ellipse",
            Self::Polygon { .. } => "polygon",
            Self::Mask { .. } => "mask",
        }
    }

    /// Rasterize the declared source-space region into a canonical canvas whose
    /// placement is the region's own enclosing bounds.
    pub fn canonical_mask(&self, width: u32, height: u32) -> Result<Vec<u8>> {
        self.canonical_mask_in(self.bounds(), width, height)
    }

    /// Rasterize this region into a different enclosing tracking rectangle. This is
    /// what lets an outer injected plaque placement and its smaller inner writable
    /// region remain separate human concepts while sharing one universal mask.
    pub fn canonical_mask_in(
        &self,
        tracking_bounds: [f64; 4],
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>> {
        if width == 0 || height == 0 {
            bail!("canonical writable-region dimensions must be non-zero");
        }
        validate_rect(tracking_bounds, "tracking bounds")?;
        let tracking = RectF {
            x: tracking_bounds[0],
            y: tracking_bounds[1],
            width: tracking_bounds[2],
            height: tracking_bounds[3],
        };
        let mut mask = vec![0; width as usize * height as usize];

        match self {
            Self::Mask { bounds, path } => {
                let image = image::open(path)
                    .with_context(|| format!("failed to load writable mask {}", path.display()))?
                    .to_luma8();
                for y in 0..height {
                    for x in 0..width {
                        let [sx, sy] = source_point(tracking, width, height, x, y);
                        let u = (sx - bounds[0]) / bounds[2];
                        let v = (sy - bounds[1]) / bounds[3];
                        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
                            continue;
                        }
                        let ix = (u * image.width().saturating_sub(1) as f64).round() as u32;
                        let iy = (v * image.height().saturating_sub(1) as f64).round() as u32;
                        mask[(y * width + x) as usize] = image.get_pixel(ix, iy).0[0];
                    }
                }
            }
            _ => {
                for y in 0..height {
                    for x in 0..width {
                        let point = source_point(tracking, width, height, x, y);
                        if self.contains_source_point(point) {
                            mask[(y * width + x) as usize] = 255;
                        }
                    }
                }
            }
        }
        Ok(mask)
    }

    fn contains_source_point(&self, point: [f64; 2]) -> bool {
        match self {
            Self::Rect { bounds } => inside_rect(point, *bounds),
            Self::RoundedRect { bounds, radius } => inside_rounded_rect(
                point,
                RectF {
                    x: bounds[0],
                    y: bounds[1],
                    width: bounds[2],
                    height: bounds[3],
                },
                *radius,
            ),
            Self::Ellipse {
                center,
                radii,
                rotation_degrees,
            } => {
                let angle = rotation_degrees.to_radians();
                let dx = point[0] - center[0];
                let dy = point[1] - center[1];
                let local_x = dx * angle.cos() + dy * angle.sin();
                let local_y = -dx * angle.sin() + dy * angle.cos();
                (local_x / radii[0]).powi(2) + (local_y / radii[1]).powi(2) <= 1.0
            }
            Self::Polygon { points } => point_in_polygon(point, points),
            Self::Mask { .. } => false,
        }
    }
}

fn source_point(rect: RectF, width: u32, height: u32, x: u32, y: u32) -> [f64; 2] {
    [
        rect.x + (x as f64 + 0.5) * rect.width / width as f64,
        rect.y + (y as f64 + 0.5) * rect.height / height as f64,
    ]
}

fn inside_rect(point: [f64; 2], bounds: [f64; 4]) -> bool {
    point[0] >= bounds[0]
        && point[0] <= bounds[0] + bounds[2]
        && point[1] >= bounds[1]
        && point[1] <= bounds[1] + bounds[3]
}

fn inside_rounded_rect(point: [f64; 2], rect: RectF, radius: f64) -> bool {
    let left = rect.x;
    let right = rect.x + rect.width;
    let top = rect.y;
    let bottom = rect.y + rect.height;
    let x = point[0];
    let y = point[1];
    if x < left || x > right || y < top || y > bottom {
        return false;
    }
    if radius <= f64::EPSILON {
        return true;
    }
    let clamped_x = x.clamp(left + radius, right - radius);
    let clamped_y = y.clamp(top + radius, bottom - radius);
    (x - clamped_x).hypot(y - clamped_y) <= radius
}

fn point_in_polygon(point: [f64; 2], polygon: &[[f64; 2]]) -> bool {
    let mut inside = false;
    let mut previous = polygon.len() - 1;
    for current in 0..polygon.len() {
        let a = polygon[current];
        let b = polygon[previous];
        let crosses = (a[1] > point[1]) != (b[1] > point[1])
            && point[0] < (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1]) + a[0];
        if crosses {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

fn ellipse_bounds(center: [f64; 2], radii: [f64; 2], rotation_degrees: f64) -> [f64; 4] {
    let angle = rotation_degrees.to_radians();
    let cos = angle.cos();
    let sin = angle.sin();
    let half_width = ((radii[0] * cos).powi(2) + (radii[1] * sin).powi(2)).sqrt();
    let half_height = ((radii[0] * sin).powi(2) + (radii[1] * cos).powi(2)).sqrt();
    [
        center[0] - half_width,
        center[1] - half_height,
        half_width * 2.0,
        half_height * 2.0,
    ]
}

fn polygon_bounds(points: &[[f64; 2]]) -> [f64; 4] {
    let min_x = points.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min);
    let min_y = points.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
    let max_x = points
        .iter()
        .map(|p| p[0])
        .fold(f64::NEG_INFINITY, f64::max);
    let max_y = points
        .iter()
        .map(|p| p[1])
        .fold(f64::NEG_INFINITY, f64::max);
    [min_x, min_y, max_x - min_x, max_y - min_y]
}

fn polygon_area(points: &[[f64; 2]]) -> f64 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum::<f64>()
        * 0.5
}

fn validate_rect(rect: [f64; 4], description: &str) -> Result<()> {
    if rect.iter().any(|value| !value.is_finite()) {
        bail!("{description} contains a non-finite coordinate");
    }
    if rect[2] <= 0.0 || rect[3] <= 0.0 {
        bail!("{description} width and height must be positive");
    }
    Ok(())
}

fn validate_point(point: [f64; 2], description: &str) -> Result<()> {
    if point.iter().any(|value| !value.is_finite()) {
        bail!("{description} contains a non-finite coordinate");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ellipse_bounds_cover_the_unrotated_ellipse() {
        let bounds = ellipse_bounds([50.0, 40.0], [20.0, 10.0], 0.0);
        assert_eq!(bounds, [30.0, 30.0, 40.0, 20.0]);
    }

    #[test]
    fn inner_region_rasterizes_inside_a_larger_tracking_enclosure() {
        let region = ResolvedWritableRegion::Rect {
            bounds: [25.0, 25.0, 50.0, 50.0],
        };
        let mask = region
            .canonical_mask_in([0.0, 0.0, 100.0, 100.0], 100, 100)
            .unwrap();
        assert_eq!(mask[10 * 100 + 10], 0);
        assert_eq!(mask[50 * 100 + 50], 255);
    }

    #[test]
    fn zero_radius_rounded_rect_still_respects_its_own_bounds() {
        let region = ResolvedWritableRegion::RoundedRect {
            bounds: [25.0, 25.0, 50.0, 50.0],
            radius: 0.0,
        };
        let mask = region
            .canonical_mask_in([0.0, 0.0, 100.0, 100.0], 100, 100)
            .unwrap();
        assert_eq!(mask[10 * 100 + 10], 0);
        assert_eq!(mask[50 * 100 + 50], 255);
    }

    #[test]
    fn ellipse_mask_excludes_bounding_box_corners() {
        let region = ResolvedWritableRegion::Ellipse {
            center: [50.0, 50.0],
            radii: [50.0, 25.0],
            rotation_degrees: 0.0,
        };
        let mask = region.canonical_mask(100, 50).unwrap();
        assert_eq!(mask[0], 0);
        assert_eq!(mask[25 * 100 + 50], 255);
    }
}
