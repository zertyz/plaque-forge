//! Text paint and mask effects.
//!
//! This module deliberately owns effects that operate on an already-shaped text mask.
//! Glyph/layout transforms belong in the typography layout stage, while effects that
//! physically alter the plaque surface (engraving, displacement, true protrusion) belong
//! in a later scene/material stage. Keeping those boundaries explicit prevents the text
//! renderer from becoming one large effect switch.

use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::{color::Rgba, surface::Surface};

#[derive(Clone, Debug)]
pub struct Style {
    fill: Rgba,
    mask_effects: Vec<MaskEffect>,
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

#[derive(Clone, Debug)]
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
}

#[derive(Debug, Deserialize)]
struct StyleFile {
    #[serde(default = "default_style_version")]
    version: u32,
    #[serde(default)]
    fill: Option<String>,
    #[serde(default)]
    effects: Vec<EffectFile>,
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

        let mut effects = Vec::new();
        if shadow_color != "#00000000" {
            let color = Rgba::parse(shadow_color).context("invalid --shadow-color")?;
            if color.a > 0
                && (shadow_blur_radius > 0
                    || shadow_offset_x_ratio != 0.0
                    || shadow_offset_y_ratio != 0.0)
            {
                effects.push(MaskEffect::Shadow {
                    offset_x_ratio: shadow_offset_x_ratio,
                    offset_y_ratio: shadow_offset_y_ratio,
                    blur_radius: shadow_blur_radius,
                    color,
                });
            }
        }
        if stroke_width_ratio > 0.0 {
            effects.push(MaskEffect::Stroke {
                width_ratio: stroke_width_ratio,
                color: Rgba::parse(stroke_color).context("invalid --stroke-color")?,
            });
        }
        if glow_radius > 0 {
            let color = Rgba::parse(glow_color).context("invalid --glow-color")?;
            if color.a > 0 {
                effects.push(MaskEffect::Glow {
                    radius: glow_radius,
                    color,
                });
            }
        }

        Ok(Self {
            fill: Rgba::parse(text_color).context("invalid --text-color")?,
            mask_effects: effects,
        })
    }

    fn from_file(path: &Path) -> Result<Self> {
        let source = fs::read_to_string(path)
            .with_context(|| format!("failed to read text style {}", path.display()))?;
        let parsed: StyleFile = toml::from_str(&source)
            .with_context(|| format!("invalid text style TOML {}", path.display()))?;
        if parsed.version != 1 {
            bail!(
                "unsupported text style version {}; this build supports version 1",
                parsed.version
            );
        }
        let fill = Rgba::parse(parsed.fill.as_deref().unwrap_or("#EBFFFFFF"))
            .context("invalid style fill color")?;
        let mut effects = Vec::with_capacity(parsed.effects.len());
        for effect in parsed.effects {
            effects.push(match effect {
                EffectFile::Stroke { width, color } => {
                    if !(0.0..=0.20).contains(&width) {
                        bail!("style stroke width must be between 0 and 0.20");
                    }
                    MaskEffect::Stroke {
                        width_ratio: width,
                        color: Rgba::parse(&color).context("invalid style stroke color")?,
                    }
                }
                EffectFile::Glow { radius, color } => {
                    if radius > 64 {
                        bail!("style glow radius must be between 0 and 64");
                    }
                    MaskEffect::Glow {
                        radius,
                        color: Rgba::parse(&color).context("invalid style glow color")?,
                    }
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
                    MaskEffect::Shadow {
                        offset_x_ratio: offset_x,
                        offset_y_ratio: offset_y,
                        blur_radius,
                        color: Rgba::parse(&color).context("invalid style shadow color")?,
                    }
                }
            });
        }
        Ok(Self {
            fill,
            mask_effects: effects,
        })
    }

    pub fn describe(&self) -> String {
        let mut parts = vec![format!(
            "fill=#{:02X}{:02X}{:02X}{:02X}",
            self.fill.r, self.fill.g, self.fill.b, self.fill.a
        )];
        for effect in &self.mask_effects {
            parts.push(match *effect {
                MaskEffect::Stroke { width_ratio, color } => format!(
                    "stroke(width={width_ratio:.5},color=#{:02X}{:02X}{:02X}{:02X})",
                    color.r, color.g, color.b, color.a
                ),
                MaskEffect::Glow { radius, color } => format!(
                    "glow(radius={radius},color=#{:02X}{:02X}{:02X}{:02X})",
                    color.r, color.g, color.b, color.a
                ),
                MaskEffect::Shadow {
                    offset_x_ratio,
                    offset_y_ratio,
                    blur_radius,
                    color,
                } => format!(
                    "shadow(x={offset_x_ratio:.5},y={offset_y_ratio:.5},blur={blur_radius},color=#{:02X}{:02X}{:02X}{:02X})",
                    color.r, color.g, color.b, color.a
                ),
            });
        }
        parts.join(";")
    }

    /// Paint a shaped text surface. Effects are composited back-to-front in declaration
    /// order and the text fill is always painted last.
    pub fn compose(&self, base: &Surface, font_size: f32, supersampling: u32) -> Result<Surface> {
        let width = base.width();
        let height = base.height();
        let alpha = base.alpha_mask();
        let mut combined = Surface::new(width, height);

        for effect in &self.mask_effects {
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
            }
        }

        let mut fill = base.clone();
        fill.recolor(self.fill);
        combined.blend_surface(&fill, 0, 0, 1.0);
        Ok(combined)
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
    use super::dilate_alpha_circular;

    #[test]
    fn circular_dilation_does_not_fill_square_corners() {
        let mut source = vec![0; 25];
        source[12] = 255;
        let dilated = dilate_alpha_circular(&source, 5, 5, 2);
        assert_eq!(dilated[12], 255);
        assert_eq!(dilated[2], 255);
        assert_eq!(dilated[0], 0);
    }
}
