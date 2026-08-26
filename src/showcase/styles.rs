//! Parametric style draft for the showcase full-widget composer.

use anyhow::Result;

use crate::render::effects::{Style, presets::Style as StyleType};

/// Draft that can be mutated via widgets and then built into a `Style` without going through a file.
#[derive(Clone, Debug)]
pub struct StyleDraft {
    pub font_weight: u16,
    pub fill_kind: FillKind,
    pub layouts: Vec<LayoutDraft>,
    pub underlays: Vec<UnderlayDraft>,
    pub overlays: Vec<OverlayDraft>,
    pub surface_effects: Vec<SurfaceEffectDraft>,
    pub animations: Vec<AnimationDraft>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FillKind {
    Flat(String),
    LinearGradient {
        top: String,
        bottom: String,
    },
    Gold {
        dark: String,
        mid: String,
        light: String,
        highlight: String,
    },
    Chrome {
        dark: String,
        mid: String,
        light: String,
    },
    Holographic,
    Fire {
        dark: String,
        mid: String,
        light: String,
    },
    Ice {
        dark: String,
        mid: String,
        light: String,
    },
    Nebula {
        dark: String,
        mid: String,
        light: String,
    },
    Liquid {
        first: String,
        second: String,
        frequency: f32,
    },
    Halftone {
        foreground: String,
        background: String,
        cell: u32,
    },
    Blueprint {
        dark: String,
        light: String,
        grid: String,
        cell: u32,
    },
    Paper {
        light: String,
        mid: String,
        dark: String,
        seed: u32,
    },
}

#[derive(Clone, Debug)]
pub struct LayoutDraft {
    pub sweep_degrees: f32,
    pub radius_scale: f32,
}

#[derive(Clone, Debug)]
pub enum UnderlayDraft {
    Stroke {
        width: f32,
        color: String,
    },
    Glow {
        radius: u32,
        color: String,
    },
    Shadow {
        offset_x: f32,
        offset_y: f32,
        blur: u32,
        color: String,
    },
    Extrude {
        depth: f32,
        angle: f32,
        color: String,
    },
    ChromaticSplit {
        offset: f32,
        red: String,
        cyan: String,
    },
    Trail {
        distance: f32,
        copies: u32,
        angle: f32,
        color: String,
    },
}

#[derive(Clone, Debug)]
pub enum OverlayDraft {
    Bevel {
        width: f32,
        highlight: String,
        shadow: String,
    },
    Letterpress {
        width: f32,
        highlight: String,
        shadow: String,
    },
}

#[derive(Clone, Debug)]
pub enum SurfaceEffectDraft {
    LaserBurn {
        depth: f32,
        warmth: f32,
        edge: u32,
        seed: u32,
    },
    Emboss {
        depth: f32,
        highlight: f32,
        shadow: f32,
        light_angle: Option<f32>,
        cast: u32,
    },
}

#[derive(Clone, Debug)]
pub enum AnimationDraft {
    Pulse {
        period: f32,
        min: f32,
        max: f32,
        phase: f32,
    },
    Shine {
        period: f32,
        width: f32,
        angle: f32,
        color: String,
    },
    Flicker {
        period: f32,
        min: f32,
        strength: f32,
        phase: f32,
    },
    Wave {
        period: f32,
        amp: f32,
        wave: f32,
        phase: f32,
    },
    Typewriter {
        period: f32,
        hold: f32,
    },
    Dissolve {
        period: f32,
        hold: f32,
        seed: u32,
    },
    Scramble {
        period: f32,
        hold: f32,
        steps: f32,
        seed: u32,
    },
    SplitFlap {
        period: f32,
        hold: f32,
        steps: f32,
    },
    Confetti {
        period: f32,
        hold: f32,
        pieces: u32,
        spread: f32,
        seed: u32,
    },
    Glitch {
        period: f32,
        ripple: f32,
        slice: f32,
        burst: f32,
        seed: u32,
    },
    Orbit {
        period: f32,
        degrees: f32,
        phase: f32,
    },
}

impl Default for StyleDraft {
    fn default() -> Self {
        Self {
            font_weight: 600,
            fill_kind: FillKind::Flat("#EBFFFFFF".to_string()),
            layouts: Vec::new(),
            underlays: Vec::new(),
            overlays: Vec::new(),
            surface_effects: Vec::new(),
            animations: Vec::new(),
        }
    }
}

impl StyleDraft {
    pub fn from_style_file(path: &std::path::Path) -> Result<Self> {
        // Load style to validate, then parse TOML again for draft fields.
        let _ = StyleType::from_file(path)?;
        let source = std::fs::read_to_string(path)?;
        let parsed: toml::Value = toml::from_str(&source)?;
        // Simplistic default – full fidelity parsing would mirror presets.rs but draft keeps it editable widget-wise.
        // For now, start from file's weight if present.
        let weight = parsed
            .get("typography")
            .and_then(|v| v.get("weight"))
            .and_then(|v| v.as_integer())
            .map(|w| w as u16)
            .unwrap_or(600);
        let draft = Self {
            font_weight: weight,
            ..Default::default()
        };
        // Keep fill as flat for simplicity; detailed material parsing is done via to_toml + Style::from_file roundtrip when building.
        Ok(draft)
    }

    pub fn to_toml_string(&self) -> Result<String> {
        let mut out = String::new();
        out.push_str("version = 5\n\n");
        out.push_str(&format!("[typography]\nweight = {}\n\n", self.font_weight));
        match &self.fill_kind {
            FillKind::Flat(c) => out.push_str(&format!("fill = {c:?}\n\n")),
            FillKind::LinearGradient { top, bottom } => {
                out.push_str(&format!(
                    "[material]\ntype = \"linear-gradient\"\ntop = {top:?}\nbottom = {bottom:?}\n\n"
                ));
            }
            FillKind::Gold {
                dark,
                mid,
                light,
                highlight,
            } => {
                out.push_str(&format!(
                    "[material]\ntype = \"gold\"\ndark = {dark:?}\nmid = {mid:?}\nlight = {light:?}\nhighlight = {highlight:?}\n\n"
                ));
            }
            FillKind::Chrome { dark, mid, light } => {
                out.push_str(&format!(
                    "[material]\ntype = \"chrome\"\ndark = {dark:?}\nmid = {mid:?}\nlight = {light:?}\n\n"
                ));
            }
            FillKind::Holographic => out.push_str("[material]\ntype = \"holographic\"\n\n"),
            FillKind::Fire { dark, mid, light } => {
                out.push_str(&format!(
                    "[material]\ntype = \"fire\"\ndark = {dark:?}\nmid = {mid:?}\nlight = {light:?}\n\n"
                ));
            }
            FillKind::Ice { dark, mid, light } => {
                out.push_str(&format!(
                    "[material]\ntype = \"ice\"\ndark = {dark:?}\nmid = {mid:?}\nlight = {light:?}\n\n"
                ));
            }
            FillKind::Nebula { dark, mid, light } => {
                out.push_str(&format!(
                    "[material]\ntype = \"nebula\"\ndark = {dark:?}\nmid = {mid:?}\nlight = {light:?}\n\n"
                ));
            }
            FillKind::Liquid {
                first,
                second,
                frequency,
            } => {
                out.push_str(&format!(
                    "[material]\ntype = \"liquid\"\nfirst = {first:?}\nsecond = {second:?}\nfrequency = {frequency}\n\n"
                ));
            }
            FillKind::Halftone {
                foreground,
                background,
                cell,
            } => {
                out.push_str(&format!(
                    "[material]\ntype = \"halftone\"\nforeground = {foreground:?}\nbackground = {background:?}\ncell = {cell}\n\n"
                ));
            }
            FillKind::Blueprint {
                dark,
                light,
                grid,
                cell,
            } => {
                out.push_str(&format!(
                    "[material]\ntype = \"blueprint\"\ndark = {dark:?}\nlight = {light:?}\ngrid = {grid:?}\ncell = {cell}\n\n"
                ));
            }
            FillKind::Paper {
                light,
                mid,
                dark,
                seed,
            } => {
                out.push_str(&format!(
                    "[material]\ntype = \"paper\"\nlight = {light:?}\nmid = {mid:?}\ndark = {dark:?}\nseed = {seed}\n\n"
                ));
            }
        }
        for l in &self.layouts {
            out.push_str(&format!(
                "[[layouts]]\ntype = \"arc\"\nsweep_degrees = {}\nradius_scale = {}\n\n",
                l.sweep_degrees, l.radius_scale
            ));
        }
        for e in &self.underlays {
            match e {
                UnderlayDraft::Stroke { width, color } => {
                    out.push_str(&format!(
                        "[[effects]]\ntype = \"stroke\"\nwidth = {width}\ncolor = {color:?}\n\n"
                    ));
                }
                UnderlayDraft::Glow { radius, color } => {
                    out.push_str(&format!(
                        "[[effects]]\ntype = \"glow\"\nradius = {radius}\ncolor = {color:?}\n\n"
                    ));
                }
                UnderlayDraft::Shadow {
                    offset_x,
                    offset_y,
                    blur,
                    color,
                } => {
                    out.push_str(&format!("[[effects]]\ntype = \"shadow\"\noffset_x = {offset_x}\noffset_y = {offset_y}\nblur_radius = {blur}\ncolor = {color:?}\n\n"));
                }
                UnderlayDraft::Extrude {
                    depth,
                    angle,
                    color,
                } => {
                    out.push_str(&format!("[[effects]]\ntype = \"extrude\"\ndepth = {depth}\nangle_degrees = {angle}\ncolor = {color:?}\n\n"));
                }
                UnderlayDraft::ChromaticSplit { offset, red, cyan } => {
                    out.push_str(&format!("[[effects]]\ntype = \"chromatic-split\"\noffset = {offset}\nred = {red:?}\ncyan = {cyan:?}\n\n"));
                }
                UnderlayDraft::Trail {
                    distance,
                    copies,
                    angle,
                    color,
                } => {
                    out.push_str(&format!("[[effects]]\ntype = \"trail\"\ndistance = {distance}\ncopies = {copies}\nangle_degrees = {angle}\ncolor = {color:?}\n\n"));
                }
            }
        }
        for e in &self.overlays {
            match e {
                OverlayDraft::Bevel {
                    width,
                    highlight,
                    shadow,
                } => {
                    out.push_str(&format!("[[effects]]\ntype = \"bevel\"\nwidth = {width}\nhighlight = {highlight:?}\nshadow = {shadow:?}\n\n"));
                }
                OverlayDraft::Letterpress {
                    width,
                    highlight,
                    shadow,
                } => {
                    out.push_str(&format!("[[effects]]\ntype = \"letterpress\"\nwidth = {width}\nhighlight = {highlight:?}\nshadow = {shadow:?}\n\n"));
                }
            }
        }
        for e in &self.surface_effects {
            match e {
                SurfaceEffectDraft::LaserBurn {
                    depth,
                    warmth,
                    edge,
                    seed,
                } => {
                    out.push_str(&format!("[[surface_effects]]\ntype = \"laser-burn\"\ndepth = {depth}\nwarmth = {warmth}\nedge_width = {edge}\nseed = {seed}\n\n"));
                }
                SurfaceEffectDraft::Emboss {
                    depth,
                    highlight,
                    shadow,
                    light_angle,
                    cast,
                } => {
                    if let Some(a) = light_angle {
                        out.push_str(&format!("[[surface_effects]]\ntype = \"emboss\"\ndepth = {depth}\nhighlight_strength = {highlight}\nshadow_strength = {shadow}\nlight_angle_degrees = {a}\ncast_shadow = {cast}\n\n"));
                    } else {
                        out.push_str(&format!("[[surface_effects]]\ntype = \"emboss\"\ndepth = {depth}\nhighlight_strength = {highlight}\nshadow_strength = {shadow}\ncast_shadow = {cast}\n\n"));
                    }
                }
            }
        }
        for a in &self.animations {
            match a {
                AnimationDraft::Pulse {
                    period,
                    min,
                    max,
                    phase,
                } => {
                    out.push_str(&format!("[[animations]]\ntype = \"pulse\"\nperiod_seconds = {period}\nminimum_opacity = {min}\nmaximum_opacity = {max}\nphase = {phase}\n\n"));
                }
                AnimationDraft::Shine {
                    period,
                    width,
                    angle,
                    color,
                } => {
                    out.push_str(&format!("[[animations]]\ntype = \"shine\"\nperiod_seconds = {period}\nwidth = {width}\nangle_degrees = {angle}\ncolor = {color:?}\n\n"));
                }
                AnimationDraft::Flicker {
                    period,
                    min,
                    strength,
                    phase,
                } => {
                    out.push_str(&format!("[[animations]]\ntype = \"flicker\"\nperiod_seconds = {period}\nminimum_opacity = {min}\nstrength = {strength}\nphase = {phase}\n\n"));
                }
                AnimationDraft::Wave {
                    period,
                    amp,
                    wave,
                    phase,
                } => {
                    out.push_str(&format!("[[animations]]\ntype = \"wave\"\nperiod_seconds = {period}\namplitude = {amp}\nwavelength = {wave}\nphase = {phase}\n\n"));
                }
                AnimationDraft::Typewriter { period, hold } => {
                    out.push_str(&format!("[[animations]]\ntype = \"typewriter\"\nperiod_seconds = {period}\nhold_fraction = {hold}\n\n"));
                }
                AnimationDraft::Dissolve { period, hold, seed } => {
                    out.push_str(&format!("[[animations]]\ntype = \"dissolve\"\nperiod_seconds = {period}\nhold_fraction = {hold}\nseed = {seed}\n\n"));
                }
                AnimationDraft::Scramble {
                    period,
                    hold,
                    steps,
                    seed,
                } => {
                    out.push_str(&format!("[[animations]]\ntype = \"scramble\"\nperiod_seconds = {period}\nhold_fraction = {hold}\nsteps_per_second = {steps}\nseed = {seed}\n\n"));
                }
                AnimationDraft::SplitFlap {
                    period,
                    hold,
                    steps,
                } => {
                    out.push_str(&format!("[[animations]]\ntype = \"split-flap\"\nperiod_seconds = {period}\nhold_fraction = {hold}\nsteps_per_second = {steps}\n\n"));
                }
                AnimationDraft::Confetti {
                    period,
                    hold,
                    pieces,
                    spread,
                    seed,
                } => {
                    out.push_str(&format!("[[animations]]\ntype = \"confetti-converge\"\nperiod_seconds = {period}\nhold_fraction = {hold}\npieces = {pieces}\nspread = {spread}\nseed = {seed}\n\n"));
                }
                AnimationDraft::Glitch {
                    period,
                    ripple,
                    slice,
                    burst,
                    seed,
                } => {
                    out.push_str(&format!("[[animations]]\ntype = \"glitch\"\nperiod_seconds = {period}\nripple = {ripple}\nslice = {slice}\nburst_fraction = {burst}\nseed = {seed}\n\n"));
                }
                AnimationDraft::Orbit {
                    period,
                    degrees,
                    phase,
                } => {
                    out.push_str(&format!("[[animations]]\ntype = \"orbit\"\nperiod_seconds = {period}\ndegrees_per_cycle = {degrees}\nphase = {phase}\n\n"));
                }
            }
        }
        Ok(out)
    }

    pub fn build_style(&self) -> Result<Style> {
        // Write to temp file then load via existing parser – reuses validation.
        let toml = self.to_toml_string()?;
        let path =
            std::env::temp_dir().join(format!("plaque-forge-showcase-{}.toml", std::process::id()));
        std::fs::write(&path, &toml)?;
        let s = Style::from_file(&path);
        let _ = std::fs::remove_file(&path);
        s
    }

    pub fn save_to_file(&self, dest: &std::path::Path) -> Result<()> {
        let toml = self.to_toml_string()?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(dest, toml)?;
        // validate after write
        let _ = Style::from_file(dest)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_builds_style() {
        let d = StyleDraft::default();
        let style = d.build_style().unwrap();
        assert_eq!(style.font_weight(), 600);
    }

    #[test]
    fn toml_roundtrip_with_effects() {
        let mut d = StyleDraft {
            fill_kind: FillKind::Gold {
                dark: "#111111FF".into(),
                mid: "#222222FF".into(),
                light: "#333333FF".into(),
                highlight: "#444444FF".into(),
            },
            ..Default::default()
        };
        d.underlays.push(UnderlayDraft::Stroke {
            width: 0.05,
            color: "#FF0000FF".into(),
        });
        d.animations.push(AnimationDraft::Pulse {
            period: 2.0,
            min: 0.5,
            max: 1.0,
            phase: 0.0,
        });
        let s = d.build_style().unwrap();
        assert!(s.has_frame_variation());
    }

    #[test]
    fn save_and_load() {
        let d = StyleDraft::default();
        let dir = std::env::temp_dir().join(format!("pf-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.toml");
        d.save_to_file(&path).unwrap();
        assert!(path.is_file());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }
}
