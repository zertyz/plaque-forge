//! Types, enums, options, and TOML schemas for text effects.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{color::Rgba, surface::Surface};

#[derive(Clone, Debug)]
pub(crate) struct TextureMaterial {
    pub(crate) image: Surface,
    pub(crate) path: PathBuf,
    pub(crate) sha256: String,
    pub(crate) tile: bool,
    pub(crate) scale: f32,
    pub(crate) offset_x: f32,
    pub(crate) offset_y: f32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum LayoutEffect {
    Arc {
        sweep_degrees: f32,
        radius_scale: f32,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum SurfaceEffect {
    LaserBurn {
        depth: f32,
        warmth: f32,
        edge_width: u32,
        seed: u32,
    },
    Emboss {
        depth: f32,
        highlight_strength: f32,
        shadow_strength: f32,
        light_angle_degrees: Option<f32>,
        cast_shadow: u32,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct DirectStyleOptions<'a> {
    pub font_weight: u16,
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
pub(crate) enum FillStyle {
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
    Blueprint {
        dark: Rgba,
        light: Rgba,
        grid: Rgba,
        cell: u32,
    },
    Paper {
        light: Rgba,
        mid: Rgba,
        dark: Rgba,
        seed: u32,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum MaskEffect {
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

/// Geometry of an extruded mask layer: step depth plus unit direction.
pub(crate) struct ExtrudeGeometry {
    pub depth: i32,
    pub dx: f32,
    pub dy: f32,
}

/// Displacement geometry of a motion trail: reach in pixels plus unit direction.
pub(crate) struct TrailGeometry {
    pub distance: f32,
    pub dx: f32,
    pub dy: f32,
}

impl MaskEffect {
    /// Effect sizes scale with the fitted font size; each helper fixes the
    /// rounding/clamping rule shared by painting and fit-envelope derivation.
    pub(crate) fn stroke_radius_px(&self, font_size: f32) -> usize {
        match *self {
            MaskEffect::Stroke { width_ratio, .. } => {
                (font_size * width_ratio).round().max(0.0) as usize
            }
            _ => 0,
        }
    }

    pub(crate) fn extrude_geometry(&self, font_size: f32) -> Option<ExtrudeGeometry> {
        match *self {
            MaskEffect::Extrude {
                depth_ratio,
                angle_degrees,
                ..
            } if depth_ratio > 0.0 => {
                let angle = angle_degrees.to_radians();
                Some(ExtrudeGeometry {
                    depth: (font_size * depth_ratio).round().max(1.0) as i32,
                    dx: angle.cos(),
                    dy: angle.sin(),
                })
            }
            _ => None,
        }
    }

    pub(crate) fn chromatic_offset_px(&self, font_size: f32) -> i32 {
        match *self {
            MaskEffect::ChromaticSplit { offset_ratio, .. } => {
                (font_size * offset_ratio).round().max(0.0) as i32
            }
            _ => 0,
        }
    }

    pub(crate) fn trail_geometry(&self, font_size: f32) -> Option<TrailGeometry> {
        match *self {
            MaskEffect::Trail {
                distance_ratio,
                copies,
                angle_degrees,
                ..
            } if distance_ratio > 0.0 && copies > 0 => {
                let angle = angle_degrees.to_radians();
                Some(TrailGeometry {
                    distance: (font_size * distance_ratio).round().max(1.0),
                    dx: angle.cos(),
                    dy: angle.sin(),
                })
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum OverlayEffect {
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
pub(crate) enum AnimationEffect {
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
    Scramble {
        period_seconds: f32,
        hold_fraction: f32,
        steps_per_second: f32,
        seed: u32,
    },
    SplitFlap {
        period_seconds: f32,
        hold_fraction: f32,
        steps_per_second: f32,
    },
    ConfettiConverge {
        period_seconds: f32,
        hold_fraction: f32,
        pieces: u32,
        spread_ratio: f32,
        seed: u32,
    },
    Glitch {
        period_seconds: f32,
        ripple_ratio: f32,
        slice_ratio: f32,
        burst_fraction: f32,
        seed: u32,
    },
    Orbit {
        period_seconds: f32,
        degrees_per_cycle: f32,
        phase: f32,
    },
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct TypographyFile {
    #[serde(default = "default_font_weight")]
    pub(crate) weight: u16,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct StyleFile {
    #[serde(default = "default_style_version")]
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) typography: Option<TypographyFile>,
    #[serde(default)]
    pub(crate) fill: Option<String>,
    #[serde(default)]
    pub(crate) material: Option<MaterialFile>,
    #[serde(default)]
    pub(crate) layouts: Vec<LayoutFile>,
    #[serde(default)]
    pub(crate) effects: Vec<EffectFile>,
    #[serde(default)]
    pub(crate) surface_effects: Vec<SurfaceEffectFile>,
    #[serde(default)]
    pub(crate) animations: Vec<AnimationFile>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(crate) enum LayoutFile {
    Arc {
        #[serde(default = "default_arc_sweep")]
        sweep_degrees: f32,
        #[serde(default = "default_arc_radius_scale")]
        radius_scale: f32,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(crate) enum SurfaceEffectFile {
    LaserBurn {
        #[serde(default = "default_laser_depth")]
        depth: f32,
        #[serde(default = "default_laser_warmth")]
        warmth: f32,
        #[serde(default = "default_laser_edge_width")]
        edge_width: u32,
        #[serde(default = "default_surface_seed")]
        seed: u32,
    },
    Emboss {
        #[serde(default = "default_emboss_depth")]
        depth: f32,
        #[serde(default = "default_emboss_highlight")]
        highlight_strength: f32,
        #[serde(default = "default_emboss_shadow")]
        shadow_strength: f32,
        #[serde(default)]
        light_angle_degrees: Option<f32>,
        #[serde(default = "default_emboss_cast_shadow")]
        cast_shadow: u32,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(crate) enum MaterialFile {
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
    ImageTexture {
        path: String,
        #[serde(default)]
        tile: bool,
        #[serde(default = "default_texture_scale")]
        scale: f32,
        #[serde(default)]
        offset_x: f32,
        #[serde(default)]
        offset_y: f32,
    },
    Blueprint {
        #[serde(default = "default_blueprint_dark")]
        dark: String,
        #[serde(default = "default_blueprint_light")]
        light: String,
        #[serde(default = "default_blueprint_grid")]
        grid: String,
        #[serde(default = "default_blueprint_cell")]
        cell: u32,
    },
    Paper {
        #[serde(default = "default_paper_light")]
        light: String,
        #[serde(default = "default_paper_mid")]
        mid: String,
        #[serde(default = "default_paper_dark")]
        dark: String,
        #[serde(default = "default_paper_seed")]
        seed: u32,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(crate) enum EffectFile {
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

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(crate) enum AnimationFile {
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
    Scramble {
        #[serde(default = "default_scramble_period")]
        period_seconds: f32,
        #[serde(default = "default_reveal_hold")]
        hold_fraction: f32,
        #[serde(default = "default_character_steps")]
        steps_per_second: f32,
        #[serde(default = "default_scramble_seed")]
        seed: u32,
    },
    SplitFlap {
        #[serde(default = "default_split_flap_period")]
        period_seconds: f32,
        #[serde(default = "default_reveal_hold")]
        hold_fraction: f32,
        #[serde(default = "default_character_steps")]
        steps_per_second: f32,
    },
    ConfettiConverge {
        #[serde(default = "default_confetti_period")]
        period_seconds: f32,
        #[serde(default = "default_reveal_hold")]
        hold_fraction: f32,
        #[serde(default = "default_confetti_pieces")]
        pieces: u32,
        #[serde(default = "default_confetti_spread")]
        spread: f32,
        #[serde(default = "default_confetti_seed")]
        seed: u32,
    },
    Glitch {
        #[serde(default = "default_glitch_period")]
        period_seconds: f32,
        #[serde(default = "default_glitch_ripple")]
        ripple: f32,
        #[serde(default = "default_glitch_slice")]
        slice: f32,
        #[serde(default = "default_glitch_burst")]
        burst_fraction: f32,
        #[serde(default = "default_glitch_seed")]
        seed: u32,
    },
    Orbit {
        #[serde(default = "default_orbit_period")]
        period_seconds: f32,
        #[serde(default = "default_orbit_degrees")]
        degrees_per_cycle: f32,
        #[serde(default)]
        phase: f32,
    },
}

pub(crate) fn default_style_version() -> u32 {
    1
}

pub(crate) fn default_font_weight() -> u16 {
    600
}
pub(crate) fn default_shadow_x() -> f32 {
    0.035
}
pub(crate) fn default_shadow_y() -> f32 {
    0.045
}
pub(crate) fn default_shadow_blur() -> u32 {
    6
}
pub(crate) fn default_shadow_color() -> String {
    "#000000A0".to_string()
}
pub(crate) fn default_extrude_angle() -> f32 {
    55.0
}
pub(crate) fn default_extrude_color() -> String {
    "#2A1608D8".to_string()
}
pub(crate) fn default_bevel_highlight() -> String {
    "#FFF1C0B8".to_string()
}
pub(crate) fn default_bevel_shadow() -> String {
    "#321B08B8".to_string()
}
pub(crate) fn default_gold_dark() -> String {
    "#5B3210FF".to_string()
}
pub(crate) fn default_gold_mid() -> String {
    "#C98B3CFF".to_string()
}
pub(crate) fn default_gold_light() -> String {
    "#F3D38AFF".to_string()
}
pub(crate) fn default_gold_highlight() -> String {
    "#FFF1C4FF".to_string()
}
pub(crate) fn default_pulse_period() -> f32 {
    2.4
}
pub(crate) fn default_pulse_minimum() -> f32 {
    0.82
}
pub(crate) fn default_pulse_maximum() -> f32 {
    1.0
}
pub(crate) fn default_shine_period() -> f32 {
    2.8
}
pub(crate) fn default_shine_width() -> f32 {
    0.18
}
pub(crate) fn default_shine_angle() -> f32 {
    35.0
}
pub(crate) fn default_shine_color() -> String {
    "#FFFFFFB8".to_string()
}
pub(crate) fn default_chrome_dark() -> String {
    "#182436FF".to_string()
}
pub(crate) fn default_chrome_mid() -> String {
    "#8EA9C7FF".to_string()
}
pub(crate) fn default_chrome_light() -> String {
    "#F7FBFFFF".to_string()
}
pub(crate) fn default_fire_dark() -> String {
    "#380707FF".to_string()
}
pub(crate) fn default_fire_mid() -> String {
    "#D84A0FFF".to_string()
}
pub(crate) fn default_fire_light() -> String {
    "#FFE066FF".to_string()
}
pub(crate) fn default_ice_dark() -> String {
    "#0B2447FF".to_string()
}
pub(crate) fn default_ice_mid() -> String {
    "#408EE0FF".to_string()
}
pub(crate) fn default_ice_light() -> String {
    "#EBF6FFFF".to_string()
}
pub(crate) fn default_nebula_dark() -> String {
    "#140728FF".to_string()
}
pub(crate) fn default_nebula_mid() -> String {
    "#7622A8FF".to_string()
}
pub(crate) fn default_nebula_light() -> String {
    "#F18AEBFF".to_string()
}
pub(crate) fn default_liquid_first() -> String {
    "#184E77FF".to_string()
}
pub(crate) fn default_liquid_second() -> String {
    "#52B69AFF".to_string()
}
pub(crate) fn default_liquid_frequency() -> f32 {
    4.0
}
pub(crate) fn default_halftone_foreground() -> String {
    "#111827FF".to_string()
}
pub(crate) fn default_halftone_background() -> String {
    "#F3F4F6FF".to_string()
}
pub(crate) fn default_halftone_cell() -> u32 {
    6
}
pub(crate) fn default_letterpress_highlight() -> String {
    "#FFFFFF55".to_string()
}
pub(crate) fn default_letterpress_shadow() -> String {
    "#00000077".to_string()
}
pub(crate) fn default_chromatic_offset() -> f32 {
    0.025
}
pub(crate) fn default_chromatic_red() -> String {
    "#FF2A55CC".to_string()
}
pub(crate) fn default_chromatic_cyan() -> String {
    "#2AD5FFCC".to_string()
}
pub(crate) fn default_trail_distance() -> f32 {
    0.16
}
pub(crate) fn default_trail_copies() -> u32 {
    4
}
pub(crate) fn default_trail_angle() -> f32 {
    180.0
}
pub(crate) fn default_trail_color() -> String {
    "#FFB70366".to_string()
}
pub(crate) fn default_flicker_period() -> f32 {
    1.6
}
pub(crate) fn default_flicker_minimum() -> f32 {
    0.65
}
pub(crate) fn default_flicker_strength() -> f32 {
    0.32
}
pub(crate) fn default_wave_period() -> f32 {
    2.8
}
pub(crate) fn default_wave_amplitude() -> f32 {
    0.035
}
pub(crate) fn default_wave_wavelength() -> f32 {
    0.42
}
pub(crate) fn default_typewriter_period() -> f32 {
    4.0
}
pub(crate) fn default_dissolve_period() -> f32 {
    4.0
}
pub(crate) fn default_reveal_hold() -> f32 {
    0.35
}
pub(crate) fn default_dissolve_seed() -> u32 {
    0x504C_4151
}
pub(crate) fn default_arc_sweep() -> f32 {
    58.0
}
pub(crate) fn default_arc_radius_scale() -> f32 {
    1.0
}
pub(crate) fn default_texture_scale() -> f32 {
    1.0
}
pub(crate) fn default_blueprint_dark() -> String {
    "#082E5EFF".to_string()
}
pub(crate) fn default_blueprint_light() -> String {
    "#5FD8FFFF".to_string()
}
pub(crate) fn default_blueprint_grid() -> String {
    "#D7F6FFB8".to_string()
}
pub(crate) fn default_blueprint_cell() -> u32 {
    8
}
pub(crate) fn default_paper_light() -> String {
    "#FFF3D2FF".to_string()
}
pub(crate) fn default_paper_mid() -> String {
    "#D6B988FF".to_string()
}
pub(crate) fn default_paper_dark() -> String {
    "#7B5B38FF".to_string()
}
pub(crate) fn default_paper_seed() -> u32 {
    0x5041_5045
}
pub(crate) fn default_laser_depth() -> f32 {
    0.72
}
pub(crate) fn default_laser_warmth() -> f32 {
    0.65
}
pub(crate) fn default_laser_edge_width() -> u32 {
    2
}
pub(crate) fn default_surface_seed() -> u32 {
    0x4255_524E
}
pub(crate) fn default_emboss_depth() -> f32 {
    0.65
}
pub(crate) fn default_emboss_highlight() -> f32 {
    0.72
}
pub(crate) fn default_emboss_shadow() -> f32 {
    0.68
}
pub(crate) fn default_emboss_cast_shadow() -> u32 {
    2
}
pub(crate) fn default_scramble_period() -> f32 {
    3.8
}
pub(crate) fn default_split_flap_period() -> f32 {
    4.2
}
pub(crate) fn default_character_steps() -> f32 {
    14.0
}
pub(crate) fn default_scramble_seed() -> u32 {
    0x5343_524D
}
pub(crate) fn default_confetti_period() -> f32 {
    4.4
}
pub(crate) fn default_confetti_pieces() -> u32 {
    720
}
pub(crate) fn default_confetti_spread() -> f32 {
    0.48
}
pub(crate) fn default_confetti_seed() -> u32 {
    0x434F_4E46
}
pub(crate) fn default_glitch_period() -> f32 {
    2.6
}
pub(crate) fn default_glitch_ripple() -> f32 {
    0.018
}
pub(crate) fn default_glitch_slice() -> f32 {
    0.085
}
pub(crate) fn default_glitch_burst() -> f32 {
    0.20
}
pub(crate) fn default_glitch_seed() -> u32 {
    0x474C_4954
}
pub(crate) fn default_orbit_period() -> f32 {
    8.0
}
pub(crate) fn default_orbit_degrees() -> f32 {
    360.0
}
