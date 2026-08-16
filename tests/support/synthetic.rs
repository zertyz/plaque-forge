//! Fast in-memory synthetic sequence generator for deterministic subsystem tests.
//!
//! Provides mathematically grounded synthetic video frames with known analytical
//! quad trajectories, textures, and foreground occlusions.

use anyhow::Result;
use plaque_forge::{
    color::Rgba,
    geometry::{Point, Quad, homography},
    model::Mat3,
    surface::Surface,
};

/// Type of synthetic background pattern.
#[derive(Debug, Clone, Copy)]
pub enum BackgroundPattern {
    /// Alternating high-contrast checkerboard for rich optical flow features.
    Checkerboard { block_size: u32 },
    /// Smooth directional diagonal gradient.
    DiagonalGradient,
    /// High-frequency textured grid with corner anchors.
    TexturedGrid { grid_spacing: u32 },
}

/// A synthetic foreground object that crosses in front of the tracked plaque.
#[derive(Debug, Clone)]
pub struct SyntheticOccluder {
    pub start_frame: usize,
    pub end_frame: usize,
    pub width: f64,
    pub height: f64,
    pub start_pos: Point,
    pub end_pos: Point,
    pub color: Rgba,
}

impl SyntheticOccluder {
    pub fn new(
        start_frame: usize,
        end_frame: usize,
        width: f64,
        height: f64,
        start_pos: Point,
        end_pos: Point,
    ) -> Self {
        Self {
            start_frame,
            end_frame,
            width,
            height,
            start_pos,
            end_pos,
            color: Rgba::new(20, 20, 20, 255),
        }
    }

    pub fn position_at(&self, frame: usize) -> Option<Point> {
        if frame < self.start_frame || frame > self.end_frame {
            return None;
        }
        let total = (self.end_frame - self.start_frame).max(1) as f64;
        let t = (frame - self.start_frame) as f64 / total;
        Some(self.start_pos.lerp(self.end_pos, t))
    }
}

/// Motion model for generating synthetic ground-truth quad trajectories.
#[derive(Clone)]
pub enum SyntheticMotion {
    /// Quad is static across all frames.
    Static,
    /// Quad undergoes constant linear translation per frame.
    LinearTranslation {
        dx_per_frame: f64,
        dy_per_frame: f64,
    },
    /// Quad rotates around its centroid.
    Rotation { radians_per_frame: f64 },
    /// Quad scales uniformly from its centroid.
    Scaling { scale_factor_per_frame: f64 },
    /// Custom analytical motion function: (frame, total_frames, base_quad) -> Quad.
    Custom(std::sync::Arc<dyn Fn(usize, usize, Quad) -> Quad + Send + Sync>),
}

/// In-memory builder for synthetic video sequences with exact ground truth.
pub struct SyntheticSequenceBuilder {
    pub width: u32,
    pub height: u32,
    pub frame_count: usize,
    pub base_quad: Quad,
    pub motion: SyntheticMotion,
    pub background: BackgroundPattern,
    pub plaque_color: Rgba,
    pub border_color: Rgba,
    pub occluder: Option<SyntheticOccluder>,
}

impl SyntheticSequenceBuilder {
    pub fn new(width: u32, height: u32, frame_count: usize) -> Self {
        let margin_x = width as f64 * 0.2;
        let margin_y = height as f64 * 0.2;
        let base_quad = Quad::from_rect(
            margin_x,
            margin_y,
            width as f64 - margin_x * 2.0,
            height as f64 - margin_y * 2.0,
        );

        Self {
            width,
            height,
            frame_count,
            base_quad,
            motion: SyntheticMotion::Static,
            background: BackgroundPattern::Checkerboard { block_size: 16 },
            plaque_color: Rgba::new(200, 160, 60, 255),
            border_color: Rgba::new(80, 50, 20, 255),
            occluder: None,
        }
    }

    pub fn with_base_quad(mut self, quad: Quad) -> Self {
        self.base_quad = quad;
        self
    }

    pub fn with_motion(mut self, motion: SyntheticMotion) -> Self {
        self.motion = motion;
        self
    }

    pub fn with_background(mut self, background: BackgroundPattern) -> Self {
        self.background = background;
        self
    }

    pub fn with_occluder(mut self, occluder: SyntheticOccluder) -> Self {
        self.occluder = Some(occluder);
        self
    }

    /// Analytical ground-truth quad at a specific frame index.
    pub fn ground_truth_quad(&self, frame: usize) -> Quad {
        match &self.motion {
            SyntheticMotion::Static => self.base_quad,
            SyntheticMotion::LinearTranslation {
                dx_per_frame,
                dy_per_frame,
            } => {
                let dx = *dx_per_frame * frame as f64;
                let dy = *dy_per_frame * frame as f64;
                Quad::new(
                    Point::new(self.base_quad.tl.x + dx, self.base_quad.tl.y + dy),
                    Point::new(self.base_quad.tr.x + dx, self.base_quad.tr.y + dy),
                    Point::new(self.base_quad.br.x + dx, self.base_quad.br.y + dy),
                    Point::new(self.base_quad.bl.x + dx, self.base_quad.bl.y + dy),
                )
            }
            SyntheticMotion::Rotation { radians_per_frame } => {
                let angle = *radians_per_frame * frame as f64;
                let (sin, cos) = angle.sin_cos();
                let center_x = (self.base_quad.tl.x + self.base_quad.br.x) * 0.5;
                let center_y = (self.base_quad.tl.y + self.base_quad.br.y) * 0.5;

                let rotate = |p: Point| -> Point {
                    let rx = p.x - center_x;
                    let ry = p.y - center_y;
                    Point::new(
                        center_x + rx * cos - ry * sin,
                        center_y + rx * sin + ry * cos,
                    )
                };

                Quad::new(
                    rotate(self.base_quad.tl),
                    rotate(self.base_quad.tr),
                    rotate(self.base_quad.br),
                    rotate(self.base_quad.bl),
                )
            }
            SyntheticMotion::Scaling {
                scale_factor_per_frame,
            } => {
                let scale = 1.0 + (*scale_factor_per_frame * frame as f64);
                let center_x = (self.base_quad.tl.x + self.base_quad.br.x) * 0.5;
                let center_y = (self.base_quad.tl.y + self.base_quad.br.y) * 0.5;

                let scale_point = |p: Point| -> Point {
                    Point::new(
                        center_x + (p.x - center_x) * scale,
                        center_y + (p.y - center_y) * scale,
                    )
                };

                Quad::new(
                    scale_point(self.base_quad.tl),
                    scale_point(self.base_quad.tr),
                    scale_point(self.base_quad.br),
                    scale_point(self.base_quad.bl),
                )
            }
            SyntheticMotion::Custom(func) => func(frame, self.frame_count, self.base_quad),
        }
    }

    /// Analytical homography transforming the base quad to frame `target_frame`.
    pub fn ground_truth_homography(&self, from_frame: usize, to_frame: usize) -> Result<Mat3> {
        let src = self.ground_truth_quad(from_frame);
        let dst = self.ground_truth_quad(to_frame);
        let h = homography(src, dst)?;
        Ok(Mat3 {
            values: [
                [h.m[0][0], h.m[0][1], h.m[0][2]],
                [h.m[1][0], h.m[1][1], h.m[1][2]],
                [h.m[2][0], h.m[2][1], h.m[2][2]],
            ],
        })
    }

    /// Render all synthetic sequence frames into memory.
    pub fn build_frames(&self) -> Vec<Surface> {
        let mut frames = Vec::with_capacity(self.frame_count);
        for frame_idx in 0..self.frame_count {
            frames.push(self.render_frame(frame_idx));
        }
        frames
    }

    /// Render a single synthetic frame.
    pub fn render_frame(&self, frame: usize) -> Surface {
        let mut surface = Surface::new(self.width, self.height);

        // 1. Draw background
        for y in 0..self.height {
            for x in 0..self.width {
                let bg_color = match self.background {
                    BackgroundPattern::Checkerboard { block_size } => {
                        let check = ((x / block_size) + (y / block_size)) % 2 == 0;
                        if check {
                            Rgba::new(45, 50, 60, 255)
                        } else {
                            Rgba::new(90, 100, 115, 255)
                        }
                    }
                    BackgroundPattern::DiagonalGradient => {
                        let factor =
                            ((x + y) as f64 / (self.width + self.height) as f64).clamp(0.0, 1.0);
                        let val = (40.0 + factor * 140.0) as u8;
                        Rgba::new(val, val + 10, val + 20, 255)
                    }
                    BackgroundPattern::TexturedGrid { grid_spacing } => {
                        if x % grid_spacing == 0 || y % grid_spacing == 0 {
                            Rgba::new(140, 150, 160, 255)
                        } else {
                            Rgba::new(30, 35, 40, 255)
                        }
                    }
                };
                surface.set_pixel(x, y, bg_color);
            }
        }

        // 2. Rasterize the moving plaque quad
        let quad = self.ground_truth_quad(frame);
        self.draw_quad(&mut surface, quad);

        // 3. Draw foreground occluder if active on this frame
        if let Some(ref occluder) = self.occluder
            && let Some(pos) = occluder.position_at(frame)
        {
            self.draw_occluder(
                &mut surface,
                pos,
                occluder.width,
                occluder.height,
                occluder.color,
            );
        }

        surface
    }

    fn draw_quad(&self, surface: &mut Surface, quad: Quad) {
        let (min_x, min_y, max_x, max_y) = quad.bounds();
        let min_xi = (min_x.floor() as i32).max(0) as u32;
        let min_yi = (min_y.floor() as i32).max(0) as u32;
        let max_xi = ((max_x.ceil() as u32).min(self.width - 1)).max(min_xi);
        let max_yi = ((max_y.ceil() as u32).min(self.height - 1)).max(min_yi);

        for y in min_yi..=max_yi {
            for x in min_xi..=max_xi {
                let pt = Point::new(x as f64 + 0.5, y as f64 + 0.5);
                if point_in_quad(pt, quad) {
                    let is_border = distance_to_quad_edge(pt, quad) < 3.0;
                    let color = if is_border {
                        self.border_color
                    } else {
                        // Subtle interior gradient for feature tracking richness
                        let rel_x = ((x as f64 - min_x) / (max_x - min_x).max(1.0)).clamp(0.0, 1.0);
                        let r = (self.plaque_color.r as f64 * (0.8 + rel_x * 0.4)).min(255.0) as u8;
                        Rgba::new(r, self.plaque_color.g, self.plaque_color.b, 255)
                    };
                    surface.set_pixel(x, y, color);
                }
            }
        }
    }

    fn draw_occluder(&self, surface: &mut Surface, pos: Point, w: f64, h: f64, color: Rgba) {
        let min_x = (pos.x.max(0.0) as u32).min(self.width);
        let min_y = (pos.y.max(0.0) as u32).min(self.height);
        let max_x = ((pos.x + w).min(self.width as f64) as u32).max(min_x);
        let max_y = ((pos.y + h).min(self.height as f64) as u32).max(min_y);

        for y in min_y..max_y {
            for x in min_x..max_x {
                surface.set_pixel(x, y, color);
            }
        }
    }
}

/// Point-in-polygon test for convex quadrilateral using cross products.
fn point_in_quad(p: Point, q: Quad) -> bool {
    let points = q.points();
    let mut sign = 0.0;
    for i in 0..4 {
        let a = points[i];
        let b = points[(i + 1) % 4];
        let cross = (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
        if cross.abs() < 1e-9 {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}

/// Minimum perpendicular distance from a point to the quad's edges.
fn distance_to_quad_edge(p: Point, q: Quad) -> f64 {
    let points = q.points();
    let mut min_dist = f64::INFINITY;
    for i in 0..4 {
        let a = points[i];
        let b = points[(i + 1) % 4];
        let ab_x = b.x - a.x;
        let ab_y = b.y - a.y;
        let length = ab_x.hypot(ab_y);
        if length > 1e-9 {
            let dist = ((p.x - a.x) * ab_y - (p.y - a.y) * ab_x).abs() / length;
            min_dist = min_dist.min(dist);
        }
    }
    min_dist
}
