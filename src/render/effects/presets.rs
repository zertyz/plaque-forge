//! Preset style loading, CLI direct option mapping, and TOML validation.

use std::{fs, path::Path};

use anyhow::{Context, Result, bail};

use crate::{color::Rgba, surface::Surface};

use super::{
    shaders::validate_reveal_animation,
    types::{
        AnimationEffect, AnimationFile, DirectStyleOptions, EffectFile, FillStyle, LayoutEffect,
        LayoutFile, MaskEffect, MaterialFile, OverlayEffect, StyleFile, SurfaceEffect,
        SurfaceEffectFile, TextureMaterial, default_font_weight,
    },
};

#[derive(Clone, Debug)]
pub struct Style {
    font_weight: u16,
    pub(crate) fill: FillStyle,
    pub(crate) texture: Option<TextureMaterial>,
    pub(crate) layouts: Vec<LayoutEffect>,
    pub(crate) underlays: Vec<MaskEffect>,
    pub(crate) overlays: Vec<OverlayEffect>,
    pub(crate) surface_effects: Vec<SurfaceEffect>,
    pub(crate) animations: Vec<AnimationEffect>,
}

fn validate_font_weight(weight: u16) -> Result<()> {
    if !(1..=1000).contains(&weight) {
        bail!("font weight must be between 1 and 1000");
    }
    Ok(())
}

impl Style {
    /// Construct a style directly from CLI-equivalent options without a file.
    #[cfg(test)]
    pub fn direct(options: DirectStyleOptions<'_>) -> Result<Self> {
        Self::load(None, options)
    }

    pub(crate) fn font_weight(&self) -> u16 {
        self.font_weight
    }

    pub fn has_frame_variation(&self) -> bool {
        !self.animations.is_empty()
    }

    pub fn load(style_file: Option<&Path>, direct: DirectStyleOptions<'_>) -> Result<Self> {
        if let Some(path) = style_file {
            return Self::from_file(path);
        }

        let DirectStyleOptions {
            font_weight,
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

        validate_font_weight(font_weight)?;
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
            font_weight,
            fill: FillStyle::Flat(Rgba::parse(text_color).context("invalid --text-color")?),
            texture: None,
            layouts: Vec::new(),
            underlays,
            overlays: Vec::new(),
            surface_effects: Vec::new(),
            animations: Vec::new(),
        })
    }

    pub fn from_file(path: &Path) -> Result<Self> {
        let source = fs::read_to_string(path)
            .with_context(|| format!("failed to read text style {}", path.display()))?;
        let parsed: StyleFile = toml::from_str(&source)
            .with_context(|| format!("invalid text style TOML {}", path.display()))?;
        if !matches!(parsed.version, 1..=5) {
            bail!(
                "unsupported text style version {}; this build supports versions 1 through 5",
                parsed.version
            );
        }
        if parsed.version < 5 && parsed.typography.is_some() {
            bail!("text style typography options require version = 5");
        }
        let font_weight = parsed
            .typography
            .as_ref()
            .map_or_else(default_font_weight, |typography| typography.weight);
        validate_font_weight(font_weight)?;

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
        let uses_v4_features = !parsed.layouts.is_empty()
            || !parsed.surface_effects.is_empty()
            || matches!(
                parsed.material.as_ref(),
                Some(
                    MaterialFile::ImageTexture { .. }
                        | MaterialFile::Blueprint { .. }
                        | MaterialFile::Paper { .. }
                )
            )
            || parsed.animations.iter().any(|animation| {
                matches!(
                    animation,
                    AnimationFile::Scramble { .. }
                        | AnimationFile::SplitFlap { .. }
                        | AnimationFile::ConfettiConverge { .. }
                        | AnimationFile::Glitch { .. }
                        | AnimationFile::Orbit { .. }
                )
            });
        if parsed.version == 1 && uses_v2_features {
            bail!(
                "text style uses material, animation, extrusion, or bevel features that require version >= 2"
            );
        }
        if parsed.version < 3 && uses_v3_features {
            bail!(
                "text style uses advanced material/effect/animation features that require version >= 3"
            );
        }
        if parsed.version < 4 && uses_v4_features {
            bail!(
                "text style uses layout, texture, character, particle, or plaque-surface features that require version >= 4"
            );
        }
        if parsed.fill.is_some() && parsed.material.is_some() {
            bail!("style must declare either fill or material, not both");
        }

        let mut texture = None;
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
            Some(MaterialFile::ImageTexture {
                path: texture_path,
                tile,
                scale,
                offset_x,
                offset_y,
            }) => {
                if !(0.05..=20.0).contains(&scale) || !offset_x.is_finite() || !offset_y.is_finite()
                {
                    bail!("image-texture scale must be 0.05..20 and offsets must be finite");
                }
                let resolved = path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(&texture_path);
                let image = image::open(&resolved)
                    .with_context(|| format!("failed to load text texture {}", resolved.display()))?
                    .to_rgba8();
                let surface = Surface::from_rgba(image.width(), image.height(), image.into_raw())?;
                texture = Some(TextureMaterial {
                    image: surface,
                    sha256: crate::digest::file_sha256(&resolved)?,
                    path: resolved,
                    tile,
                    scale,
                    offset_x,
                    offset_y,
                });
                FillStyle::Flat(Rgba::new(255, 255, 255, 255))
            }
            Some(MaterialFile::Blueprint {
                dark,
                light,
                grid,
                cell,
            }) => {
                if !(3..=64).contains(&cell) {
                    bail!("blueprint cell must be between 3 and 64 pixels");
                }
                FillStyle::Blueprint {
                    dark: Rgba::parse(&dark).context("invalid blueprint dark color")?,
                    light: Rgba::parse(&light).context("invalid blueprint light color")?,
                    grid: Rgba::parse(&grid).context("invalid blueprint grid color")?,
                    cell,
                }
            }
            Some(MaterialFile::Paper {
                light,
                mid,
                dark,
                seed,
            }) => FillStyle::Paper {
                light: Rgba::parse(&light).context("invalid paper light color")?,
                mid: Rgba::parse(&mid).context("invalid paper mid color")?,
                dark: Rgba::parse(&dark).context("invalid paper dark color")?,
                seed,
            },
        };

        let mut layouts = Vec::new();
        for layout in parsed.layouts {
            match layout {
                LayoutFile::Arc {
                    sweep_degrees,
                    radius_scale,
                } => {
                    if !sweep_degrees.is_finite() || !(1.0..=330.0).contains(&sweep_degrees.abs()) {
                        bail!("arc sweep_degrees magnitude must be between 1 and 330");
                    }
                    if !(0.2..=5.0).contains(&radius_scale) {
                        bail!("arc radius_scale must be between 0.2 and 5");
                    }
                    layouts.push(LayoutEffect::Arc {
                        sweep_degrees,
                        radius_scale,
                    });
                }
            }
        }

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

        let mut surface_effects = Vec::new();
        for effect in parsed.surface_effects {
            surface_effects.push(match effect {
                SurfaceEffectFile::LaserBurn {
                    depth,
                    warmth,
                    edge_width,
                    seed,
                } => {
                    if !(0.0..=1.0).contains(&depth)
                        || !(0.0..=1.0).contains(&warmth)
                        || edge_width > 24
                    {
                        bail!("laser-burn depth/warmth must be 0..1 and edge_width <= 24");
                    }
                    SurfaceEffect::LaserBurn {
                        depth,
                        warmth,
                        edge_width,
                        seed,
                    }
                }
                SurfaceEffectFile::Emboss {
                    depth,
                    highlight_strength,
                    shadow_strength,
                    light_angle_degrees,
                    cast_shadow,
                } => {
                    if !(0.0..=2.0).contains(&depth)
                        || !(0.0..=1.5).contains(&highlight_strength)
                        || !(0.0..=1.5).contains(&shadow_strength)
                        || light_angle_degrees.is_some_and(|angle| !angle.is_finite())
                        || cast_shadow > 32
                    {
                        bail!("emboss parameters are outside their supported ranges");
                    }
                    SurfaceEffect::Emboss {
                        depth,
                        highlight_strength,
                        shadow_strength,
                        light_angle_degrees,
                        cast_shadow,
                    }
                }
            });
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
                    if !(0.0..=1.0).contains(&minimum_opacity)
                        || !(0.0..=1.0).contains(&strength)
                    {
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
                AnimationFile::Scramble {
                    period_seconds,
                    hold_fraction,
                    steps_per_second,
                    seed,
                } => {
                    validate_reveal_animation("scramble", period_seconds, hold_fraction)?;
                    if !(1.0..=60.0).contains(&steps_per_second) {
                        bail!("scramble steps_per_second must be between 1 and 60");
                    }
                    AnimationEffect::Scramble {
                        period_seconds,
                        hold_fraction,
                        steps_per_second,
                        seed,
                    }
                }
                AnimationFile::SplitFlap {
                    period_seconds,
                    hold_fraction,
                    steps_per_second,
                } => {
                    validate_reveal_animation("split-flap", period_seconds, hold_fraction)?;
                    if !(1.0..=60.0).contains(&steps_per_second) {
                        bail!("split-flap steps_per_second must be between 1 and 60");
                    }
                    AnimationEffect::SplitFlap {
                        period_seconds,
                        hold_fraction,
                        steps_per_second,
                    }
                }
                AnimationFile::ConfettiConverge {
                    period_seconds,
                    hold_fraction,
                    pieces,
                    spread,
                    seed,
                } => {
                    validate_reveal_animation("confetti-converge", period_seconds, hold_fraction)?;
                    if !(32..=10_000).contains(&pieces) || !(0.05..=2.0).contains(&spread) {
                        bail!("confetti pieces must be 32..10000 and spread 0.05..2.0");
                    }
                    AnimationEffect::ConfettiConverge {
                        period_seconds,
                        hold_fraction,
                        pieces,
                        spread_ratio: spread,
                        seed,
                    }
                }
                AnimationFile::Glitch {
                    period_seconds,
                    ripple,
                    slice,
                    burst_fraction,
                    seed,
                } => {
                    if !period_seconds.is_finite()
                        || period_seconds <= 0.0
                        || !(0.0..=0.15).contains(&ripple)
                        || !(0.0..=0.30).contains(&slice)
                        || !(0.01..=0.95).contains(&burst_fraction)
                    {
                        bail!(
                            "glitch period/ripple/slice/burst parameters are outside their supported ranges"
                        );
                    }
                    AnimationEffect::Glitch {
                        period_seconds,
                        ripple_ratio: ripple,
                        slice_ratio: slice,
                        burst_fraction,
                        seed,
                    }
                }
                AnimationFile::Orbit {
                    period_seconds,
                    degrees_per_cycle,
                    phase,
                } => {
                    if !period_seconds.is_finite()
                        || period_seconds <= 0.0
                        || !degrees_per_cycle.is_finite()
                    {
                        bail!("orbit period must be positive and degrees_per_cycle finite");
                    }
                    AnimationEffect::Orbit {
                        period_seconds,
                        degrees_per_cycle,
                        phase,
                    }
                }
            });
        }

        Ok(Self {
            font_weight,
            fill,
            texture,
            layouts,
            underlays,
            overlays,
            surface_effects,
            animations,
        })
    }
}
