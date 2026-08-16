use anyhow::{Result, bail};

use crate::{color::Rgba, surface::Surface};

pub(super) fn arc_warp(source: &Surface, sweep_degrees: f32, radius_scale: f32) -> Surface {
    let Some(bounds) = source.alpha_bounds() else {
        return source.clone();
    };
    if sweep_degrees.abs() < 0.5 {
        return source.clone();
    }
    let width = (bounds.2 - bounds.0 + 1).max(1) as f32;
    let height = (bounds.3 - bounds.1 + 1).max(1) as f32;
    let sweep = sweep_degrees.to_radians().clamp(-5.8, 5.8);
    let radius = (width / sweep.abs().max(0.05) * radius_scale.max(0.2)).max(height * 0.8);
    let center_x = (bounds.0 + bounds.2) as f32 * 0.5;
    let center_y = (bounds.1 + bounds.3) as f32 * 0.5;
    let mut output = Surface::new(source.width(), source.height());

    // Warp the already-shaped, supersampled title strip onto a circular baseline.
    // This preserves shaping/kerning while bending the complete glyph coverage before
    // materials, outlines and bevels are applied.
    for y in bounds.1..=bounds.3 {
        for x in bounds.0..=bounds.2 {
            let pixel = source.pixel(x, y);
            if pixel.a == 0 {
                continue;
            }
            let nx = (x as f32 - center_x) / width;
            let angle = nx * sweep;
            let local_y = y as f32 - center_y;
            let (sin, cos) = angle.sin_cos();
            let baseline_x = center_x + sin * radius;
            let baseline_y = center_y + sweep.signum() * radius * (1.0 - cos);
            let tx = baseline_x - local_y * sin;
            let ty = baseline_y + local_y * cos;
            splat(&mut output, tx, ty, pixel);
        }
    }
    output
}

pub(super) fn rotate_surface(source: &Surface, degrees: f32) -> Surface {
    if degrees.abs() < 0.01 {
        return source.clone();
    }
    let Some(bounds) = source.alpha_bounds() else {
        return source.clone();
    };
    let cx = (bounds.0 + bounds.2) as f32 * 0.5;
    let cy = (bounds.1 + bounds.3) as f32 * 0.5;
    let angle = degrees.to_radians();
    let (sin, cos) = angle.sin_cos();
    let mut output = Surface::new(source.width(), source.height());
    for y in bounds.1..=bounds.3 {
        for x in bounds.0..=bounds.2 {
            let pixel = source.pixel(x, y);
            if pixel.a == 0 {
                continue;
            }
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let tx = cx + dx * cos - dy * sin;
            let ty = cy + dx * sin + dy * cos;
            splat(&mut output, tx, ty, pixel);
        }
    }
    output
}

pub(super) fn glitch_surface(
    source: &Surface,
    time_seconds: f64,
    period_seconds: f32,
    ripple_ratio: f32,
    slice_ratio: f32,
    burst_fraction: f32,
    seed: u32,
) -> Surface {
    let Some(bounds) = source.alpha_bounds() else {
        return source.clone();
    };
    let width = (bounds.2 - bounds.0 + 1).max(1) as f32;
    let height = (bounds.3 - bounds.1 + 1).max(1) as f32;
    let phase = (time_seconds / period_seconds as f64).rem_euclid(1.0) as f32;
    let burst = phase < burst_fraction;
    let tick = (time_seconds * 24.0).floor().max(0.0) as u32;
    let ripple = width * ripple_ratio;
    let max_slice = width * slice_ratio;
    let mut out = Surface::new(source.width(), source.height());
    for y in 0..source.height() {
        let ny = if height > 0.0 {
            (y as f32 - bounds.1 as f32) / height
        } else {
            0.0
        };
        let gentle =
            ripple * (ny * std::f32::consts::TAU * 3.0 + phase * std::f32::consts::TAU).sin();
        let band = y / 7;
        let h = hash3(seed ^ tick, band, 0);
        let slice = if burst && (h & 0x7) <= 2 {
            (((h >> 8) & 0xffff) as f32 / 65535.0 * 2.0 - 1.0) * max_slice
        } else {
            0.0
        };
        let shift = gentle + slice;
        for x in 0..source.width() {
            let sx = (x as f32 - shift).round() as i32;
            if sx < 0 || sx >= source.width() as i32 {
                continue;
            }
            let mut p = source.pixel(sx as u32, y);
            if burst && slice.abs() > 0.5 {
                // Small deterministic channel skew keeps the distortion visibly "glitch" rather
                // than just a horizontal ripple.
                let red_x = (sx - 2).clamp(0, source.width() as i32 - 1) as u32;
                let blue_x = (sx + 2).clamp(0, source.width() as i32 - 1) as u32;
                p.r = source.pixel(red_x, y).r;
                p.b = source.pixel(blue_x, y).b;
            }
            out.set_pixel(x, y, p);
        }
    }
    out
}

pub(super) fn confetti_converge(
    source: &Surface,
    progress: f32,
    pieces: u32,
    spread_ratio: f32,
    seed: u32,
) -> Surface {
    let progress = smoothstep(progress.clamp(0.0, 1.0));
    if progress >= 0.995 {
        return source.clone();
    }
    let Some(bounds) = source.alpha_bounds() else {
        return source.clone();
    };
    let width = (bounds.2 - bounds.0 + 1).max(1) as f32;
    let height = (bounds.3 - bounds.1 + 1).max(1) as f32;
    let area = (width * height).max(1.0);
    let step = (area / pieces.max(16) as f32).sqrt().clamp(2.0, 24.0) as u32;
    let spread = width.max(height) * spread_ratio.max(0.05);
    let mut output = Surface::new(source.width(), source.height());

    for y in (bounds.1..=bounds.3).step_by(step as usize) {
        for x in (bounds.0..=bounds.2).step_by(step as usize) {
            let pixel = source.pixel(x, y);
            if pixel.a == 0 {
                continue;
            }
            let h = hash3(seed, x, y);
            let angle = ((h & 0xffff) as f32 / 65535.0) * std::f32::consts::TAU;
            let radius = (((h >> 16) & 0xffff) as f32 / 65535.0).sqrt() * spread;
            let sx = x as f32 + angle.cos() * radius;
            let sy = y as f32 + angle.sin() * radius;
            let tx = sx + (x as f32 - sx) * progress;
            let ty = sy + (y as f32 - sy) * progress;
            let size = 1 + ((h >> 28) & 0x3) as i32;
            let vivid = Rgba::new(
                pixel.r.saturating_add(((h >> 8) & 0x1f) as u8),
                pixel.g.saturating_add(((h >> 13) & 0x1f) as u8),
                pixel.b.saturating_add(((h >> 18) & 0x1f) as u8),
                pixel.a,
            );
            for oy in -size..=size {
                for ox in -size..=size {
                    if ox.abs() + oy.abs() > size + 1 {
                        continue;
                    }
                    output.blend_pixel(tx.round() as i32 + ox, ty.round() as i32 + oy, vivid, 1.0);
                }
            }
        }
    }
    output
}

pub(super) fn paint_texture(
    base: &Surface,
    texture: &Surface,
    tile: bool,
    scale: f32,
    offset_x: f32,
    offset_y: f32,
) -> Result<Surface> {
    if texture.width() == 0 || texture.height() == 0 {
        bail!("text texture is empty");
    }
    let alpha = base.alpha_mask();
    let Some(bounds) = base.alpha_bounds() else {
        return Ok(base.clone());
    };
    let bw = (bounds.2 - bounds.0 + 1).max(1) as f32;
    let bh = (bounds.3 - bounds.1 + 1).max(1) as f32;
    let scale = scale.max(0.01);
    let mut out = Surface::new(base.width(), base.height());
    for y in bounds.1..=bounds.3 {
        for x in bounds.0..=bounds.2 {
            let coverage = alpha[(y * base.width() + x) as usize];
            if coverage == 0 {
                continue;
            }
            let nx = (x - bounds.0) as f32 / bw;
            let ny = (y - bounds.1) as f32 / bh;
            let (u, v) = if tile {
                (
                    (nx / scale + offset_x).rem_euclid(1.0),
                    (ny / scale + offset_y).rem_euclid(1.0),
                )
            } else {
                (
                    ((nx - 0.5) / scale + 0.5 + offset_x).clamp(0.0, 1.0),
                    ((ny - 0.5) / scale + 0.5 + offset_y).clamp(0.0, 1.0),
                )
            };
            let sx = (u * texture.width().saturating_sub(1) as f32).round() as u32;
            let sy = (v * texture.height().saturating_sub(1) as f32).round() as u32;
            let p = texture.pixel(sx, sy);
            let a = ((coverage as u16 * p.a as u16 + 127) / 255) as u8;
            out.set_pixel(x, y, Rgba::new(p.r, p.g, p.b, a));
        }
    }
    Ok(out)
}

pub(super) fn paint_blueprint(
    base: &Surface,
    dark: Rgba,
    light: Rgba,
    grid: Rgba,
    cell: u32,
) -> Result<Surface> {
    let Some(bounds) = base.alpha_bounds() else {
        return Ok(base.clone());
    };
    let alpha = base.alpha_mask();
    let mut out = Surface::new(base.width(), base.height());
    let cell = cell.max(3);
    for y in bounds.1..=bounds.3 {
        for x in bounds.0..=bounds.2 {
            let coverage = alpha[(y * base.width() + x) as usize];
            if coverage == 0 {
                continue;
            }
            let t = (y - bounds.1) as f32 / (bounds.3 - bounds.1 + 1).max(1) as f32;
            let mut c = mix(dark, light, (0.25 + t * 0.55).clamp(0.0, 1.0));
            if x % cell == 0 || y % cell == 0 || (x + y) % (cell * 3) == 0 {
                c = mix(c, grid, 0.55);
            }
            c.a = ((c.a as u16 * coverage as u16 + 127) / 255) as u8;
            out.set_pixel(x, y, c);
        }
    }
    Ok(out)
}

pub(super) fn paint_paper(
    base: &Surface,
    light: Rgba,
    mid: Rgba,
    dark: Rgba,
    seed: u32,
) -> Result<Surface> {
    let Some(bounds) = base.alpha_bounds() else {
        return Ok(base.clone());
    };
    let alpha = base.alpha_mask();
    let mut out = Surface::new(base.width(), base.height());
    for y in bounds.1..=bounds.3 {
        for x in bounds.0..=bounds.2 {
            let coverage = alpha[(y * base.width() + x) as usize];
            if coverage == 0 {
                continue;
            }
            let h = hash3(seed, x, y);
            let noise = (h & 0xff) as f32 / 255.0;
            let fiber = (((x as f32 * 0.14).sin() + (y as f32 * 0.47).cos()) * 0.5 + 0.5) * 0.22;
            let t = (noise * 0.65 + fiber).clamp(0.0, 1.0);
            let mut c = if t < 0.6 {
                mix(light, mid, t / 0.6)
            } else {
                mix(mid, dark, (t - 0.6) / 0.4)
            };
            c.a = ((c.a as u16 * coverage as u16 + 127) / 255) as u8;
            out.set_pixel(x, y, c);
        }
    }
    Ok(out)
}

pub(super) fn laser_burn_overlay(
    plaque: &Surface,
    glyph_mask: &[u8],
    depth: f32,
    warmth: f32,
    edge_width: u32,
    seed: u32,
) -> Result<Surface> {
    validate_mask(plaque, glyph_mask)?;
    let w = plaque.width() as usize;
    let h = plaque.height() as usize;
    let expanded = dilate(glyph_mask, w, h, edge_width.max(1) as usize);
    let mut out = Surface::new(plaque.width(), plaque.height());
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let ink = glyph_mask[i] as f32 / 255.0;
            let rim = (expanded[i].saturating_sub(glyph_mask[i])) as f32 / 255.0;
            if ink <= 0.0 && rim <= 0.0 {
                continue;
            }
            let p = plaque.pixel(x as u32, y as u32);
            let grain = ((hash3(seed, x as u32, y as u32) & 0xff) as f32 / 255.0 - 0.5) * 0.18;
            let burn = (depth.clamp(0.0, 1.0) * ink * (1.0 + grain)).clamp(0.0, 0.92);
            let warm = warmth.clamp(0.0, 1.0) * ink;
            let mut r = p.r as f32 * (1.0 - burn) + 68.0 * burn + 48.0 * warm;
            let mut g = p.g as f32 * (1.0 - burn) + 24.0 * burn + 12.0 * warm;
            let mut b = p.b as f32 * (1.0 - burn) + 8.0 * burn;
            if rim > 0.0 {
                let char = 0.38 * rim;
                r *= 1.0 - char;
                g *= 1.0 - char;
                b *= 1.0 - char;
            }
            out.set_pixel(
                x as u32,
                y as u32,
                Rgba::new(
                    r.clamp(0.0, 255.0) as u8,
                    g.clamp(0.0, 255.0) as u8,
                    b.clamp(0.0, 255.0) as u8,
                    ((ink.max(rim) * 255.0).round()) as u8,
                ),
            );
        }
    }
    Ok(out)
}

pub(super) fn emboss_overlay(
    plaque: &Surface,
    glyph_mask: &[u8],
    depth: f32,
    highlight_strength: f32,
    shadow_strength: f32,
    light_angle_degrees: Option<f32>,
    cast_shadow: u32,
) -> Result<Surface> {
    validate_mask(plaque, glyph_mask)?;
    let w = plaque.width() as usize;
    let h = plaque.height() as usize;
    let angle = light_angle_degrees
        .unwrap_or_else(|| estimate_light_angle(plaque))
        .to_radians();
    let lx = angle.cos();
    let ly = angle.sin();
    let mut out = Surface::new(plaque.width(), plaque.height());
    let cast_dx = (-lx * cast_shadow as f32).round() as i32;
    let cast_dy = (-ly * cast_shadow as f32).round() as i32;

    if cast_shadow > 0 {
        for y in 0..h {
            for x in 0..w {
                let a = glyph_mask[y * w + x];
                if a == 0 {
                    continue;
                }
                out.blend_pixel(
                    x as i32 + cast_dx,
                    y as i32 + cast_dy,
                    Rgba::new(
                        0,
                        0,
                        0,
                        (a as f32 * shadow_strength.clamp(0.0, 1.0) * 0.55) as u8,
                    ),
                    1.0,
                );
            }
        }
    }

    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let a = glyph_mask[i] as f32 / 255.0;
            if a <= 0.0 {
                continue;
            }
            let left = sample_mask(glyph_mask, w, h, x as i32 - 1, y as i32) as f32 / 255.0;
            let right = sample_mask(glyph_mask, w, h, x as i32 + 1, y as i32) as f32 / 255.0;
            let up = sample_mask(glyph_mask, w, h, x as i32, y as i32 - 1) as f32 / 255.0;
            let down = sample_mask(glyph_mask, w, h, x as i32, y as i32 + 1) as f32 / 255.0;
            let nx = (left - right) * depth.max(0.0) * 4.0;
            let ny = (up - down) * depth.max(0.0) * 4.0;
            let diffuse = (nx * lx + ny * ly).clamp(-1.0, 1.0);
            let p = plaque.pixel(x as u32, y as u32);
            let factor = if diffuse >= 0.0 {
                1.0 + diffuse * highlight_strength.clamp(0.0, 1.5)
            } else {
                1.0 + diffuse * shadow_strength.clamp(0.0, 1.5)
            };
            let r = (p.r as f32 * factor).clamp(0.0, 255.0) as u8;
            let g = (p.g as f32 * factor).clamp(0.0, 255.0) as u8;
            let b = (p.b as f32 * factor).clamp(0.0, 255.0) as u8;
            out.set_pixel(x as u32, y as u32, Rgba::new(r, g, b, (a * 255.0) as u8));
        }
    }
    Ok(out)
}

fn estimate_light_angle(plaque: &Surface) -> f32 {
    if plaque.width() < 3 || plaque.height() < 3 {
        return 315.0;
    }
    let mut gx = 0.0_f64;
    let mut gy = 0.0_f64;
    let mut samples = 0_u64;
    for y in (1..plaque.height() - 1).step_by(3) {
        for x in (1..plaque.width() - 1).step_by(3) {
            let l = luminance(plaque.pixel(x - 1, y));
            let r = luminance(plaque.pixel(x + 1, y));
            let u = luminance(plaque.pixel(x, y - 1));
            let d = luminance(plaque.pixel(x, y + 1));
            gx += r - l;
            gy += d - u;
            samples += 1;
        }
    }
    if samples == 0 || (gx.abs() + gy.abs()) < 1e-6 {
        315.0
    } else {
        gy.atan2(gx).to_degrees() as f32
    }
}

fn luminance(color: Rgba) -> f64 {
    color.r as f64 * 0.2126 + color.g as f64 * 0.7152 + color.b as f64 * 0.0722
}

fn validate_mask(surface: &Surface, mask: &[u8]) -> Result<()> {
    if mask.len() != surface.width() as usize * surface.height() as usize {
        bail!("surface-effect mask dimensions do not match plaque canvas");
    }
    Ok(())
}

fn dilate(source: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    if radius == 0 {
        return source.to_vec();
    }
    let mut out = vec![0u8; source.len()];
    for y in 0..height {
        for x in 0..width {
            let mut m = 0u8;
            for oy in -(radius as i32)..=radius as i32 {
                for ox in -(radius as i32)..=radius as i32 {
                    if ox * ox + oy * oy > (radius * radius) as i32 {
                        continue;
                    }
                    m = m.max(sample_mask(
                        source,
                        width,
                        height,
                        x as i32 + ox,
                        y as i32 + oy,
                    ));
                }
            }
            out[y * width + x] = m;
        }
    }
    out
}

fn sample_mask(source: &[u8], width: usize, height: usize, x: i32, y: i32) -> u8 {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        0
    } else {
        source[y as usize * width + x as usize]
    }
}

fn splat(output: &mut Surface, x: f32, y: f32, pixel: Rgba) {
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let fx = x - x.floor();
    let fy = y - y.floor();
    for (dx, dy, weight) in [
        (0, 0, (1.0 - fx) * (1.0 - fy)),
        (1, 0, fx * (1.0 - fy)),
        (0, 1, (1.0 - fx) * fy),
        (1, 1, fx * fy),
    ] {
        if weight > 0.0 {
            output.blend_pixel(x0 + dx, y0 + dy, pixel, weight);
        }
    }
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn hash3(seed: u32, x: u32, y: u32) -> u32 {
    let mut h = seed ^ x.wrapping_mul(0x9E37_79B9) ^ y.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^ (h >> 16)
}

fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    let c = |x: u8, y: u8| {
        (x as f32 + (y as f32 - x as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Rgba::new(c(a.r, b.r), c(a.g, b.g), c(a.b, b.b), c(a.a, b.a))
}

#[cfg(test)]
mod tests {
    use super::{arc_warp, laser_burn_overlay, paint_texture};
    use crate::{color::Rgba, surface::Surface};

    #[test]
    fn arc_warp_preserves_visible_coverage() {
        let mut source = Surface::new(48, 32);
        for y in 13..19 {
            for x in 8..40 {
                source.set_pixel(x, y, Rgba::new(255, 255, 255, 255));
            }
        }
        let warped = arc_warp(&source, 70.0, 1.0);
        assert!(warped.alpha_bounds().is_some());
        assert_ne!(warped.alpha_mask(), source.alpha_mask());
    }

    #[test]
    fn image_texture_never_paints_outside_glyph_alpha() {
        let mut base = Surface::new(8, 8);
        base.set_pixel(3, 3, Rgba::new(255, 255, 255, 255));
        let mut texture = Surface::new(2, 2);
        for y in 0..2 {
            for x in 0..2 {
                texture.set_pixel(x, y, Rgba::new(180, 120, 40, 255));
            }
        }
        let painted = paint_texture(&base, &texture, true, 1.0, 0.0, 0.0).unwrap();
        assert_eq!(painted.pixel(0, 0).a, 0);
        assert_eq!(painted.pixel(3, 3).a, 255);
    }

    #[test]
    fn laser_burn_uses_plaque_color_as_its_source() {
        let mut plaque = Surface::new(5, 5);
        for y in 0..5 {
            for x in 0..5 {
                plaque.set_pixel(x, y, Rgba::new(170, 130, 80, 255));
            }
        }
        let mut mask = vec![0_u8; 25];
        mask[12] = 255;
        let burn = laser_burn_overlay(&plaque, &mask, 0.8, 0.6, 1, 7).unwrap();
        let source = plaque.pixel(2, 2);
        let burned = burn.pixel(2, 2);
        assert!(burned.a > 0);
        assert_ne!(
            (burned.r, burned.g, burned.b),
            (source.r, source.g, source.b)
        );
    }
}
