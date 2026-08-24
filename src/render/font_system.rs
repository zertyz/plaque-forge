//! Hermetic font-system construction for deterministic typography.
//!
//! cosmic-text's convenience constructors load every font installed on the
//! workstation before adding the explicitly requested sources, so family
//! discovery and glyph fallback would otherwise depend on whichever fonts
//! happen to be installed. Deterministic rendering instead treats the requested
//! font file as the only glyph source: a glyph it cannot supply is an error,
//! never a silent environment-dependent substitution.

use std::path::Path;

use cosmic_text::{Fallback, FontSystem, fontdb};
use unicode_script::Script;

/// Locale fixed for shaping so results never depend on workstation environment.
const HERMETIC_LOCALE: &str = "en-US";

/// Build a [`FontSystem`] whose only glyph source is the given font file.
pub(crate) fn hermetic_font_system(font_path: &Path) -> FontSystem {
    let mut db = fontdb::Database::new();
    db.load_font_source(fontdb::Source::File(font_path.to_path_buf()));
    FontSystem::new_with_locale_and_db_and_fallback(HERMETIC_LOCALE.to_string(), db, NoFallback)
}

/// A [`Fallback`] policy that never names a substitute family.
struct NoFallback;

impl Fallback for NoFallback {
    fn common_fallback(&self) -> &[&'static str] {
        &[]
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        &[]
    }

    fn script_fallback(&self, _script: Script, _locale: &str) -> &[&'static str] {
        &[]
    }
}

/// The repository-pinned reference font for deterministic tests and
/// homologation. Scripts such as `check_homologated_assets.sh` pass the same
/// file; keep the two references in sync.
#[cfg(test)]
pub(crate) fn pinned_test_font() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fonts")
        .join("NotoSerif-Regular.ttf")
}

#[cfg(test)]
mod tests {
    use super::{HERMETIC_LOCALE, hermetic_font_system, pinned_test_font};

    #[test]
    fn font_system_contains_only_the_requested_face_with_fixed_locale() {
        let font_system = hermetic_font_system(&pinned_test_font());

        assert_eq!(
            font_system.db().len(),
            1,
            "font system must not load any workstation-installed fonts"
        );
        assert_eq!(font_system.locale(), HERMETIC_LOCALE);
    }
}
