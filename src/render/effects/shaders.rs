//! Material shaders, color ramps, procedurals, and text animations.

use crate::{color::Rgba, surface::Surface};
use anyhow::{Context, Result, bail};

/// Alpha produced when painting a color over partial glyph coverage.
fn coverage_alpha(color: Rgba, coverage: u8) -> u8 {
    ((coverage as u16 * color.a as u16 + 127) / 255) as u8
}

/// Paint every coverage-bearing glyph pixel with a spatially evaluated color.
///
/// The closure receives raw pixel coordinates plus the glyph-boundary-normalized
/// coordinates (each clamped to `0.0..=1.0`, bottom/right positive).
fn paint_glyph_material(
    base: &Surface,
    what: &str,
    mut color_at: impl FnMut(u32, u32, f32, f32) -> Rgba,
) -> Result<Surface> {
    let alpha = base.alpha_mask();
    let bounds = base
        .alpha_bounds()
        .with_context(|| format!("{what} has no visible glyphs"))?;
    let span_x = (bounds.2 - bounds.0).max(1) as f32;
    let span_y = (bounds.3 - bounds.1).max(1) as f32;
    let mut surface = Surface::new(base.width(), base.height());
    for y in bounds.1..=bounds.3 {
        let ny = ((y - bounds.1) as f32 / span_y).clamp(0.0, 1.0);
        for x in bounds.0..=bounds.2 {
            let coverage = alpha[(y * base.width() + x) as usize];
            if coverage == 0 {
                continue;
            }
            let nx = ((x - bounds.0) as f32 / span_x).clamp(0.0, 1.0);
            let color = color_at(x, y, nx, ny);
            surface.set_pixel(
                x,
                y,
                Rgba::new(color.r, color.g, color.b, coverage_alpha(color, coverage)),
            );
        }
    }
    Ok(surface)
}

pub fn paint_vertical_material(base: &Surface, color_at: impl Fn(f32) -> Rgba) -> Result<Surface> {
    paint_glyph_material(base, "text material", |_x, _y, _nx, ny| color_at(ny))
}

pub fn paint_xy_material(base: &Surface, color_at: impl Fn(f32, f32) -> Rgba) -> Result<Surface> {
    paint_glyph_material(base, "text material", |_x, _y, nx, ny| color_at(nx, ny))
}

pub fn paint_halftone_material(
    base: &Surface,
    foreground: Rgba,
    background: Rgba,
    cell: u32,
) -> Result<Surface> {
    let cell = cell.max(2);
    paint_glyph_material(base, "halftone material", |x, y, _nx, _ny| {
        let cx = (x % cell) as f32 / cell as f32 - 0.5;
        let cy = (y % cell) as f32 / cell as f32 - 0.5;
        if cx * cx + cy * cy <= 0.12 {
            foreground
        } else {
            background
        }
    })
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

/// Quantized presentation state shared by progressive text-reveal animations.
#[derive(Clone, Copy)]
struct RevealTick {
    progress: f32,
    tick: usize,
}

fn reveal_tick(
    time_seconds: f64,
    period_seconds: f32,
    hold_fraction: f32,
    steps_per_second: f32,
) -> RevealTick {
    RevealTick {
        progress: reveal_progress(time_seconds, period_seconds, hold_fraction),
        tick: (time_seconds * steps_per_second as f64).floor().max(0.0) as usize,
    }
}

/// Walk the target text once, preserving whitespace, and let `substitute`
/// replace not-yet-revealed characters (`None` keeps the original).
fn reveal_animated_text(
    target: &str,
    state: RevealTick,
    substitute: impl Fn(usize, usize, f32) -> Option<char>,
) -> String {
    if state.progress >= 0.999 {
        return target.to_string();
    }
    let chars: Vec<char> = target.chars().collect();
    let total = chars.iter().filter(|ch| !ch.is_whitespace()).count().max(1);
    let mut ordinal = 0usize;
    chars
        .into_iter()
        .enumerate()
        .map(|(index, ch)| {
            if ch.is_whitespace() {
                return ch;
            }
            let fraction = (ordinal + 1) as f32 / total as f32;
            let current = ordinal;
            ordinal += 1;
            substitute(current, index, fraction).unwrap_or(ch)
        })
        .collect()
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
    let state = reveal_tick(
        time_seconds,
        period_seconds,
        hold_fraction,
        steps_per_second,
    );
    reveal_animated_text(target, state, |ordinal, _index, fraction| {
        if state.progress >= fraction {
            return None;
        }
        let index = ordinal as u32;
        let mut h =
            seed ^ index.wrapping_mul(0x9E37_79B9) ^ (state.tick as u32).wrapping_mul(0x85EB_CA6B);
        h ^= h >> 16;
        Some(ALPHABET[(h as usize) % ALPHABET.len()] as char)
    })
}

pub fn split_flap_text(
    target: &str,
    time_seconds: f64,
    period_seconds: f32,
    hold_fraction: f32,
    steps_per_second: f32,
) -> String {
    const FLAPS: &[u8] = b" ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let state = reveal_tick(
        time_seconds,
        period_seconds,
        hold_fraction,
        steps_per_second,
    );
    reveal_animated_text(target, state, |_ordinal, index, fraction| {
        let settle_at = 0.35 + 0.60 * fraction;
        if state.progress >= settle_at {
            return None;
        }
        Some(FLAPS[(state.tick + index * 3) % FLAPS.len()] as char)
    })
}
