//! Text paint, mask effects, materials, and frame-varying presentation.
//!
//! Layout and glyph geometry stay in `typography`. This module consumes already-shaped
//! coverage, which lets static material/effect work be cached while animations such as a
//! moving shine reevaluate only presentation state.

use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::{color::Rgba, surface::Surface};

#[derive(Clone, Debug)]
pub struct Style {
    fill: FillStyle,
    underlays: Vec<MaskEffect>,
    overlays: Vec<OverlayEffect>,
    animations: Vec<AnimationEffect>,
}

#[derive(Clone, Copy, Debug)]
pub struct DirectStyleOptions<'a> {
    pub text_color: &'a str,
    pub stroke_color: &'a str,
    pub glow_color: &'a str,
    pub glow_radius: u32,
    pub stroke_width_ratio: f32,
    pub shadow_offset_x_ratio: f32,
    pub shadow_offset_y_ratio: f32,
    pub shadow_blur_radius: u32,
    pub shadow_color: &'a str,
}

#[derive(Clone, Copy, Debug)]
enum FillStyle {
    Flat(Rgba),
    LinearGradient { top: Rgba, bottom: Rgba },
    Gold {
        dark: Rgba,
        mid: Rgba,
        light: Rgba,
        highlight: Rgba,
    },
}

#[derive(Clone, Copy, Debug)]
enum MaskEffect {
    Stroke {
        width_ratio: f32,
        color: Rgba,
    },
    Glow {
        radius: u32,
        color: Rgba,
    },
    Shadow {
        offset_x_ratio: f32,
        offset_y_ratio: f32,
        blur_radius: u32,
        color: Rgba,
    },
    Extrude {
        depth_ratio: f32,
        angle_degrees: f32,
        color: Rgba,
    },
}

#[derive(Clone, Copy, Debug)]
enum OverlayEffect {
    Bevel {
        width_ratio: f32,
        highlight: Rgba,
        shadow: Rgba,
    },
}

#[derive(Clone, Copy, Debug)]
enum AnimationEffect {
    Pulse {
        period_seconds: f32,
        minimum_opacity: f32,
        maximum_opacity: f32,
        phase: f32,
    },
    Shine {
        period_seconds: f32,
        width_ratio: f32,
        angle_degrees: f32,
        color: Rgba,
    },
}

#[derive(Debug, Deserialize)]
struct StyleFile {
    #[serde(default = "default_style_version")]
    version: u32,
    #[serde(default)]
    fill: Option<String>,
    #[serde(default)]
    material: Option<MaterialFile>,
    #[serde(default)]
    effects: Vec<EffectFile>,
    #[serde(default)]
    animations: Vec<AnimationFile>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum MaterialFile {
    LinearGradient {
        top: String,
        bottom: String,
    },
    Gold {
        #[serde(default = "default_gold_dark")]
        dark: String,
        #[serde(default = "default_gold_mid")]
        mid: String,
        #[serde(default = "default_gold_light")]
        light: String,
        #[serde(default = "default_gold_highlight")]
        highlight: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum EffectFile {
    Stroke {
        width: f32,
        color: String,
    },
    Glow {
        radius: u32,
        color: String,
    },
    Shadow {
        #[serde(default = "default_shadow_x")]
        offset_x: f32,
        #[serde(default = "default_shadow_y")]
        offset_y: f32,
        #[serde(default = "default_shadow_blur")]
        blur_radius: u32,
        #[serde(default = "default_shadow_color")]
        color: String,
    },
    Extrude {
        depth: f32,
        #[serde(default = "default_extrude_angle")]
        angle_degrees: f32,
        #[serde(default = "default_extrude_color")]
        color: String,
    },
    Bevel {
        width: f32,
        #[serde(default = "default_bevel_highlight")]
        highlight: String,
        #[serde(default = "default_bevel_shadow")]
        shadow: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum AnimationFile {
    Pulse {
        #[serde(default = "default_pulse_period")]
        period_seconds: f32,
        #[serde(default = "default_pulse_minimum")]
        minimum_opacity: f32,
        #[serde(default = "default_pulse_maximum")]
        maximum_opacity: f32,
        #[serde(default)]
        phase: f32,
    },
    Shine {
        #[serde(default = "default_shine_period")]
        period_seconds: f32,
        #[serde(default = "default_shine_width")]
        width: f32,
        #[serde(default = "default_shine_angle")]
        angle_degrees: f32,
        #[serde(default = "default_shine_color")]
        color: String,
    },
}

fn default_style_version() -> u32 {
    1
}

fn default_shadow_x() -> f32 {
    0.035
}

fn default_shadow_y() -> f32 {
    0.045
}

fn default_shadow_blur() -> u32 {
    6
}

fn default_shadow_color() -> String {
    "#000000A0".to_string()
}

fn default_extrude_angle() -> f32 {
    55.0
}

fn default_extrude_color() -> String {
    "#2A1608D8".to_string()
}

fn default_bevel_highlight() -> String {
    "#FFF1C0B8".to_string()
}

fn default_bevel_shadow() -> String {
    "#321B08B8".to_string()
}

fn default_gold_dark() -> String {
    "#5B3210FF".to_string()
}

fn default_gold_mid() -> String {
    "#C98B3CFF".to_string()
}

fn default_gold_light() -> String {
    "#F3D38AFF".to_string()
}

fn default_gold_highlight() -> String {
    "#FFF1C4FF".to_string()
}

fn default_pulse_period() -> f32 {
    2.4
}

fn default_pulse_minimum() -> f32 {
    0.82
}

fn default_pulse_maximum() -> f32 {
    1.0
}

fn default_shine_period() -> f32 {
    2.8
}

fn default_shine_width() -> f32 {
    0.12
}

fn default_shine_angle() -> f32 {
    18.0
}

fn default_shine_color() -> String {
    "#FFF7D0C8".to_string()
}

impl Style {
    pub fn load(style_file: Option<&Path>, direct: DirectStyleOptions<'_>) -> Result<Self> {
        if let Some(path) = style_file {
            return Self::from_file(path);
        }

        let DirectStyleOptions {
            text_color,
            stroke_color,
            glow_color,
            glow_radius,
            stroke_width_ratio,
            shadow_offset_x_ratio,
            shadow_offset_y_ratio,
            shadow_blur_radius,
            shadow_color,
        } = direct;

        if glow_radius > 64 {
            bail!("--glow-radius must be between 0 and 64");
        }
        if !(0.0..=0.20).contains(&stroke_width_ratio) {
            bail!("--stroke-width must be between 0 and 0.20");
        }
        if !(-0.50..=0.50).contains(&shadow_offset_x_ratio)
            || !(-0.50..=0.50).contains(&shadow_offset_y_ratio)
        {
            bail!("shadow offsets must be between -0.50 and 0.50 of the font size");
        }
        if shadow_blur_radius > 64 {
            bail!("--shadow-blur-radius must be between 0 and 64");
        }

        let mut underlays = Vec::new();
        if shadow_color != "#00000000" {
            let color = Rgba::parse(shadow_color).context("invalid --shadow-color")?;
            if color.a > 0
                && (shadow_blur_radius > 0
                    || shadow_offset_x_ratio != 0.0
                    || shadow_offset_y_ratio != 0.0)
            {
                underlays.push(MaskEffect::Shadow {
                    offset_x_ratio: shadow_offset_x_ratio,
                    offset_y_ratio: shadow_offset_y_ratio,
                    blur_radius: shadow_blur_radius,
                    color,
                });
            }
        }
        if stroke_width_ratio > 0.0 {
            underlays.push(MaskEffect::Stroke {
                width_ratio: stroke_width_ratio,
                color: Rgba::parse(stroke_color).context("invalid --stroke-color")?,
            });
        }
        if glow_radius > 0 {
            let color = Rgba::parse(glow_color).context("invalid --glow-color")?;
            if color.a > 0 {
                underlays.push(MaskEffect::Glow {
                    radius: glow_radius,
                    color,
                });
            }
        }

        Ok(Self {
            fill: FillStyle::Flat(Rgba::parse(text_color).context("invalid --text-color")?),
            underlays,
            overlays: Vec::new(),
            animations: Vec::new(),
        })
    }

    fn from_file(path: &Path) -> Result<Self> {
        let source = fs::read_to_string(path)
            .with_context(|| format!("failed to read text style {}", path.display()))?;
        let parsed: StyleFile = toml::from_str(&source)
            .with_context(|| format!("invalid text style TOML {}", path.display()))?;
        if !matches!(parsed.version, 1 | 2) {
            bail!(
                "unsupported text style version {}; this build supports versions 1 and 2",
                parsed.version
            );
        }
        let uses_v2_features = parsed.material.is_some()
            || !parsed.animations.is_empty()
            || parsed
                .effects
                .iter()
                .any(|effect| matches!(effect, EffectFile::Extrude { .. } | EffectFile::Bevel { .. }));
        if parsed.version == 1 && uses_v2_features {
            bail!("text style uses material, animation, extrusion, or bevel features that require version = 2");
        }
        if parsed.fill.is_some() && parsed.material.is_some() {
            bail!("style must declare either fill or material, not both");
        }
        let fill = match parsed.material {
            None => FillStyle::Flat(
                Rgba::parse(parsed.fill.as_deref().unwrap_or("#EBFFFFFF"))
                    .context("invalid style fill color")?,
            ),
            Some(MaterialFile::LinearGradient { top, bottom }) => FillStyle::LinearGradient {
                top: Rgba::parse(&top).context("invalid gradient top color")?,
                bottom: Rgba::parse(&bottom).context("invalid gradient bottom color")?,
            },
            Some(MaterialFile::Gold {
                dark,
                mid,
                light,
                highlight,
            }) => FillStyle::Gold {
                dark: Rgba::parse(&dark).context("invalid gold dark color")?,
                mid: Rgba::parse(&mid).context("invalid gold mid color")?,
                light: Rgba::parse(&light).context("invalid gold light color")?,
                highlight: Rgba::parse(&highlight).context("invalid gold highlight color")?,
            },
        };

        let mut underlays = Vec::new();
        let mut overlays = Vec::new();
        for effect in parsed.effects {
            match effect {
                EffectFile::Stroke { width, color } => {
                    if !(0.0..=0.20).contains(&width) {
                        bail!("style stroke width must be between 0 and 0.20");
                    }
                    underlays.push(MaskEffect::Stroke {
                        width_ratio: width,
                        color: Rgba::parse(&color).context("invalid style stroke color")?,
                    });
                }
                EffectFile::Glow { radius, color } => {
                    if radius > 64 {
                        bail!("style glow radius must be between 0 and 64");
                    }
                    underlays.push(MaskEffect::Glow {
                        radius,
                        color: Rgba::parse(&color).context("invalid style glow color")?,
                    });
                }
                EffectFile::Shadow {
                    offset_x,
                    offset_y,
                    blur_radius,
                    color,
                } => {
                    if !(-0.50..=0.50).contains(&offset_x) || !(-0.50..=0.50).contains(&offset_y) {
                        bail!("style shadow offsets must be between -0.50 and 0.50");
                    }
                    if blur_radius > 64 {
                        bail!("style shadow blur radius must be between 0 and 64");
                    }
                    underlays.push(MaskEffect::Shadow {
                        offset_x_ratio: offset_x,
                        offset_y_ratio: offset_y,
                        blur_radius,
                        color: Rgba::parse(&color).context("invalid style shadow color")?,
                    });
                }
                EffectFile::Extrude {
                    depth,
                    angle_degrees,
                    color,
                } => {
                    if !(0.0..=0.35).contains(&depth) {
                        bail!("style extrude depth must be between 0 and 0.35");
                    }
                    if !angle_degrees.is_finite() {
                        bail!("style extrude angle must be finite");
                    }
                    underlays.push(MaskEffect::Extrude {
                        depth_ratio: depth,
                        angle_degrees,
                        color: Rgba::parse(&color).context("invalid style extrude color")?,
                    });
                }
                EffectFile::Bevel {
                    width,
                    highlight,
                    shadow,
                } => {
                    if !(0.0..=0.20).contains(&width) {
                        bail!("style bevel width must be between 0 and 0.20");
                    }
                    overlays.push(OverlayEffect::Bevel {
                        width_ratio: width,
                        highlight: Rgba::parse(&highlight)
                            .context("invalid style bevel highlight color")?,
                        shadow: Rgba::parse(&shadow)
                            .context("invalid style bevel shadow color")?,
                    });
                }
            }
        }

        let mut animations = Vec::new();
        for animation in parsed.animations {
            animations.push(match animation {
                AnimationFile::Pulse {
                    period_seconds,
                    minimum_opacity,
                    maximum_opacity,
                    phase,
                } => {
                    if !period_seconds.is_finite() || period_seconds <= 0.0 {
                        bail!("pulse period_seconds must be positive");
                    }
                    if !(0.0..=1.0).contains(&minimum_opacity)
                        || !(0.0..=1.0).contains(&maximum_opacity)
                        || minimum_opacity > maximum_opacity
                    {
                        bail!("pulse opacity range must satisfy 0 <= minimum <= maximum <= 1");
                    }
                    AnimationEffect::Pulse {
                        period_seconds,
                        minimum_opacity,
                        maximum_opacity,
                        phase,
                    }
                }
                AnimationFile::Shine {
                    period_seconds,
                    width,
                    angle_degrees,
                    color,
                } => {
                    if !period_seconds.is_finite() || period_seconds <= 0.0 {
                        bail!("shine period_seconds must be positive");
                    }
                    if !(0.01..=0.75).contains(&width) {
                        bail!("shine width must be between 0.01 and 0.75");
                    }
                    if !angle_degrees.is_finite() {
                        bail!("shine angle must be finite");
                    }
                    AnimationEffect::Shine {
                        period_seconds,
                        width_ratio: width,
                        angle_degrees,
                        color: Rgba::parse(&color).context("invalid shine color")?,
                    }
                }
            });
        }

        Ok(Self {
            fill,
            underlays,
            overlays,
            animations,
        })
    }

    pub fn describe(&self) -> String {
        let mut parts = vec![match self.fill {
            FillStyle::Flat(color) => format!("fill={}", format_color(color)),
            FillStyle::LinearGradient { top, bottom } => format!(
                "linear-gradient(top={},bottom={})",
                format_color(top),
                format_color(bottom)
            ),
            FillStyle::Gold {
                dark,
                mid,
                light,
                highlight,
            } => format!(
                "gold(dark={},mid={},light={},highlight={})",
                format_color(dark),
                format_color(mid),
                format_color(light),
                format_color(highlight)
            ),
        }];
        for effect in &self.underlays {
            parts.push(match *effect {
                MaskEffect::Stroke { width_ratio, color } => format!(
                    "stroke(width={width_ratio:.5},color={})",
                    format_color(color)
                ),
                MaskEffect::Glow { radius, color } => {
                    format!("glow(radius={radius},color={})", format_color(color))
                }
                MaskEffect::Shadow {
                    offset_x_ratio,
                    offset_y_ratio,
                    blur_radius,
                    color,
                } => format!(
                    "shadow(x={offset_x_ratio:.5},y={offset_y_ratio:.5},blur={blur_radius},color={})",
                    format_color(color)
                ),
                MaskEffect::Extrude {
                    depth_ratio,
                    angle_degrees,
                    color,
                } => format!(
                    "extrude(depth={depth_ratio:.5},angle={angle_degrees:.2},color={})",
                    format_color(color)
                ),
            });
        }
        for effect in &self.overlays {
            match *effect {
                OverlayEffect::Bevel {
                    width_ratio,
                    highlight,
                    shadow,
                } => parts.push(format!(
                    "bevel(width={width_ratio:.5},highlight={},shadow={})",
                    format_color(highlight),
                    format_color(shadow)
                )),
            }
        }
        for animation in &self.animations {
            parts.push(match *animation {
                AnimationEffect::Pulse {
                    period_seconds,
                    minimum_opacity,
                    maximum_opacity,
                    phase,
                } => format!(
                    "pulse(period={period_seconds:.3},min={minimum_opacity:.3},max={maximum_opacity:.3},phase={phase:.3})"
                ),
                AnimationEffect::Shine {
                    period_seconds,
                    width_ratio,
                    angle_degrees,
                    color,
                } => format!(
                    "shine(period={period_seconds:.3},width={width_ratio:.3},angle={angle_degrees:.2},color={})",
                    format_color(color)
                ),
            });
        }
        parts.join(";")
    }

    /// Paint the static portion of a shaped text surface. Layout and font discovery are
    /// not repeated for animated effects.
    pub fn compose(&self, base: &Surface, font_size: f32, supersampling: u32) -> Result<Surface> {
        let width = base.width();
        let height = base.height();
        let alpha = base.alpha_mask();
        let mut combined = Surface::new(width, height);

        for effect in &self.underlays {
            match *effect {
                MaskEffect::Stroke { width_ratio, color } => {
                    let radius = (font_size * width_ratio).round().max(0.0) as usize;
                    if radius == 0 || color.a == 0 {
                        continue;
                    }
                    let stroke_alpha =
                        dilate_alpha_circular(&alpha, width as usize, height as usize, radius);
                    let stroke = Surface::from_alpha_mask(width, height, &stroke_alpha, color)?;
                    combined.blend_surface(&stroke, 0, 0, 1.0);
                }
                MaskEffect::Glow { radius, color } => {
                    if radius == 0 || color.a == 0 {
                        continue;
                    }
                    let glow = Surface::from_alpha_mask(width, height, &alpha, color)?
                        .box_blur((radius * supersampling).max(2), 2);
                    combined.blend_surface(&glow, 0, 0, 1.0);
                }
                MaskEffect::Shadow {
                    offset_x_ratio,
                    offset_y_ratio,
                    blur_radius,
                    color,
                } => {
                    if color.a == 0 {
                        continue;
                    }
                    let mut shadow = Surface::from_alpha_mask(width, height, &alpha, color)?;
                    if blur_radius > 0 {
                        shadow = shadow.box_blur((blur_radius * supersampling).max(1), 2);
                    }
                    let dx = (font_size * offset_x_ratio).round() as i32;
                    let dy = (font_size * offset_y_ratio).round() as i32;
                    combined.blend_surface(&shadow, dx, dy, 1.0);
                }
                MaskEffect::Extrude {
                    depth_ratio,
                    angle_degrees,
                    color,
                } => {
                    if depth_ratio <= 0.0 || color.a == 0 {
                        continue;
                    }
                    let depth = (font_size * depth_ratio).round().max(1.0) as i32;
                    let angle = angle_degrees.to_radians();
                    let dx = angle.cos();
                    let dy = angle.sin();
                    let layer = Surface::from_alpha_mask(width, height, &alpha, color)?;
                    for step in (1..=depth).rev() {
                        combined.blend_surface(
                            &layer,
                            (dx * step as f32).round() as i32,
                            (dy * step as f32).round() as i32,
                            1.0,
                        );
                    }
                }
            }
        }

        let fill = self.paint_fill(base)?;
        combined.blend_surface(&fill, 0, 0, 1.0);

        for effect in &self.overlays {
            match *effect {
                OverlayEffect::Bevel {
                    width_ratio,
                    highlight,
                    shadow,
                } => {
                    let radius = (font_size * width_ratio).round().max(1.0) as i32;
                    let (highlight_mask, shadow_mask) = directional_bevel_masks(
                        &alpha,
                        width as usize,
                        height as usize,
                        radius,
                    );
                    let highlight_layer =
                        Surface::from_alpha_mask(width, height, &highlight_mask, highlight)?;
                    let shadow_layer =
                        Surface::from_alpha_mask(width, height, &shadow_mask, shadow)?;
                    combined.blend_surface(&highlight_layer, 0, 0, 1.0);
                    combined.blend_surface(&shadow_layer, 0, 0, 1.0);
                }
            }
        }
        Ok(combined)
    }

    /// Coverage that must remain fully inside the writable region while fitting.
    ///
    /// Hard geometry (glyph fill, stroke, extrusion) participates in fit safety. Soft
    /// atmospheric effects (glow and blurred shadow) may be clipped by the writable
    /// mask; treating their blur tails as hard geometry made otherwise good titles
    /// substantially smaller.
    pub fn fit_envelope(&self, base: &Surface, font_size: f32) -> Result<Surface> {
        let width = base.width();
        let height = base.height();
        let alpha = base.alpha_mask();
        let opaque = Rgba::new(255, 255, 255, 255);
        let mut envelope = Surface::new(width, height);

        for effect in &self.underlays {
            match *effect {
                MaskEffect::Stroke { width_ratio, .. } => {
                    let radius = (font_size * width_ratio).round().max(0.0) as usize;
                    if radius > 0 {
                        let expanded =
                            dilate_alpha_circular(&alpha, width as usize, height as usize, radius);
                        let layer = Surface::from_alpha_mask(width, height, &expanded, opaque)?;
                        envelope.blend_surface(&layer, 0, 0, 1.0);
                    }
                }
                MaskEffect::Extrude {
                    depth_ratio,
                    angle_degrees,
                    ..
                } => {
                    if depth_ratio <= 0.0 {
                        continue;
                    }
                    let depth = (font_size * depth_ratio).round().max(1.0) as i32;
                    let angle = angle_degrees.to_radians();
                    let dx = angle.cos();
                    let dy = angle.sin();
                    let layer = Surface::from_alpha_mask(width, height, &alpha, opaque)?;
                    for step in 1..=depth {
                        envelope.blend_surface(
                            &layer,
                            (dx * step as f32).round() as i32,
                            (dy * step as f32).round() as i32,
                            1.0,
                        );
                    }
                }
                MaskEffect::Glow { .. } | MaskEffect::Shadow { .. } => {}
            }
        }

        let fill = Surface::from_alpha_mask(width, height, &alpha, opaque)?;
        envelope.blend_surface(&fill, 0, 0, 1.0);
        Ok(envelope)
    }

    /// Opacity animation that can be applied at the final plaque-compositing boundary.
    pub fn frame_opacity(&self, time_seconds: f64) -> f32 {
        self.animations.iter().fold(1.0_f32, |opacity, animation| {
            let value = match *animation {
                AnimationEffect::Pulse {
                    period_seconds,
                    minimum_opacity,
                    maximum_opacity,
                    phase,
                } => {
                    let angle = std::f64::consts::TAU
                        * (time_seconds / period_seconds as f64 + phase as f64);
                    let wave = ((angle.sin() + 1.0) * 0.5) as f32;
                    minimum_opacity + (maximum_opacity - minimum_opacity) * wave
                }
                AnimationEffect::Shine { .. } => 1.0,
            };
            opacity * value
        })
    }

    /// Build only the frame-varying overlay. `glyph_mask` is the shaped fill coverage,
    /// excluding static glow/shadow, so shine never leaks into the halo.
    pub fn frame_overlay(
        &self,
        glyph_mask: &[u8],
        width: u32,
        height: u32,
        time_seconds: f64,
    ) -> Result<Option<Surface>> {
        if glyph_mask.len() != width as usize * height as usize {
            bail!("animated glyph mask dimensions do not match the title layer");
        }
        let mut output: Option<Surface> = None;
        for animation in &self.animations {
            let AnimationEffect::Shine {
                period_seconds,
                width_ratio,
                angle_degrees,
                color,
            } = *animation
            else {
                continue;
            };
            if color.a == 0 {
                continue;
            }
            let mut shine = Surface::new(width, height);
            let angle = angle_degrees.to_radians();
            let axis_x = angle.cos();
            let axis_y = angle.sin();
            let projections = [
                0.0_f32,
                (width.saturating_sub(1) as f32) * axis_x,
                (height.saturating_sub(1) as f32) * axis_y,
                (width.saturating_sub(1) as f32) * axis_x
                    + (height.saturating_sub(1) as f32) * axis_y,
            ];
            let min_projection = projections.iter().copied().fold(f32::INFINITY, f32::min);
            let max_projection = projections
                .iter()
                .copied()
                .fold(f32::NEG_INFINITY, f32::max);
            let span = (max_projection - min_projection).max(1.0);
            let stripe_width = (span * width_ratio).max(1.0);
            let progress = (time_seconds / period_seconds as f64).rem_euclid(1.0) as f32;
            let center = min_projection - stripe_width
                + progress * (span + stripe_width * 2.0);
            for y in 0..height {
                for x in 0..width {
                    let alpha = glyph_mask[(y * width + x) as usize];
                    if alpha == 0 {
                        continue;
                    }
                    let projected = x as f32 * axis_x + y as f32 * axis_y;
                    let distance = (projected - center).abs();
                    if distance > stripe_width {
                        continue;
                    }
                    let envelope = (1.0 - distance / stripe_width).powi(2);
                    let animated_alpha =
                        (color.a as f32 * envelope * alpha as f32 / 255.0).round() as u8;
                    shine.set_pixel(
                        x,
                        y,
                        Rgba::new(color.r, color.g, color.b, animated_alpha),
                    );
                }
            }
            match &mut output {
                Some(existing) => existing.blend_surface(&shine, 0, 0, 1.0),
                None => output = Some(shine),
            }
        }
        Ok(output)
    }

    fn paint_fill(&self, base: &Surface) -> Result<Surface> {
        match self.fill {
            FillStyle::Flat(color) => {
                let mut fill = base.clone();
                fill.recolor(color);
                Ok(fill)
            }
            FillStyle::LinearGradient { top, bottom } => {
                paint_vertical_material(base, |t| lerp_color(top, bottom, t))
            }
            FillStyle::Gold {
                dark,
                mid,
                light,
                highlight,
            } => paint_vertical_material(base, |t| gold_color(dark, mid, light, highlight, t)),
        }
    }
}

fn paint_vertical_material(
    base: &Surface,
    color_at: impl Fn(f32) -> Rgba,
) -> Result<Surface> {
    let alpha = base.alpha_mask();
    let bounds = base.alpha_bounds().context("text material has no visible glyphs")?;
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

fn gold_color(dark: Rgba, mid: Rgba, light: Rgba, highlight: Rgba, t: f32) -> Rgba {
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

fn lerp_color(a: Rgba, b: Rgba, t: f32) -> Rgba {
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

fn format_color(color: Rgba) -> String {
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        color.r, color.g, color.b, color.a
    )
}

fn directional_bevel_masks(
    source: &[u8],
    width: usize,
    height: usize,
    radius: i32,
) -> (Vec<u8>, Vec<u8>) {
    let mut highlight = vec![0; source.len()];
    let mut shadow = vec![0; source.len()];
    for y in 0..height {
        for x in 0..width {
            let current = source[y * width + x];
            if current == 0 {
                continue;
            }
            let top_left = sample_alpha(source, width, height, x as i32 - radius, y as i32 - radius);
            let bottom_right =
                sample_alpha(source, width, height, x as i32 + radius, y as i32 + radius);
            highlight[y * width + x] = current.saturating_sub(top_left);
            shadow[y * width + x] = current.saturating_sub(bottom_right);
        }
    }
    (highlight, shadow)
}

fn sample_alpha(source: &[u8], width: usize, height: usize, x: i32, y: i32) -> u8 {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        0
    } else {
        source[y as usize * width + x as usize]
    }
}

fn dilate_alpha_circular(source: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    if radius == 0 {
        return source.to_vec();
    }
    let radius_squared = (radius * radius) as isize;
    let offsets: Vec<(isize, isize)> = (-(radius as isize)..=radius as isize)
        .flat_map(|dy| {
            (-(radius as isize)..=radius as isize)
                .filter_map(move |dx| (dx * dx + dy * dy <= radius_squared).then_some((dx, dy)))
        })
        .collect();
    let mut output = vec![0u8; source.len()];
    for y in 0..height {
        for x in 0..width {
            let mut value = 0u8;
            for &(dx, dy) in &offsets {
                let xx = x as isize + dx;
                let yy = y as isize + dy;
                if xx < 0 || yy < 0 || xx >= width as isize || yy >= height as isize {
                    continue;
                }
                value = value.max(source[yy as usize * width + xx as usize]);
                if value == u8::MAX {
                    break;
                }
            }
            output[y * width + x] = value;
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{dilate_alpha_circular, directional_bevel_masks};

    #[test]
    fn circular_dilation_does_not_fill_square_corners() {
        let mut source = vec![0; 25];
        source[12] = 255;
        let dilated = dilate_alpha_circular(&source, 5, 5, 2);
        assert_eq!(dilated[12], 255);
        assert_eq!(dilated[2], 255);
        assert_eq!(dilated[0], 0);
    }

    #[test]
    fn bevel_separates_opposite_edges() {
        let source = vec![255; 25];
        let (highlight, shadow) = directional_bevel_masks(&source, 5, 5, 1);
        assert!(highlight[0] > 0);
        assert!(shadow[24] > 0);
    }
}
