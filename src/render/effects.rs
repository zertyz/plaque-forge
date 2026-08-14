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
    LinearGradient {
        top: Rgba,
        bottom: Rgba,
    },
    Gold {
        dark: Rgba,
        mid: Rgba,
        light: Rgba,
        highlight: Rgba,
    },
    Chrome {
        dark: Rgba,
        mid: Rgba,
        light: Rgba,
    },
    Holographic,
    Fire {
        dark: Rgba,
        mid: Rgba,
        light: Rgba,
    },
    Ice {
        dark: Rgba,
        mid: Rgba,
        light: Rgba,
    },
    Nebula {
        dark: Rgba,
        mid: Rgba,
        light: Rgba,
    },
    Liquid {
        first: Rgba,
        second: Rgba,
        frequency: f32,
    },
    Halftone {
        foreground: Rgba,
        background: Rgba,
        cell: u32,
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
    ChromaticSplit {
        offset_ratio: f32,
        red: Rgba,
        cyan: Rgba,
    },
    Trail {
        distance_ratio: f32,
        copies: u32,
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
    Letterpress {
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
    Flicker {
        period_seconds: f32,
        minimum_opacity: f32,
        strength: f32,
        phase: f32,
    },
    Wave {
        period_seconds: f32,
        amplitude_ratio: f32,
        wavelength_ratio: f32,
        phase: f32,
    },
    Typewriter {
        period_seconds: f32,
        hold_fraction: f32,
    },
    Dissolve {
        period_seconds: f32,
        hold_fraction: f32,
        seed: u32,
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
    Chrome {
        #[serde(default = "default_chrome_dark")]
        dark: String,
        #[serde(default = "default_chrome_mid")]
        mid: String,
        #[serde(default = "default_chrome_light")]
        light: String,
    },
    Holographic,
    Fire {
        #[serde(default = "default_fire_dark")]
        dark: String,
        #[serde(default = "default_fire_mid")]
        mid: String,
        #[serde(default = "default_fire_light")]
        light: String,
    },
    Ice {
        #[serde(default = "default_ice_dark")]
        dark: String,
        #[serde(default = "default_ice_mid")]
        mid: String,
        #[serde(default = "default_ice_light")]
        light: String,
    },
    Nebula {
        #[serde(default = "default_nebula_dark")]
        dark: String,
        #[serde(default = "default_nebula_mid")]
        mid: String,
        #[serde(default = "default_nebula_light")]
        light: String,
    },
    Liquid {
        #[serde(default = "default_liquid_first")]
        first: String,
        #[serde(default = "default_liquid_second")]
        second: String,
        #[serde(default = "default_liquid_frequency")]
        frequency: f32,
    },
    Halftone {
        #[serde(default = "default_halftone_foreground")]
        foreground: String,
        #[serde(default = "default_halftone_background")]
        background: String,
        #[serde(default = "default_halftone_cell")]
        cell: u32,
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
    Letterpress {
        width: f32,
        #[serde(default = "default_letterpress_highlight")]
        highlight: String,
        #[serde(default = "default_letterpress_shadow")]
        shadow: String,
    },
    ChromaticSplit {
        #[serde(default = "default_chromatic_offset")]
        offset: f32,
        #[serde(default = "default_chromatic_red")]
        red: String,
        #[serde(default = "default_chromatic_cyan")]
        cyan: String,
    },
    Trail {
        #[serde(default = "default_trail_distance")]
        distance: f32,
        #[serde(default = "default_trail_copies")]
        copies: u32,
        #[serde(default = "default_trail_angle")]
        angle_degrees: f32,
        #[serde(default = "default_trail_color")]
        color: String,
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
    Flicker {
        #[serde(default = "default_flicker_period")]
        period_seconds: f32,
        #[serde(default = "default_flicker_minimum")]
        minimum_opacity: f32,
        #[serde(default = "default_flicker_strength")]
        strength: f32,
        #[serde(default)]
        phase: f32,
    },
    Wave {
        #[serde(default = "default_wave_period")]
        period_seconds: f32,
        #[serde(default = "default_wave_amplitude")]
        amplitude: f32,
        #[serde(default = "default_wave_wavelength")]
        wavelength: f32,
        #[serde(default)]
        phase: f32,
    },
    Typewriter {
        #[serde(default = "default_typewriter_period")]
        period_seconds: f32,
        #[serde(default = "default_reveal_hold")]
        hold_fraction: f32,
    },
    Dissolve {
        #[serde(default = "default_dissolve_period")]
        period_seconds: f32,
        #[serde(default = "default_reveal_hold")]
        hold_fraction: f32,
        #[serde(default = "default_dissolve_seed")]
        seed: u32,
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

fn default_chrome_dark() -> String {
    "#18202AFF".to_string()
}
fn default_chrome_mid() -> String {
    "#82909FFF".to_string()
}
fn default_chrome_light() -> String {
    "#F7FBFFFF".to_string()
}
fn default_fire_dark() -> String {
    "#681000FF".to_string()
}
fn default_fire_mid() -> String {
    "#E84908FF".to_string()
}
fn default_fire_light() -> String {
    "#FFE59AFF".to_string()
}
fn default_ice_dark() -> String {
    "#174764FF".to_string()
}
fn default_ice_mid() -> String {
    "#77D9F4FF".to_string()
}
fn default_ice_light() -> String {
    "#F3FFFFFF".to_string()
}
fn default_nebula_dark() -> String {
    "#32134FFF".to_string()
}
fn default_nebula_mid() -> String {
    "#B346D2FF".to_string()
}
fn default_nebula_light() -> String {
    "#57D8F3FF".to_string()
}
fn default_liquid_first() -> String {
    "#26E6D5FF".to_string()
}
fn default_liquid_second() -> String {
    "#3D5AF1FF".to_string()
}
fn default_liquid_frequency() -> f32 {
    3.0
}
fn default_halftone_foreground() -> String {
    "#FFF4D0FF".to_string()
}
fn default_halftone_background() -> String {
    "#FF4A78FF".to_string()
}
fn default_halftone_cell() -> u32 {
    6
}
fn default_letterpress_highlight() -> String {
    "#FFF0C078".to_string()
}
fn default_letterpress_shadow() -> String {
    "#120A08C8".to_string()
}
fn default_chromatic_offset() -> f32 {
    0.035
}
fn default_chromatic_red() -> String {
    "#FF244CB8".to_string()
}
fn default_chromatic_cyan() -> String {
    "#26F5FFB8".to_string()
}
fn default_trail_distance() -> f32 {
    0.18
}
fn default_trail_copies() -> u32 {
    7
}
fn default_trail_angle() -> f32 {
    180.0
}
fn default_trail_color() -> String {
    "#4AD8FF58".to_string()
}
fn default_flicker_period() -> f32 {
    1.8
}
fn default_flicker_minimum() -> f32 {
    0.72
}
fn default_flicker_strength() -> f32 {
    0.32
}
fn default_wave_period() -> f32 {
    2.8
}
fn default_wave_amplitude() -> f32 {
    0.035
}
fn default_wave_wavelength() -> f32 {
    0.42
}
fn default_typewriter_period() -> f32 {
    4.0
}
fn default_dissolve_period() -> f32 {
    4.0
}
fn default_reveal_hold() -> f32 {
    0.35
}
fn default_dissolve_seed() -> u32 {
    0x504C_4151
}

impl Style {
    pub fn has_frame_variation(&self) -> bool {
        !self.animations.is_empty()
    }

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
        if !matches!(parsed.version, 1..=3) {
            bail!(
                "unsupported text style version {}; this build supports versions 1, 2, and 3",
                parsed.version
            );
        }
        let uses_v2_features = parsed.material.is_some()
            || !parsed.animations.is_empty()
            || parsed.effects.iter().any(|effect| {
                matches!(
                    effect,
                    EffectFile::Extrude { .. } | EffectFile::Bevel { .. }
                )
            });
        let uses_v3_features = matches!(
            parsed.material.as_ref(),
            Some(
                MaterialFile::Chrome { .. }
                    | MaterialFile::Holographic
                    | MaterialFile::Fire { .. }
                    | MaterialFile::Ice { .. }
                    | MaterialFile::Nebula { .. }
                    | MaterialFile::Liquid { .. }
                    | MaterialFile::Halftone { .. }
            )
        ) || parsed.effects.iter().any(|effect| {
            matches!(
                effect,
                EffectFile::Letterpress { .. }
                    | EffectFile::ChromaticSplit { .. }
                    | EffectFile::Trail { .. }
            )
        }) || parsed.animations.iter().any(|animation| {
            matches!(
                animation,
                AnimationFile::Flicker { .. }
                    | AnimationFile::Wave { .. }
                    | AnimationFile::Typewriter { .. }
                    | AnimationFile::Dissolve { .. }
            )
        });
        if parsed.version == 1 && uses_v2_features {
            bail!(
                "text style uses material, animation, extrusion, or bevel features that require version >= 2"
            );
        }
        if parsed.version < 3 && uses_v3_features {
            bail!(
                "text style uses advanced material/effect/animation features that require version = 3"
            );
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
            Some(MaterialFile::Chrome { dark, mid, light }) => FillStyle::Chrome {
                dark: Rgba::parse(&dark).context("invalid chrome dark color")?,
                mid: Rgba::parse(&mid).context("invalid chrome mid color")?,
                light: Rgba::parse(&light).context("invalid chrome light color")?,
            },
            Some(MaterialFile::Holographic) => FillStyle::Holographic,
            Some(MaterialFile::Fire { dark, mid, light }) => FillStyle::Fire {
                dark: Rgba::parse(&dark).context("invalid fire dark color")?,
                mid: Rgba::parse(&mid).context("invalid fire mid color")?,
                light: Rgba::parse(&light).context("invalid fire light color")?,
            },
            Some(MaterialFile::Ice { dark, mid, light }) => FillStyle::Ice {
                dark: Rgba::parse(&dark).context("invalid ice dark color")?,
                mid: Rgba::parse(&mid).context("invalid ice mid color")?,
                light: Rgba::parse(&light).context("invalid ice light color")?,
            },
            Some(MaterialFile::Nebula { dark, mid, light }) => FillStyle::Nebula {
                dark: Rgba::parse(&dark).context("invalid nebula dark color")?,
                mid: Rgba::parse(&mid).context("invalid nebula mid color")?,
                light: Rgba::parse(&light).context("invalid nebula light color")?,
            },
            Some(MaterialFile::Liquid {
                first,
                second,
                frequency,
            }) => {
                if !(0.25..=20.0).contains(&frequency) {
                    bail!("liquid material frequency must be between 0.25 and 20");
                }
                FillStyle::Liquid {
                    first: Rgba::parse(&first).context("invalid liquid first color")?,
                    second: Rgba::parse(&second).context("invalid liquid second color")?,
                    frequency,
                }
            }
            Some(MaterialFile::Halftone {
                foreground,
                background,
                cell,
            }) => {
                if !(2..=64).contains(&cell) {
                    bail!("halftone material cell must be between 2 and 64 pixels");
                }
                FillStyle::Halftone {
                    foreground: Rgba::parse(&foreground).context("invalid halftone foreground")?,
                    background: Rgba::parse(&background).context("invalid halftone background")?,
                    cell,
                }
            }
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
                        shadow: Rgba::parse(&shadow).context("invalid style bevel shadow color")?,
                    });
                }
                EffectFile::Letterpress {
                    width,
                    highlight,
                    shadow,
                } => {
                    if !(0.0..=0.20).contains(&width) {
                        bail!("style letterpress width must be between 0 and 0.20");
                    }
                    overlays.push(OverlayEffect::Letterpress {
                        width_ratio: width,
                        highlight: Rgba::parse(&highlight)
                            .context("invalid letterpress highlight color")?,
                        shadow: Rgba::parse(&shadow).context("invalid letterpress shadow color")?,
                    });
                }
                EffectFile::ChromaticSplit { offset, red, cyan } => {
                    if !(0.0..=0.25).contains(&offset) {
                        bail!("chromatic-split offset must be between 0 and 0.25");
                    }
                    underlays.push(MaskEffect::ChromaticSplit {
                        offset_ratio: offset,
                        red: Rgba::parse(&red).context("invalid chromatic red color")?,
                        cyan: Rgba::parse(&cyan).context("invalid chromatic cyan color")?,
                    });
                }
                EffectFile::Trail {
                    distance,
                    copies,
                    angle_degrees,
                    color,
                } => {
                    if !(0.0..=0.75).contains(&distance) {
                        bail!("trail distance must be between 0 and 0.75");
                    }
                    if !(1..=32).contains(&copies) {
                        bail!("trail copies must be between 1 and 32");
                    }
                    if !angle_degrees.is_finite() {
                        bail!("trail angle must be finite");
                    }
                    underlays.push(MaskEffect::Trail {
                        distance_ratio: distance,
                        copies,
                        angle_degrees,
                        color: Rgba::parse(&color).context("invalid trail color")?,
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
                AnimationFile::Flicker {
                    period_seconds,
                    minimum_opacity,
                    strength,
                    phase,
                } => {
                    if !period_seconds.is_finite() || period_seconds <= 0.0 {
                        bail!("flicker period_seconds must be positive");
                    }
                    if !(0.0..=1.0).contains(&minimum_opacity) || !(0.0..=1.0).contains(&strength) {
                        bail!("flicker minimum_opacity and strength must be between 0 and 1");
                    }
                    AnimationEffect::Flicker {
                        period_seconds,
                        minimum_opacity,
                        strength,
                        phase,
                    }
                }
                AnimationFile::Wave {
                    period_seconds,
                    amplitude,
                    wavelength,
                    phase,
                } => {
                    if !period_seconds.is_finite() || period_seconds <= 0.0 {
                        bail!("wave period_seconds must be positive");
                    }
                    if !(0.0..=0.20).contains(&amplitude) || !(0.05..=4.0).contains(&wavelength) {
                        bail!("wave amplitude must be 0..0.20 and wavelength 0.05..4.0");
                    }
                    AnimationEffect::Wave {
                        period_seconds,
                        amplitude_ratio: amplitude,
                        wavelength_ratio: wavelength,
                        phase,
                    }
                }
                AnimationFile::Typewriter {
                    period_seconds,
                    hold_fraction,
                } => {
                    validate_reveal_animation("typewriter", period_seconds, hold_fraction)?;
                    AnimationEffect::Typewriter {
                        period_seconds,
                        hold_fraction,
                    }
                }
                AnimationFile::Dissolve {
                    period_seconds,
                    hold_fraction,
                    seed,
                } => {
                    validate_reveal_animation("dissolve", period_seconds, hold_fraction)?;
                    AnimationEffect::Dissolve {
                        period_seconds,
                        hold_fraction,
                        seed,
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
            FillStyle::Chrome { dark, mid, light } => format!(
                "chrome(dark={},mid={},light={})",
                format_color(dark),
                format_color(mid),
                format_color(light)
            ),
            FillStyle::Holographic => "holographic".to_string(),
            FillStyle::Fire { dark, mid, light } => format!(
                "fire(dark={},mid={},light={})",
                format_color(dark),
                format_color(mid),
                format_color(light)
            ),
            FillStyle::Ice { dark, mid, light } => format!(
                "ice(dark={},mid={},light={})",
                format_color(dark),
                format_color(mid),
                format_color(light)
            ),
            FillStyle::Nebula { dark, mid, light } => format!(
                "nebula(dark={},mid={},light={})",
                format_color(dark),
                format_color(mid),
                format_color(light)
            ),
            FillStyle::Liquid {
                first,
                second,
                frequency,
            } => format!(
                "liquid(first={},second={},frequency={frequency:.3})",
                format_color(first),
                format_color(second)
            ),
            FillStyle::Halftone {
                foreground,
                background,
                cell,
            } => format!(
                "halftone(foreground={},background={},cell={cell})",
                format_color(foreground),
                format_color(background)
            ),
        }];
        for effect in &self.underlays {
            parts.push(match *effect {
                MaskEffect::Stroke { width_ratio, color } => format!(
                    "stroke(width={width_ratio:.5},color={})", format_color(color)
                ),
                MaskEffect::Glow { radius, color } => {
                    format!("glow(radius={radius},color={})", format_color(color))
                }
                MaskEffect::Shadow { offset_x_ratio, offset_y_ratio, blur_radius, color } => format!(
                    "shadow(x={offset_x_ratio:.5},y={offset_y_ratio:.5},blur={blur_radius},color={})",
                    format_color(color)
                ),
                MaskEffect::Extrude { depth_ratio, angle_degrees, color } => format!(
                    "extrude(depth={depth_ratio:.5},angle={angle_degrees:.2},color={})",
                    format_color(color)
                ),
                MaskEffect::ChromaticSplit { offset_ratio, red, cyan } => format!(
                    "chromatic-split(offset={offset_ratio:.5},red={},cyan={})",
                    format_color(red), format_color(cyan)
                ),
                MaskEffect::Trail { distance_ratio, copies, angle_degrees, color } => format!(
                    "trail(distance={distance_ratio:.5},copies={copies},angle={angle_degrees:.2},color={})",
                    format_color(color)
                ),
            });
        }
        for effect in &self.overlays {
            parts.push(match *effect {
                OverlayEffect::Bevel {
                    width_ratio,
                    highlight,
                    shadow,
                } => format!(
                    "bevel(width={width_ratio:.5},highlight={},shadow={})",
                    format_color(highlight),
                    format_color(shadow)
                ),
                OverlayEffect::Letterpress {
                    width_ratio,
                    highlight,
                    shadow,
                } => format!(
                    "letterpress(width={width_ratio:.5},highlight={},shadow={})",
                    format_color(highlight),
                    format_color(shadow)
                ),
            });
        }
        for animation in &self.animations {
            parts.push(match *animation {
                AnimationEffect::Pulse { period_seconds, minimum_opacity, maximum_opacity, phase } => format!(
                    "pulse(period={period_seconds:.3},min={minimum_opacity:.3},max={maximum_opacity:.3},phase={phase:.3})"
                ),
                AnimationEffect::Shine { period_seconds, width_ratio, angle_degrees, color } => format!(
                    "shine(period={period_seconds:.3},width={width_ratio:.3},angle={angle_degrees:.2},color={})",
                    format_color(color)
                ),
                AnimationEffect::Flicker { period_seconds, minimum_opacity, strength, phase } => format!(
                    "flicker(period={period_seconds:.3},min={minimum_opacity:.3},strength={strength:.3},phase={phase:.3})"
                ),
                AnimationEffect::Wave { period_seconds, amplitude_ratio, wavelength_ratio, phase } => format!(
                    "wave(period={period_seconds:.3},amplitude={amplitude_ratio:.3},wavelength={wavelength_ratio:.3},phase={phase:.3})"
                ),
                AnimationEffect::Typewriter { period_seconds, hold_fraction } => format!(
                    "typewriter(period={period_seconds:.3},hold={hold_fraction:.3})"
                ),
                AnimationEffect::Dissolve { period_seconds, hold_fraction, seed } => format!(
                    "dissolve(period={period_seconds:.3},hold={hold_fraction:.3},seed={seed})"
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
                MaskEffect::ChromaticSplit {
                    offset_ratio,
                    red,
                    cyan,
                } => {
                    let offset = (font_size * offset_ratio).round().max(0.0) as i32;
                    if offset == 0 {
                        continue;
                    }
                    if red.a > 0 {
                        let red_layer = Surface::from_alpha_mask(width, height, &alpha, red)?;
                        combined.blend_surface(&red_layer, -offset, 0, 1.0);
                    }
                    if cyan.a > 0 {
                        let cyan_layer = Surface::from_alpha_mask(width, height, &alpha, cyan)?;
                        combined.blend_surface(&cyan_layer, offset, 0, 1.0);
                    }
                }
                MaskEffect::Trail {
                    distance_ratio,
                    copies,
                    angle_degrees,
                    color,
                } => {
                    if distance_ratio <= 0.0 || copies == 0 || color.a == 0 {
                        continue;
                    }
                    let distance = (font_size * distance_ratio).round().max(1.0);
                    let angle = angle_degrees.to_radians();
                    let dx = angle.cos();
                    let dy = angle.sin();
                    let layer = Surface::from_alpha_mask(width, height, &alpha, color)?;
                    for copy in (1..=copies).rev() {
                        let t = copy as f32 / copies as f32;
                        combined.blend_surface(
                            &layer,
                            (dx * distance * t).round() as i32,
                            (dy * distance * t).round() as i32,
                            0.25 + 0.55 * (1.0 - t),
                        );
                    }
                }
            }
        }

        let fill = self.paint_fill(base, supersampling)?;
        combined.blend_surface(&fill, 0, 0, 1.0);

        for effect in &self.overlays {
            match *effect {
                OverlayEffect::Bevel {
                    width_ratio,
                    highlight,
                    shadow,
                } => {
                    let radius = (font_size * width_ratio).round().max(1.0) as i32;
                    let (highlight_mask, shadow_mask) =
                        directional_bevel_masks(&alpha, width as usize, height as usize, radius);
                    let highlight_layer =
                        Surface::from_alpha_mask(width, height, &highlight_mask, highlight)?;
                    let shadow_layer =
                        Surface::from_alpha_mask(width, height, &shadow_mask, shadow)?;
                    combined.blend_surface(&highlight_layer, 0, 0, 1.0);
                    combined.blend_surface(&shadow_layer, 0, 0, 1.0);
                }
                OverlayEffect::Letterpress {
                    width_ratio,
                    highlight,
                    shadow,
                } => {
                    let radius = (font_size * width_ratio).round().max(1.0) as i32;
                    let (top_left, bottom_right) =
                        directional_bevel_masks(&alpha, width as usize, height as usize, radius);
                    let inset_shadow = Surface::from_alpha_mask(width, height, &top_left, shadow)?;
                    let inset_highlight =
                        Surface::from_alpha_mask(width, height, &bottom_right, highlight)?;
                    combined.blend_surface(&inset_shadow, 0, 0, 1.0);
                    combined.blend_surface(&inset_highlight, 0, 0, 1.0);
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
    /// Alpha-only equivalent of [`Style::fit_envelope`]. Fitting calls this many
    /// times, so routing the strictly geometric probe through an RGBA compositor
    /// would waste most of typography's runtime on colors that are never observed.
    /// The alpha-over operation and effect geometry intentionally match the final
    /// compositor; only RGB material painting is omitted.
    pub fn fit_envelope_alpha(
        &self,
        alpha: &[u8],
        width: u32,
        height: u32,
        font_size: f32,
    ) -> Vec<u8> {
        debug_assert_eq!(alpha.len(), width as usize * height as usize);
        let mut envelope = vec![0_u8; alpha.len()];

        for effect in &self.underlays {
            match *effect {
                MaskEffect::Stroke { width_ratio, .. } => {
                    let radius = (font_size * width_ratio).round().max(0.0) as usize;
                    if radius > 0 {
                        let expanded =
                            dilate_alpha_circular(alpha, width as usize, height as usize, radius);
                        alpha_over_shifted(
                            &mut envelope,
                            &expanded,
                            width as usize,
                            height as usize,
                            0,
                            0,
                        );
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
                    for step in 1..=depth {
                        alpha_over_shifted(
                            &mut envelope,
                            alpha,
                            width as usize,
                            height as usize,
                            (dx * step as f32).round() as i32,
                            (dy * step as f32).round() as i32,
                        );
                    }
                }
                MaskEffect::ChromaticSplit { offset_ratio, .. } => {
                    let offset = (font_size * offset_ratio).round().max(0.0) as i32;
                    if offset > 0 {
                        alpha_over_shifted(
                            &mut envelope,
                            alpha,
                            width as usize,
                            height as usize,
                            -offset,
                            0,
                        );
                        alpha_over_shifted(
                            &mut envelope,
                            alpha,
                            width as usize,
                            height as usize,
                            offset,
                            0,
                        );
                    }
                }
                MaskEffect::Trail {
                    distance_ratio,
                    copies,
                    angle_degrees,
                    ..
                } => {
                    if distance_ratio > 0.0 && copies > 0 {
                        let distance = (font_size * distance_ratio).round().max(1.0);
                        let angle = angle_degrees.to_radians();
                        alpha_over_shifted(
                            &mut envelope,
                            alpha,
                            width as usize,
                            height as usize,
                            (angle.cos() * distance).round() as i32,
                            (angle.sin() * distance).round() as i32,
                        );
                    }
                }
                MaskEffect::Glow { .. } | MaskEffect::Shadow { .. } => {}
            }
        }

        for animation in &self.animations {
            if let AnimationEffect::Wave {
                amplitude_ratio, ..
            } = *animation
            {
                let glyph_height = alpha_bounds(alpha, width as usize)
                    .map(|bounds| bounds.3 - bounds.1 + 1)
                    .unwrap_or(1) as f32;
                let amplitude = (glyph_height * amplitude_ratio).round().max(0.0) as i32;
                if amplitude > 0 {
                    alpha_over_shifted(
                        &mut envelope,
                        alpha,
                        width as usize,
                        height as usize,
                        0,
                        -amplitude,
                    );
                    alpha_over_shifted(
                        &mut envelope,
                        alpha,
                        width as usize,
                        height as usize,
                        0,
                        amplitude,
                    );
                }
            }
        }

        alpha_over_shifted(&mut envelope, alpha, width as usize, height as usize, 0, 0);
        envelope
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
                AnimationEffect::Flicker {
                    period_seconds,
                    minimum_opacity,
                    strength,
                    phase,
                } => {
                    let p = time_seconds / period_seconds as f64 + phase as f64;
                    let fast = (std::f64::consts::TAU * p * 11.0).sin() * 0.5 + 0.5;
                    let slow = (std::f64::consts::TAU * p * 3.0 + 1.7).sin() * 0.5 + 0.5;
                    let signal = (0.72 * fast + 0.28 * slow) as f32;
                    (1.0 - strength * (1.0 - signal)).max(minimum_opacity)
                }
                AnimationEffect::Shine { .. }
                | AnimationEffect::Wave { .. }
                | AnimationEffect::Typewriter { .. }
                | AnimationEffect::Dissolve { .. } => 1.0,
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
            let center = min_projection - stripe_width + progress * (span + stripe_width * 2.0);
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
                    shine.set_pixel(x, y, Rgba::new(color.r, color.g, color.b, animated_alpha));
                }
            }
            match &mut output {
                Some(existing) => existing.blend_surface(&shine, 0, 0, 1.0),
                None => output = Some(shine),
            }
        }
        Ok(output)
    }

    /// Apply frame-varying geometry/reveal effects after static paint and shine have
    /// been composed. This is deliberately raster-level: font shaping remains cached.
    pub fn frame_transform(&self, layer: &Surface, time_seconds: f64) -> Result<Surface> {
        let mut current = layer.clone();
        for animation in &self.animations {
            current = match *animation {
                AnimationEffect::Wave {
                    period_seconds,
                    amplitude_ratio,
                    wavelength_ratio,
                    phase,
                } => wave_surface(
                    &current,
                    time_seconds,
                    period_seconds,
                    amplitude_ratio,
                    wavelength_ratio,
                    phase,
                ),
                AnimationEffect::Typewriter {
                    period_seconds,
                    hold_fraction,
                } => reveal_surface(
                    &current,
                    reveal_progress(time_seconds, period_seconds, hold_fraction),
                ),
                AnimationEffect::Dissolve {
                    period_seconds,
                    hold_fraction,
                    seed,
                } => dissolve_surface(
                    &current,
                    reveal_progress(time_seconds, period_seconds, hold_fraction),
                    seed,
                ),
                AnimationEffect::Pulse { .. }
                | AnimationEffect::Shine { .. }
                | AnimationEffect::Flicker { .. } => current,
            };
        }
        Ok(current)
    }

    fn paint_fill(&self, base: &Surface, supersampling: u32) -> Result<Surface> {
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
            FillStyle::Chrome { dark, mid, light } => {
                paint_vertical_material(base, |t| chrome_color(dark, mid, light, t))
            }
            FillStyle::Holographic => paint_xy_material(base, |x, y| {
                let hue = (x * 0.72 + y * 0.46).rem_euclid(1.0);
                hsv_color(hue, 0.62, 1.0, 255)
            }),
            FillStyle::Fire { dark, mid, light } => paint_xy_material(base, |x, y| {
                let flame = (y + 0.12 * (x * std::f32::consts::TAU * 3.0).sin()).clamp(0.0, 1.0);
                if flame < 0.58 {
                    lerp_color(light, mid, flame / 0.58)
                } else {
                    lerp_color(mid, dark, (flame - 0.58) / 0.42)
                }
            }),
            FillStyle::Ice { dark, mid, light } => paint_xy_material(base, |x, y| {
                let crystal = ((x * 17.0 + y * 11.0).sin().abs() * 0.22 + y).clamp(0.0, 1.0);
                if crystal < 0.48 {
                    lerp_color(light, mid, crystal / 0.48)
                } else {
                    lerp_color(mid, dark, (crystal - 0.48) / 0.52)
                }
            }),
            FillStyle::Nebula { dark, mid, light } => paint_xy_material(base, |x, y| {
                let swirl = ((x * 8.3 + y * 5.7).sin() * 0.5 + 0.5) * 0.45 + y * 0.55;
                if swirl < 0.5 {
                    lerp_color(dark, mid, swirl * 2.0)
                } else {
                    lerp_color(mid, light, (swirl - 0.5) * 2.0)
                }
            }),
            FillStyle::Liquid {
                first,
                second,
                frequency,
            } => paint_xy_material(base, |x, y| {
                let wave = ((x * frequency * std::f32::consts::TAU + y * 5.0).sin() * 0.5 + 0.5)
                    * 0.55
                    + y * 0.45;
                lerp_color(first, second, wave.clamp(0.0, 1.0))
            }),
            FillStyle::Halftone {
                foreground,
                background,
                cell,
            } => paint_halftone_material(
                base,
                foreground,
                background,
                cell.saturating_mul(supersampling.max(1)),
            ),
        }
    }
}

fn paint_vertical_material(base: &Surface, color_at: impl Fn(f32) -> Rgba) -> Result<Surface> {
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

fn paint_xy_material(base: &Surface, color_at: impl Fn(f32, f32) -> Rgba) -> Result<Surface> {
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

fn paint_halftone_material(
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

fn chrome_color(dark: Rgba, mid: Rgba, light: Rgba, t: f32) -> Rgba {
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

fn hsv_color(hue: f32, saturation: f32, value: f32, alpha: u8) -> Rgba {
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

fn validate_reveal_animation(name: &str, period_seconds: f32, hold_fraction: f32) -> Result<()> {
    if !period_seconds.is_finite() || period_seconds <= 0.0 {
        bail!("{name} period_seconds must be positive");
    }
    if !(0.0..0.95).contains(&hold_fraction) {
        bail!("{name} hold_fraction must be between 0 and 0.95");
    }
    Ok(())
}

fn reveal_progress(time_seconds: f64, period_seconds: f32, hold_fraction: f32) -> f32 {
    let phase = (time_seconds / period_seconds as f64).rem_euclid(1.0) as f32;
    let reveal = (1.0 - hold_fraction).max(0.05);
    if phase >= reveal {
        1.0
    } else {
        (phase / reveal).clamp(0.0, 1.0)
    }
}

fn wave_surface(
    source: &Surface,
    time_seconds: f64,
    period_seconds: f32,
    amplitude_ratio: f32,
    wavelength_ratio: f32,
    phase: f32,
) -> Surface {
    let Some(bounds) = source.alpha_bounds() else {
        return source.clone();
    };
    let glyph_height = (bounds.3 - bounds.1 + 1).max(1) as f32;
    let glyph_width = (bounds.2 - bounds.0 + 1).max(1) as f32;
    let amplitude = glyph_height * amplitude_ratio;
    let wavelength = (glyph_width * wavelength_ratio).max(1.0);
    let temporal = std::f32::consts::TAU * (time_seconds as f32 / period_seconds + phase);
    let mut output = Surface::new(source.width(), source.height());
    for y in 0..source.height() {
        for x in 0..source.width() {
            let shift = amplitude
                * (std::f32::consts::TAU * (x as f32 - bounds.0 as f32) / wavelength + temporal)
                    .sin();
            let sy = (y as f32 - shift).round() as i32;
            if sy >= 0 && sy < source.height() as i32 {
                output.set_pixel(x, y, source.pixel(x, sy as u32));
            }
        }
    }
    output
}

fn reveal_surface(source: &Surface, progress: f32) -> Surface {
    let Some(bounds) = source.alpha_bounds() else {
        return source.clone();
    };
    let right = bounds.0 as f32 + (bounds.2 - bounds.0 + 1) as f32 * progress.clamp(0.0, 1.0);
    let mut output = Surface::new(source.width(), source.height());
    for y in 0..source.height() {
        for x in 0..source.width() {
            if x as f32 <= right {
                output.set_pixel(x, y, source.pixel(x, y));
            }
        }
    }
    output
}

fn dissolve_surface(source: &Surface, progress: f32, seed: u32) -> Surface {
    let progress = progress.clamp(0.0, 1.0);
    let mut output = Surface::new(source.width(), source.height());
    for y in 0..source.height() {
        for x in 0..source.width() {
            let pixel = source.pixel(x, y);
            if pixel.a == 0 {
                continue;
            }
            let mut hash = seed ^ x.wrapping_mul(0x9E37_79B9) ^ y.wrapping_mul(0x85EB_CA6B);
            hash ^= hash >> 16;
            hash = hash.wrapping_mul(0x7FEB_352D);
            hash ^= hash >> 15;
            let threshold = (hash & 0xFFFF) as f32 / 65535.0;
            if threshold <= progress {
                output.set_pixel(x, y, pixel);
            }
        }
    }
    output
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
            let top_left =
                sample_alpha(source, width, height, x as i32 - radius, y as i32 - radius);
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

fn alpha_bounds(source: &[u8], width: usize) -> Option<(u32, u32, u32, u32)> {
    if width == 0 || source.is_empty() || !source.len().is_multiple_of(width) {
        return None;
    }
    let height = source.len() / width;
    let (mut x0, mut y0, mut x1, mut y1) = (width, height, 0_usize, 0_usize);
    let mut any = false;
    for (index, &value) in source.iter().enumerate() {
        if value == 0 {
            continue;
        }
        let x = index % width;
        let y = index / width;
        any = true;
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    any.then_some((x0 as u32, y0 as u32, x1 as u32, y1 as u32))
}

fn alpha_over_shifted(
    output: &mut [u8],
    input: &[u8],
    width: usize,
    height: usize,
    dx: i32,
    dy: i32,
) {
    debug_assert_eq!(output.len(), width * height);
    debug_assert_eq!(input.len(), width * height);
    let left = dx.max(0) as usize;
    let top = dy.max(0) as usize;
    let right = (width as i32 + dx).min(width as i32).max(0) as usize;
    let bottom = (height as i32 + dy).min(height as i32).max(0) as usize;
    for y in top..bottom {
        let source_y = (y as i32 - dy) as usize;
        for x in left..right {
            let source_x = (x as i32 - dx) as usize;
            let source_alpha = input[source_y * width + source_x] as u16;
            let destination = &mut output[y * width + x];
            let remaining = (255 - *destination as u16) * (255 - source_alpha);
            *destination = (255 - (remaining + 127) / 255) as u8;
        }
    }
}

fn dilate_alpha_circular(source: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    if radius == 0 {
        return source.to_vec();
    }
    let Some((x0, y0, x1, y1)) = alpha_bounds(source, width) else {
        return vec![0; source.len()];
    };
    let (x0, y0, x1, y1) = (x0 as usize, y0 as usize, x1 as usize, y1 as usize);

    // Exact grayscale disk dilation. For each vertical disk offset, compute the
    // corresponding horizontal max-filter with a monotonic deque. This preserves
    // every output byte while reducing O(width*height*radius²) to
    // O(ink-bounds*radius).
    let radius_squared = radius * radius;
    let row_spans = (0..=radius)
        .map(|dy| {
            let dx = ((radius_squared - dy * dy) as f64).sqrt().floor() as usize;
            (dy, dx)
        })
        .collect::<Vec<_>>();
    let mut output = vec![0u8; source.len()];
    for (dy, dx) in row_spans {
        let left = x0.saturating_sub(dx);
        let right = x1.saturating_add(dx).min(width - 1);
        let mut filtered = vec![0_u8; right - left + 1];
        for source_y in y0..=y1 {
            horizontal_max_filter(
                &source[source_y * width..(source_y + 1) * width],
                x0,
                x1,
                dx,
                left,
                &mut filtered,
            );
            if let Some(target_y) = source_y.checked_sub(dy) {
                merge_max(
                    &mut output[target_y * width + left..=target_y * width + right],
                    &filtered,
                );
            }
            if dy > 0 && source_y + dy < height {
                let target_y = source_y + dy;
                merge_max(
                    &mut output[target_y * width + left..=target_y * width + right],
                    &filtered,
                );
            }
        }
    }
    output
}

fn horizontal_max_filter(
    source: &[u8],
    source_left: usize,
    source_right: usize,
    radius: usize,
    output_left: usize,
    output: &mut [u8],
) {
    let mut deque = std::collections::VecDeque::<usize>::new();
    let mut next = source_left;
    for (offset, destination) in output.iter_mut().enumerate() {
        let x = output_left + offset;
        let window_right = x.saturating_add(radius).min(source_right);
        while next <= window_right {
            while deque
                .back()
                .is_some_and(|&index| source[index] <= source[next])
            {
                deque.pop_back();
            }
            deque.push_back(next);
            next += 1;
        }
        let window_left = x.saturating_sub(radius).max(source_left);
        while deque.front().is_some_and(|&index| index < window_left) {
            deque.pop_front();
        }
        *destination = deque.front().map_or(0, |&index| source[index]);
    }
}

fn merge_max(output: &mut [u8], input: &[u8]) {
    for (output, &input) in output.iter_mut().zip(input) {
        *output = (*output).max(input);
    }
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
    fn optimized_disk_dilation_is_byte_exact() {
        let width = 23;
        let height = 17;
        let mut source = vec![0_u8; width * height];
        for (index, value) in source.iter_mut().enumerate() {
            if index % 11 == 0 || index % 29 == 0 {
                *value = ((index * 73) % 255 + 1) as u8;
            }
        }
        for radius in 1..=6 {
            let expected = slow_disk_dilation(&source, width, height, radius);
            assert_eq!(
                dilate_alpha_circular(&source, width, height, radius),
                expected
            );
        }
    }

    fn slow_disk_dilation(source: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
        let radius_squared = (radius * radius) as isize;
        let mut output = vec![0; source.len()];
        for y in 0..height {
            for x in 0..width {
                for dy in -(radius as isize)..=radius as isize {
                    for dx in -(radius as isize)..=radius as isize {
                        if dx * dx + dy * dy > radius_squared {
                            continue;
                        }
                        let xx = x as isize + dx;
                        let yy = y as isize + dy;
                        if (0..width as isize).contains(&xx) && (0..height as isize).contains(&yy) {
                            output[y * width + x] = output[y * width + x]
                                .max(source[yy as usize * width + xx as usize]);
                        }
                    }
                }
            }
        }
        output
    }

    #[test]
    fn bevel_separates_opposite_edges() {
        let source = vec![255; 25];
        let (highlight, shadow) = directional_bevel_masks(&source, 5, 5, 1);
        assert!(highlight[0] > 0);
        assert!(shadow[24] > 0);
    }
}
