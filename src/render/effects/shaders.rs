//! Material shaders, color ramps, procedurals, and text animations.

use crate::{color::Rgba, surface::Surface};
use anyhow::{Context, Result, bail};

pub fn paint_vertical_material(base: &Surface, color_at: impl Fn(f32) -> Rgba) -> Result<Surface> {
    let alpha = base.alpha_mask();
    let bounds = base
        .alpha_bounds()
        .context("text material has no visible glyphs")?;
    let height = (bounds.3 - bounds.1).max(1) as f32;
    let mut surface = Surface::new(base.width(), base.height());
    for y in bounds.1..=bounds.3 {
        let t = (y - bounds.1) as f32 / height;
        let color = color_at(t.clamp(0.0, 1.0));
        for x in bounds.0..=bounds.2 {
            let coverage = alpha[(y * base.width() + x) as usize];
            if coverage == 0 {
                continue;
            }
            let a = ((coverage as u16 * color.a as u16 + 127) / 255) as u8;
            surface.set_pixel(x, y, Rgba::new(color.r, color.g, color.b, a));
        }
    }
    Ok(surface)
}

pub fn paint_xy_material(base: &Surface, color_at: impl Fn(f32, f32) -> Rgba) -> Result<Surface> {
    let alpha = base.alpha_mask();
    let bounds = base
        .alpha_bounds()
        .context("text material has no visible glyphs")?;
    let width = (bounds.2 - bounds.0).max(1) as f32;
    let height = (bounds.3 - bounds.1).max(1) as f32;
    let mut surface = Surface::new(base.width(), base.height());
    for y in bounds.1..=bounds.3 {
        let ny = (y - bounds.1) as f32 / height;
        for x in bounds.0..=bounds.2 {
            let coverage = alpha[(y * base.width() + x) as usize];
            if coverage == 0 {
                continue;
            }
            let nx = (x - bounds.0) as f32 / width;
            let color = color_at(nx.clamp(0.0, 1.0), ny.clamp(0.0, 1.0));
            let a = ((coverage as u16 * color.a as u16 + 127) / 255) as u8;
            surface.set_pixel(x, y, Rgba::new(color.r, color.g, color.b, a));
        }
    }
    Ok(surface)
}

pub fn paint_halftone_material(
    base: &Surface,
    foreground: Rgba,
    background: Rgba,
    cell: u32,
) -> Result<Surface> {
    let cell = cell.max(2);
    let alpha = base.alpha_mask();
    let bounds = base
        .alpha_bounds()
        .context("halftone material has no visible glyphs")?;
    let mut surface = Surface::new(base.width(), base.height());
    for y in bounds.1..=bounds.3 {
        for x in bounds.0..=bounds.2 {
            let coverage = alpha[(y * base.width() + x) as usize];
            if coverage == 0 {
                continue;
            }
            let cx = (x % cell) as f32 / cell as f32 - 0.5;
            let cy = (y % cell) as f32 / cell as f32 - 0.5;
            let color = if cx * cx + cy * cy <= 0.12 {
                foreground
            } else {
                background
            };
            let a = ((coverage as u16 * color.a as u16 + 127) / 255) as u8;
            surface.set_pixel(x, y, Rgba::new(color.r, color.g, color.b, a));
        }
    }
    Ok(surface)
}

pub fn gold_color(dark: Rgba, mid: Rgba, light: Rgba, highlight: Rgba, t: f32) -> Rgba {
    // Multiple bands create the high/low reflections expected from polished metal rather
    // than merely coloring glyphs yellow.
    if t < 0.18 {
        lerp_color(dark, light, t / 0.18)
    } else if t < 0.34 {
        lerp_color(light, highlight, (t - 0.18) / 0.16)
    } else if t < 0.52 {
        lerp_color(highlight, mid, (t - 0.34) / 0.18)
    } else if t < 0.76 {
        lerp_color(mid, dark, (t - 0.52) / 0.24)
    } else {
        lerp_color(dark, light, (t - 0.76) / 0.24)
    }
}

pub fn chrome_color(dark: Rgba, mid: Rgba, light: Rgba, t: f32) -> Rgba {
    if t < 0.12 {
        lerp_color(dark, light, t / 0.12)
    } else if t < 0.28 {
        lerp_color(light, mid, (t - 0.12) / 0.16)
    } else if t < 0.48 {
        lerp_color(mid, dark, (t - 0.28) / 0.20)
    } else if t < 0.62 {
        lerp_color(dark, light, (t - 0.48) / 0.14)
    } else if t < 0.78 {
        lerp_color(light, mid, (t - 0.62) / 0.16)
    } else {
        lerp_color(mid, dark, (t - 0.78) / 0.22)
    }
}

pub fn hsv_color(hue: f32, saturation: f32, value: f32, alpha: u8) -> Rgba {
    let hue = hue.rem_euclid(1.0) * 6.0;
    let i = hue.floor() as i32;
    let f = hue - i as f32;
    let p = value * (1.0 - saturation);
    let q = value * (1.0 - saturation * f);
    let t = value * (1.0 - saturation * (1.0 - f));
    let (r, g, b) = match i.rem_euclid(6) {
        0 => (value, t, p),
        1 => (q, value, p),
        2 => (p, value, t),
        3 => (p, q, value),
        4 => (t, p, value),
        _ => (value, p, q),
    };
    Rgba::new(
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8,
        alpha,
    )
}

pub fn lerp_color(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    let channel = |left: u8, right: u8| {
        (left as f32 + (right as f32 - left as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Rgba::new(
        channel(a.r, b.r),
        channel(a.g, b.g),
        channel(a.b, b.b),
        channel(a.a, b.a),
    )
}

pub fn format_color(color: Rgba) -> String {
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        color.r, color.g, color.b, color.a
    )
}

pub fn validate_reveal_animation(
    name: &str,
    period_seconds: f32,
    hold_fraction: f32,
) -> Result<()> {
    if !period_seconds.is_finite() || period_seconds <= 0.0 {
        bail!("{name} period_seconds must be positive");
    }
    if !(0.0..0.95).contains(&hold_fraction) {
        bail!("{name} hold_fraction must be between 0 and 0.95");
    }
    Ok(())
}

pub fn reveal_progress(time_seconds: f64, period_seconds: f32, hold_fraction: f32) -> f32 {
    let phase = (time_seconds / period_seconds as f64).rem_euclid(1.0) as f32;
    let reveal = (1.0 - hold_fraction).max(0.05);
    if phase >= reveal {
        1.0
    } else {
        (phase / reveal).clamp(0.0, 1.0)
    }
}

pub fn scramble_text(
    target: &str,
    time_seconds: f64,
    period_seconds: f32,
    hold_fraction: f32,
    steps_per_second: f32,
    seed: u32,
) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let progress = reveal_progress(time_seconds, period_seconds, hold_fraction);
    if progress >= 0.999 {
        return target.to_string();
    }
    let visible: Vec<char> = target.chars().collect();
    let total = visible.iter().filter(|c| !c.is_whitespace()).count().max(1);
    let tick = (time_seconds * steps_per_second as f64).floor().max(0.0) as u32;
    let mut ordinal = 0usize;
    visible
        .into_iter()
        .map(|ch| {
            if ch.is_whitespace() {
                return ch;
            }
            let threshold = (ordinal + 1) as f32 / total as f32;
            let index = ordinal as u32;
            ordinal += 1;
            if progress >= threshold {
                ch
            } else {
                let mut h = seed ^ index.wrapping_mul(0x9E37_79B9) ^ tick.wrapping_mul(0x85EB_CA6B);
                h ^= h >> 16;
                ALPHABET[(h as usize) % ALPHABET.len()] as char
            }
        })
        .collect()
}

pub fn split_flap_text(
    target: &str,
    time_seconds: f64,
    period_seconds: f32,
    hold_fraction: f32,
    steps_per_second: f32,
) -> String {
    const FLAPS: &[u8] = b" ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let progress = reveal_progress(time_seconds, period_seconds, hold_fraction);
    if progress >= 0.999 {
        return target.to_string();
    }
    let chars: Vec<char> = target.chars().collect();
    let total = chars.iter().filter(|ch| !ch.is_whitespace()).count().max(1);
    let tick = (time_seconds * steps_per_second as f64).floor().max(0.0) as usize;
    let mut ordinal = 0usize;
    chars
        .into_iter()
        .enumerate()
        .map(|(index, ch)| {
            if ch.is_whitespace() {
                return ch;
            }
            let settle_at = 0.35 + 0.60 * (ordinal + 1) as f32 / total as f32;
            ordinal += 1;
            if progress >= settle_at {
                ch
            } else {
                FLAPS[(tick + index * 3) % FLAPS.len()] as char
            }
        })
        .collect()
}
