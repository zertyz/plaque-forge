//! Preview quality tiers.
//!
//! FAST keeps interactive playback smooth by clamping expensive parameters at
//! style-construction time; FINE is the exact pipeline the CLI renders with.
//! Clamps always stay inside the parser's validated ranges so both tiers
//! produce loadable styles.

use crate::render::effects::types::{AnimationFile, EffectFile, StyleFile};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Fast,
    Fine,
}

impl Tier {
    pub fn toggled(self) -> Self {
        match self {
            Self::Fast => Self::Fine,
            Self::Fine => Self::Fast,
        }
    }

    /// Supersampling used for the title bake.
    pub fn supersampling(self) -> u32 {
        match self {
            Self::Fast => 2,
            Self::Fine => 4,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Fast => "FAST",
            Self::Fine => "FINE",
        }
    }

    const MAX_BLUR: u32 = 8;

    /// Clamp a style file in place for smooth preview.
    #[allow(unused)]
    pub(crate) fn apply_to_style(self, file: &mut StyleFile) {
        if self == Tier::Fine {
            return;
        }
        for effect in &mut file.effects {
            match effect {
                EffectFile::Glow { radius, .. } => *radius = (*radius).min(Self::MAX_BLUR),
                EffectFile::Shadow { blur_radius, .. } => {
                    *blur_radius = (*blur_radius).min(Self::MAX_BLUR)
                }
                EffectFile::Trail { copies, .. } => *copies = ((*copies).max(1) / 2).clamp(1, 32),
                _ => {}
            }
        }
        for animation in &mut file.animations {
            if let AnimationFile::ConfettiConverge { pieces, .. } = animation {
                *pieces = ((*pieces).max(1) / 2).clamp(32, 10_000);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::effects::Style;
    use std::path::Path;

    fn parse(file: &StyleFile) -> String {
        let source = toml::to_string(file).unwrap();
        let parsed = Style::parse_str(&source, Path::new("."));
        match parsed {
            Ok(_) => String::new(),
            Err(error) => format!("{error:#}"),
        }
    }

    fn extreme() -> StyleFile {
        toml::from_str(
            "version = 5\n\
             [[effects]]\n\
             type = \"glow\"\n\
             radius = 64\n\
             color = \"#FFFFFFFF\"\n\
             [[effects]]\n\
             type = \"shadow\"\n\
             offset_x = 0.1\n\
             offset_y = 0.1\n\
             blur_radius = 64\n\
             color = \"#000000FF\"\n\
             [[effects]]\n\
             type = \"trail\"\n\
             distance = 0.5\n\
             copies = 16\n\
             angle_degrees = 30\n\
             color = \"#FFFFFF80\"\n\
             [[animations]]\n\
             type = \"confetti-converge\"\n\
             period_seconds = 3.0\n\
             hold_fraction = 0.5\n\
             pieces = 10000\n\
             spread = 1.0\n",
        )
        .unwrap()
    }

    #[test]
    fn fine_leaves_the_style_untouched() {
        let mut file = extreme();
        let before = toml::to_string(&file).unwrap();
        Tier::Fine.apply_to_style(&mut file);
        assert_eq!(before, toml::to_string(&file).unwrap());
        assert_eq!(Tier::Fine.supersampling(), 4);
    }

    #[test]
    fn fast_clamps_expensive_parameters_into_valid_ranges() {
        let mut file = extreme();
        Tier::Fast.apply_to_style(&mut file);
        assert_eq!(Tier::Fast.supersampling(), 2);
        assert!(
            matches!(&file.effects[0], EffectFile::Glow { radius: 8, .. }),
            "glow radius clamped"
        );
        assert!(
            matches!(&file.effects[1], EffectFile::Shadow { blur_radius: 8, .. }),
            "shadow blur clamped"
        );
        assert!(
            matches!(&file.effects[2], EffectFile::Trail { copies: 8, .. }),
            "trail copies halved"
        );
        assert!(
            matches!(
                &file.animations[0],
                AnimationFile::ConfettiConverge { pieces: 5000, .. }
            ),
            "confetti pieces halved but above the floor"
        );
        let error = parse(&file);
        assert!(error.is_empty(), "clamped style must still parse: {error}");
    }

    #[test]
    fn toggling_flips_between_tiers() {
        assert_eq!(Tier::Fast.toggled(), Tier::Fine);
        assert_eq!(Tier::Fine.toggled(), Tier::Fast);
    }
}
