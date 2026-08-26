//! Diagnostics overlay generation.

use crate::surface::Surface;
use crate::color::Rgba;
use crate::geometry::Quad;

use super::state::OverlayMode;

/// Draw a yellow border around a quad on the given surface.
pub fn draw_quad_border(surface: &mut Surface, quad: Quad, color: Rgba, thickness: i32) {
    let points = quad.points();
    for i in 0..4 {
        let p0 = points[i];
        let p1 = points[(i + 1) % 4];
        draw_line(surface, p0, p1, color, thickness);
    }
}

fn draw_line(surface: &mut Surface, a: crate::geometry::Point, b: crate::geometry::Point, color: Rgba, thickness: i32) {
    let x0 = a.x.round() as i32;
    let y0 = a.y.round() as i32;
    let x1 = b.x.round() as i32;
    let y1 = b.y.round() as i32;
    let dx = (x1 - x0).abs();
    let dy = (y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx - dy;
    let mut x = x0;
    let mut y = y0;
    loop {
        for tx in -thickness/2..=thickness/2 {
            for ty in -thickness/2..=thickness/2 {
                surface.blend_pixel(x + tx, y + ty, color, 1.0);
            }
        }
        if x == x1 && y == y1 { break; }
        let e2 = 2 * err;
        if e2 > -dy { err -= dy; x += sx; }
        if e2 < dx { err += dx; y += sy; }
    }
}

/// Fill mask pixels with solid green (or overlay color) where mask > 128.
pub fn fill_mask_overlay(surface: &mut Surface, mask: &[u8], color: Rgba) {
    assert_eq!(mask.len(), surface.width() as usize * surface.height() as usize);
    for (i, &m) in mask.iter().enumerate() {
        if m > 32 {
            let x = (i % surface.width() as usize) as i32;
            let y = (i / surface.width() as usize) as i32;
            let alpha = (m as f32 / 255.0 * color.a as f32 / 255.0).clamp(0.0, 1.0);
            // blend with mask alpha
            surface.blend_pixel(x, y, Rgba::new(color.r, color.g, color.b, (alpha*255.0) as u8), alpha);
        }
    }
}

/// Convert surface to greyscale in place.
pub fn to_greyscale(surface: &mut Surface) {
    for y in 0..surface.height() {
        for x in 0..surface.width() {
            let p = surface.pixel(x, y);
            let l = (0.299 * p.r as f32 + 0.587 * p.g as f32 + 0.114 * p.b as f32) as u8;
            surface.set_pixel(x, y, Rgba::new(l, l, l, p.a));
        }
    }
}

/// Blend text notice onto surface (centered) when analysis missing.
pub fn draw_missing_analysis_notice(surface: &mut Surface) {
    // Greyscale already done; add semi-transparent dark overlay then text via simple pixel text.
    // We draw a dark bar in the middle and white text approximated via rectangles.
    let w = surface.width() as i32;
    let h = surface.height() as i32;
    let bar_h = 60;
    let y0 = h/2 - bar_h/2;
    for y in y0..y0+bar_h {
        for x in 0..w {
            surface.blend_pixel(x, y, Rgba::new(0, 0, 0, 160), 0.6);
        }
    }
    // No font rasterization available here without cosmic-text; we rely on UI overlay text instead.
    // This function only does greyscale + dark bar; egui will render the two lines.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::Surface;
    use crate::color::Rgba;
    use crate::geometry::{Quad, Point};

    #[test]
    fn greyscale_converts_color() {
        let mut s = Surface::new(2, 1);
        s.set_pixel(0, 0, Rgba::new(255, 0, 0, 255));
        to_greyscale(&mut s);
        let p = s.pixel(0, 0);
        assert_eq!(p.r, p.g);
        assert_eq!(p.g, p.b);
    }

    #[test]
    fn quad_border_draws_yellow() {
        let mut s = Surface::new(10, 10);
        let quad = Quad::new(Point::new(1.0,1.0), Point::new(8.0,1.0), Point::new(8.0,8.0), Point::new(1.0,8.0));
        draw_quad_border(&mut s, quad, Rgba::new(255,255,0,255), 1);
        // corner should be yellow
        let p = s.pixel(1, 1);
        assert!(p.r > 200 && p.g > 200);
    }

    #[test]
    fn mask_overlay_fills_green() {
        let mut s = Surface::new(2, 2);
        let mask = vec![0, 255, 0, 128];
        fill_mask_overlay(&mut s, &mask, Rgba::new(0,255,0,255));
        let p = s.pixel(1, 0);
        assert!(p.g > 100);
    }
}
