//! Text paint, mask effects, materials, and frame-varying presentation.
//!
//! Layout and glyph geometry stay in `typography`. This module consumes already-shaped
//! coverage, which lets static material/effect work be cached while animations such as a
//! moving shine reevaluate only presentation state.

use anyhow::{Result, bail};

use crate::{color::Rgba, surface::Surface};

mod advanced;
pub mod filters;
pub mod presets;
pub mod shaders;
pub mod types;

pub use filters::*;
pub use presets::*;
pub use shaders::*;
pub use types::*;

impl Style {
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
            FillStyle::Blueprint {
                dark,
                light,
                grid,
                cell,
            } => format!(
                "blueprint(dark={},light={},grid={},cell={cell})",
                format_color(dark),
                format_color(light),
                format_color(grid)
            ),
            FillStyle::Paper {
                light,
                mid,
                dark,
                seed,
            } => format!(
                "paper(light={},mid={},dark={},seed={seed})",
                format_color(light),
                format_color(mid),
                format_color(dark)
            ),
        }];
        parts.insert(0, format!("typography(weight={})", self.font_weight()));
        if let Some(texture) = &self.texture {
            parts.push(format!(
                "image-texture(path={},sha256={},tile={},scale={:.3},offset=({:.3},{:.3}))",
                texture.path.display(),
                texture.sha256,
                texture.tile,
                texture.scale,
                texture.offset_x,
                texture.offset_y
            ));
        }
        for layout in &self.layouts {
            parts.push(match *layout {
                LayoutEffect::Arc {
                    sweep_degrees,
                    radius_scale,
                } => format!("arc(sweep={sweep_degrees:.2},radius-scale={radius_scale:.3})"),
            });
        }
        for effect in &self.underlays {
            parts.push(match *effect {
                MaskEffect::Stroke { width_ratio, color } => format!("stroke(width={width_ratio:.5},color={})", format_color(color)),
                MaskEffect::Glow { radius, color } => format!("glow(radius={radius},color={})", format_color(color)),
                MaskEffect::Shadow { offset_x_ratio, offset_y_ratio, blur_radius, color } => format!("shadow(x={offset_x_ratio:.5},y={offset_y_ratio:.5},blur={blur_radius},color={})", format_color(color)),
                MaskEffect::Extrude { depth_ratio, angle_degrees, color } => format!("extrude(depth={depth_ratio:.5},angle={angle_degrees:.2},color={})", format_color(color)),
                MaskEffect::ChromaticSplit { offset_ratio, red, cyan } => format!("chromatic-split(offset={offset_ratio:.5},red={},cyan={})", format_color(red), format_color(cyan)),
                MaskEffect::Trail { distance_ratio, copies, angle_degrees, color } => format!("trail(distance={distance_ratio:.5},copies={copies},angle={angle_degrees:.2},color={})", format_color(color)),
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
        for effect in &self.surface_effects {
            parts.push(match *effect {
                SurfaceEffect::LaserBurn { depth, warmth, edge_width, seed } => format!("laser-burn(depth={depth:.3},warmth={warmth:.3},edge={edge_width},seed={seed})"),
                SurfaceEffect::Emboss { depth, highlight_strength, shadow_strength, light_angle_degrees, cast_shadow } => format!("emboss(depth={depth:.3},highlight={highlight_strength:.3},shadow={shadow_strength:.3},light={},cast={cast_shadow})", light_angle_degrees.map(|angle| format!("{angle:.2}")).unwrap_or_else(|| "auto".to_string())),
            });
        }
        for animation in &self.animations {
            parts.push(match *animation {
                AnimationEffect::Pulse { period_seconds, minimum_opacity, maximum_opacity, phase } => format!("pulse(period={period_seconds:.3},min={minimum_opacity:.3},max={maximum_opacity:.3},phase={phase:.3})"),
                AnimationEffect::Shine { period_seconds, width_ratio, angle_degrees, color } => format!("shine(period={period_seconds:.3},width={width_ratio:.3},angle={angle_degrees:.2},color={})", format_color(color)),
                AnimationEffect::Flicker { period_seconds, minimum_opacity, strength, phase } => format!("flicker(period={period_seconds:.3},min={minimum_opacity:.3},strength={strength:.3},phase={phase:.3})"),
                AnimationEffect::Wave { period_seconds, amplitude_ratio, wavelength_ratio, phase } => format!("wave(period={period_seconds:.3},amplitude={amplitude_ratio:.3},wavelength={wavelength_ratio:.3},phase={phase:.3})"),
                AnimationEffect::Typewriter { period_seconds, hold_fraction } => format!("typewriter(period={period_seconds:.3},hold={hold_fraction:.3})"),
                AnimationEffect::Dissolve { period_seconds, hold_fraction, seed } => format!("dissolve(period={period_seconds:.3},hold={hold_fraction:.3},seed={seed})"),
                AnimationEffect::Scramble { period_seconds, hold_fraction, steps_per_second, seed } => format!("scramble(period={period_seconds:.3},hold={hold_fraction:.3},steps={steps_per_second:.2},seed={seed})"),
                AnimationEffect::SplitFlap { period_seconds, hold_fraction, steps_per_second } => format!("split-flap(period={period_seconds:.3},hold={hold_fraction:.3},steps={steps_per_second:.2})"),
                AnimationEffect::ConfettiConverge { period_seconds, hold_fraction, pieces, spread_ratio, seed } => format!("confetti-converge(period={period_seconds:.3},hold={hold_fraction:.3},pieces={pieces},spread={spread_ratio:.3},seed={seed})"),
                AnimationEffect::Glitch { period_seconds, ripple_ratio, slice_ratio, burst_fraction, seed } => format!("glitch(period={period_seconds:.3},ripple={ripple_ratio:.3},slice={slice_ratio:.3},burst={burst_fraction:.3},seed={seed})"),
                AnimationEffect::Orbit { period_seconds, degrees_per_cycle, phase } => format!("orbit(period={period_seconds:.3},degrees={degrees_per_cycle:.2},phase={phase:.3})"),
            });
        }
        parts.join(";")
    }

    pub fn layout_transform(&self, base: &Surface) -> Result<Surface> {
        let mut current = base.clone();
        for effect in &self.layouts {
            current = match *effect {
                LayoutEffect::Arc {
                    sweep_degrees,
                    radius_scale,
                } => advanced::arc_warp(&current, sweep_degrees, radius_scale),
            };
        }
        Ok(current)
    }

    pub fn dynamic_text(&self, target: &str, time_seconds: f64) -> Option<String> {
        let animation = self.animations.iter().find(|animation| {
            matches!(
                animation,
                AnimationEffect::Scramble { .. } | AnimationEffect::SplitFlap { .. }
            )
        })?;
        Some(match *animation {
            AnimationEffect::Scramble {
                period_seconds,
                hold_fraction,
                steps_per_second,
                seed,
            } => scramble_text(
                target,
                time_seconds,
                period_seconds,
                hold_fraction,
                steps_per_second,
                seed,
            ),
            AnimationEffect::SplitFlap {
                period_seconds,
                hold_fraction,
                steps_per_second,
            } => split_flap_text(
                target,
                time_seconds,
                period_seconds,
                hold_fraction,
                steps_per_second,
            ),
            _ => unreachable!(),
        })
    }

    pub fn has_surface_effects(&self) -> bool {
        !self.surface_effects.is_empty()
    }

    pub fn surface_overlay(&self, plaque: &Surface, glyph_mask: &[u8]) -> Result<Option<Surface>> {
        if self.surface_effects.is_empty() {
            return Ok(None);
        }
        let mut combined = Surface::new(plaque.width(), plaque.height());
        for effect in &self.surface_effects {
            let layer = match *effect {
                SurfaceEffect::LaserBurn {
                    depth,
                    warmth,
                    edge_width,
                    seed,
                } => advanced::laser_burn_overlay(
                    plaque, glyph_mask, depth, warmth, edge_width, seed,
                )?,
                SurfaceEffect::Emboss {
                    depth,
                    highlight_strength,
                    shadow_strength,
                    light_angle_degrees,
                    cast_shadow,
                } => advanced::emboss_overlay(
                    plaque,
                    glyph_mask,
                    depth,
                    highlight_strength,
                    shadow_strength,
                    light_angle_degrees,
                    cast_shadow,
                )?,
            };
            combined.blend_surface(&layer, 0, 0, 1.0);
        }
        Ok(Some(combined))
    }

    pub fn frame_transform_mask(
        &self,
        mask: &[u8],
        width: u32,
        height: u32,
        time_seconds: f64,
    ) -> Result<Vec<u8>> {
        let white = Surface::from_alpha_mask(width, height, mask, Rgba::new(255, 255, 255, 255))?;
        Ok(self.frame_transform(&white, time_seconds)?.alpha_mask())
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
                MaskEffect::Stroke { color, .. } => {
                    let radius = effect.stroke_radius_px(font_size);
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
                MaskEffect::Extrude { color, .. } => {
                    if color.a == 0 {
                        continue;
                    }
                    let Some(geometry) = effect.extrude_geometry(font_size) else {
                        continue;
                    };
                    let layer = Surface::from_alpha_mask(width, height, &alpha, color)?;
                    for step in (1..=geometry.depth).rev() {
                        combined.blend_surface(
                            &layer,
                            (geometry.dx * step as f32).round() as i32,
                            (geometry.dy * step as f32).round() as i32,
                            1.0,
                        );
                    }
                }
                MaskEffect::ChromaticSplit { red, cyan, .. } => {
                    let offset = effect.chromatic_offset_px(font_size);
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
                MaskEffect::Trail { copies, color, .. } => {
                    if color.a == 0 {
                        continue;
                    }
                    let Some(geometry) = effect.trail_geometry(font_size) else {
                        continue;
                    };
                    let layer = Surface::from_alpha_mask(width, height, &alpha, color)?;
                    for copy in (1..=copies).rev() {
                        let t = copy as f32 / copies as f32;
                        combined.blend_surface(
                            &layer,
                            (geometry.dx * geometry.distance * t).round() as i32,
                            (geometry.dy * geometry.distance * t).round() as i32,
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
                } => apply_relief_overlay(
                    ReliefCanvas {
                        combined: &mut combined,
                        alpha: &alpha,
                        width,
                        height,
                    },
                    width_ratio,
                    font_size,
                    highlight,
                    shadow,
                    false,
                )?,
                OverlayEffect::Letterpress {
                    width_ratio,
                    highlight,
                    shadow,
                } => apply_relief_overlay(
                    ReliefCanvas {
                        combined: &mut combined,
                        alpha: &alpha,
                        width,
                        height,
                    },
                    width_ratio,
                    font_size,
                    highlight,
                    shadow,
                    true,
                )?,
            }
        }
        Ok(combined)
    }

    /// Coverage that must remain fully inside the writable region while fitting.
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
                MaskEffect::Stroke { .. } => {
                    let radius = effect.stroke_radius_px(font_size);
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
                MaskEffect::Extrude { .. } => {
                    if let Some(geometry) = effect.extrude_geometry(font_size) {
                        for step in 1..=geometry.depth {
                            alpha_over_shifted(
                                &mut envelope,
                                alpha,
                                width as usize,
                                height as usize,
                                (geometry.dx * step as f32).round() as i32,
                                (geometry.dy * step as f32).round() as i32,
                            );
                        }
                    }
                }
                MaskEffect::ChromaticSplit { .. } => {
                    let offset = effect.chromatic_offset_px(font_size);
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
                MaskEffect::Trail { .. } => {
                    if let Some(geometry) = effect.trail_geometry(font_size) {
                        alpha_over_shifted(
                            &mut envelope,
                            alpha,
                            width as usize,
                            height as usize,
                            (geometry.dx * geometry.distance).round() as i32,
                            (geometry.dy * geometry.distance).round() as i32,
                        );
                    }
                }
                MaskEffect::Glow { .. } | MaskEffect::Shadow { .. } => {}
            }
        }

        for animation in &self.animations {
            match *animation {
                AnimationEffect::Wave {
                    amplitude_ratio, ..
                } => {
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
                AnimationEffect::Glitch {
                    slice_ratio,
                    ripple_ratio,
                    ..
                } => {
                    let glyph_width = alpha_bounds(alpha, width as usize)
                        .map(|bounds| bounds.2 - bounds.0 + 1)
                        .unwrap_or(1) as f32;
                    let shift = (glyph_width * (slice_ratio + ripple_ratio))
                        .round()
                        .max(0.0) as i32;
                    if shift > 0 {
                        alpha_over_shifted(
                            &mut envelope,
                            alpha,
                            width as usize,
                            height as usize,
                            -shift,
                            0,
                        );
                        alpha_over_shifted(
                            &mut envelope,
                            alpha,
                            width as usize,
                            height as usize,
                            shift,
                            0,
                        );
                    }
                }
                _ => {}
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
                | AnimationEffect::Dissolve { .. }
                | AnimationEffect::Scramble { .. }
                | AnimationEffect::SplitFlap { .. }
                | AnimationEffect::ConfettiConverge { .. }
                | AnimationEffect::Glitch { .. }
                | AnimationEffect::Orbit { .. } => 1.0,
            };
            opacity * value
        })
    }

    /// Build only the frame-varying overlay.
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
    /// been composed.
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
                AnimationEffect::ConfettiConverge {
                    period_seconds,
                    hold_fraction,
                    pieces,
                    spread_ratio,
                    seed,
                } => advanced::confetti_converge(
                    &current,
                    reveal_progress(time_seconds, period_seconds, hold_fraction),
                    pieces,
                    spread_ratio,
                    seed,
                ),
                AnimationEffect::Glitch {
                    period_seconds,
                    ripple_ratio,
                    slice_ratio,
                    burst_fraction,
                    seed,
                } => advanced::glitch_surface(
                    &current,
                    time_seconds,
                    period_seconds,
                    ripple_ratio,
                    slice_ratio,
                    burst_fraction,
                    seed,
                ),
                AnimationEffect::Orbit {
                    period_seconds,
                    degrees_per_cycle,
                    phase,
                } => {
                    let progress = (time_seconds / period_seconds as f64 + phase as f64)
                        .rem_euclid(1.0) as f32;
                    advanced::rotate_surface(&current, degrees_per_cycle * progress)
                }
                AnimationEffect::Pulse { .. }
                | AnimationEffect::Shine { .. }
                | AnimationEffect::Flicker { .. }
                | AnimationEffect::Scramble { .. }
                | AnimationEffect::SplitFlap { .. } => current,
            };
        }
        Ok(current)
    }
}

/// Blend the two directional edge masks of a bevel-family overlay.
///
/// Raised (`bevel`) lights top/left edges; pressed (`letterpress`) swaps the
/// color roles so edges read as inset. Blend order is preserved because the
/// layers are translucent.
/// Canvas geometry shared by the bevel-family overlays.
struct ReliefCanvas<'a> {
    combined: &'a mut Surface,
    alpha: &'a [u8],
    width: u32,
    height: u32,
}

fn apply_relief_overlay(
    canvas: ReliefCanvas<'_>,
    width_ratio: f32,
    font_size: f32,
    highlight: Rgba,
    shadow: Rgba,
    letterpress: bool,
) -> Result<()> {
    let radius = (font_size * width_ratio).round().max(1.0) as i32;
    let (top_left, bottom_right) = directional_bevel_masks(
        canvas.alpha,
        canvas.width as usize,
        canvas.height as usize,
        radius,
    );
    let first_color = if letterpress { shadow } else { highlight };
    let second_color = if letterpress { highlight } else { shadow };
    for (mask, color) in [(&top_left, first_color), (&bottom_right, second_color)] {
        let layer = Surface::from_alpha_mask(canvas.width, canvas.height, mask, color)?;
        canvas.combined.blend_surface(&layer, 0, 0, 1.0);
    }
    Ok(())
}

impl Style {
    fn paint_fill(&self, base: &Surface, supersampling: u32) -> Result<Surface> {
        if let Some(texture) = &self.texture {
            return advanced::paint_texture(
                base,
                &texture.image,
                texture.tile,
                texture.scale,
                texture.offset_x,
                texture.offset_y,
            );
        }
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
            FillStyle::Blueprint {
                dark,
                light,
                grid,
                cell,
            } => advanced::paint_blueprint(
                base,
                dark,
                light,
                grid,
                cell.saturating_mul(supersampling.max(1)),
            ),
            FillStyle::Paper {
                light,
                mid,
                dark,
                seed,
            } => advanced::paint_paper(base, light, mid, dark, seed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DirectStyleOptions, Style, dilate_alpha_circular, directional_bevel_masks, scramble_text,
        split_flap_text,
    };
    use crate::surface::Surface;

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

    #[test]
    fn scramble_is_deterministic_and_settles_on_target() {
        let target = "SEE WHAT OTHERS CANNOT";
        let first = scramble_text(target, 0.25, 4.0, 0.30, 15.0, 1234);
        let repeated = scramble_text(target, 0.25, 4.0, 0.30, 15.0, 1234);
        assert_eq!(first, repeated);
        assert_ne!(first, target);
        assert_eq!(scramble_text(target, 3.6, 4.0, 0.30, 15.0, 1234), target);
    }

    #[test]
    fn split_flap_preserves_whitespace_and_settles_on_target() {
        let target = "OGRE ROBOT";
        let intermediate = split_flap_text(target, 0.4, 4.0, 0.30, 16.0);
        assert_eq!(intermediate.chars().nth(4), Some(' '));
        assert_eq!(split_flap_text(target, 3.6, 4.0, 0.30, 16.0), target);
    }

    fn synthetic_glyph_surface(width: u32, height: u32) -> Surface {
        let mut surface = Surface::new(width, height);
        let inset_x = width / 4;
        let inset_y = height / 4;
        for y in inset_y..(height - inset_y) {
            for x in inset_x..(width - inset_x) {
                surface.set_pixel(x, y, crate::color::Rgba::new(255, 255, 255, 200));
            }
        }
        surface
    }

    fn default_direct_options() -> DirectStyleOptions<'static> {
        DirectStyleOptions {
            font_weight: 600,
            text_color: "#FFFFFFFF",
            stroke_color: "#000000FF",
            glow_color: "#00000000",
            glow_radius: 0,
            stroke_width_ratio: 0.0,
            shadow_offset_x_ratio: 0.0,
            shadow_offset_y_ratio: 0.0,
            shadow_blur_radius: 0,
            shadow_color: "#00000000",
        }
    }

    #[test]
    fn flat_fill_produces_uniform_color_in_opaque_region() {
        let base = synthetic_glyph_surface(32, 32);
        let style = Style::direct(DirectStyleOptions {
            font_weight: 600,
            text_color: "#FF0000FF",
            ..default_direct_options()
        })
        .unwrap();
        let composed = style.compose(&base, 16.0, 1).unwrap();
        let center = composed.pixel(16, 16);
        assert!(center.a > 0, "center pixel should be visible");
        assert!(
            center.r > 200,
            "center pixel should be red, got r={}",
            center.r
        );
    }

    #[test]
    fn glow_extends_beyond_glyph_boundary() {
        let width = 64;
        let height = 48;
        let base = synthetic_glyph_surface(width, height);
        let base_bounds = base.alpha_bounds().expect("base has alpha");

        let style = Style::direct(DirectStyleOptions {
            glow_color: "#00FF00FF",
            glow_radius: 4,
            ..default_direct_options()
        })
        .unwrap();
        let composed = style.compose(&base, 16.0, 1).unwrap();
        let glow_bounds = composed.alpha_bounds().expect("composed has alpha");

        assert!(
            glow_bounds.0 < base_bounds.0
                || glow_bounds.1 < base_bounds.1
                || glow_bounds.2 > base_bounds.2
                || glow_bounds.3 > base_bounds.3,
            "glow should extend beyond glyph boundary: base {:?}, glow {:?}",
            base_bounds,
            glow_bounds
        );
    }

    #[test]
    fn stroke_widens_the_visible_footprint() {
        let width = 64;
        let height = 48;
        let base = synthetic_glyph_surface(width, height);
        let base_bounds = base.alpha_bounds().expect("base has alpha");

        let style = Style::direct(DirectStyleOptions {
            stroke_width_ratio: 0.15,
            ..default_direct_options()
        })
        .unwrap();
        let composed = style.compose(&base, 16.0, 1).unwrap();
        let stroke_bounds = composed.alpha_bounds().expect("composed has alpha");

        assert!(
            stroke_bounds.0 < base_bounds.0
                || stroke_bounds.1 < base_bounds.1
                || stroke_bounds.2 > base_bounds.2
                || stroke_bounds.3 > base_bounds.3,
            "stroke should widen the visible footprint: base {:?}, stroke {:?}",
            base_bounds,
            stroke_bounds
        );
    }

    #[test]
    fn shadow_shifts_the_alpha_centroid() {
        let width = 64;
        let height = 48;
        let base = synthetic_glyph_surface(width, height);

        let style_no_shadow = Style::direct(default_direct_options()).unwrap();
        let composed_no = style_no_shadow.compose(&base, 16.0, 1).unwrap();

        let style_shadow = Style::direct(DirectStyleOptions {
            shadow_offset_x_ratio: 0.2,
            shadow_offset_y_ratio: 0.2,
            shadow_blur_radius: 2,
            shadow_color: "#000000FF",
            ..default_direct_options()
        })
        .unwrap();
        let composed_shadow = style_shadow.compose(&base, 16.0, 1).unwrap();

        let bounds_no = composed_no.alpha_bounds().expect("no-shadow has alpha");
        let bounds_shadow = composed_shadow.alpha_bounds().expect("shadow has alpha");
        assert!(
            bounds_shadow.2 > bounds_no.2 || bounds_shadow.3 > bounds_no.3,
            "shadow should shift the footprint: no-shadow {:?}, shadow {:?}",
            bounds_no,
            bounds_shadow
        );
    }

    #[test]
    fn zero_radius_glow_is_a_no_op() {
        let base = synthetic_glyph_surface(32, 32);

        let style_no_glow = Style::direct(default_direct_options()).unwrap();
        let composed_no = style_no_glow.compose(&base, 16.0, 1).unwrap();

        let style_zero_glow = Style::direct(DirectStyleOptions {
            glow_color: "#00FF00FF",
            glow_radius: 0,
            ..default_direct_options()
        })
        .unwrap();
        let composed_zero = style_zero_glow.compose(&base, 16.0, 1).unwrap();

        assert_eq!(
            composed_no.alpha_bounds(),
            composed_zero.alpha_bounds(),
            "glow_radius=0 should not change the footprint"
        );
    }
}
