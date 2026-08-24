//! System font discovery shared by every media backend.
//!
//! Installed faces are read through fontdb, which is already in the dependency
//! graph for typography; no additional crates are pulled in.

use std::collections::BTreeSet;

use cosmic_text::fontdb::{self, Style};

/// Case-insensitive view over workstation-installed typefaces.
///
/// A trait so catalog construction stays deterministic under test without
/// touching whatever happens to be installed on the host.
pub trait FamilyIndex {
    /// Resolve a fontconfig-style pattern to its canonical family name.
    fn match_pattern(&self, pattern: &str) -> Option<String>;

    /// Distinct family names sorted alphabetically, minus the excluded labels
    /// (compared case-insensitively).
    fn families_excluding(&self, exclude_lowercase: &BTreeSet<String>) -> Vec<String>;
}

/// fontdb-backed index over the fonts installed on this workstation.
pub struct SystemFonts {
    db: fontdb::Database,
}

impl SystemFonts {
    pub fn load() -> Self {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        Self { db }
    }

    fn candidate_families(&self, pattern: &str) -> Vec<String> {
        let mut candidates: Vec<(u8, u16, String, String)> = self
            .db
            .faces()
            .filter(|face| {
                face.families
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case(pattern))
            })
            .map(|face| {
                let style_rank = match face.style {
                    Style::Normal => 0,
                    Style::Oblique => 1,
                    Style::Italic => 2,
                };
                let weight_distance = face.weight.0.abs_diff(400);
                let canonical = face
                    .families
                    .first()
                    .map(|(name, _)| name.clone())
                    .unwrap_or_else(|| pattern.to_string());
                (
                    style_rank,
                    weight_distance,
                    face.post_script_name.clone(),
                    canonical,
                )
            })
            .collect();
        candidates.sort();
        candidates
            .into_iter()
            .map(|(_, _, _, family)| family)
            .collect()
    }
}

impl FamilyIndex for SystemFonts {
    fn match_pattern(&self, pattern: &str) -> Option<String> {
        self.candidate_families(pattern).into_iter().next()
    }

    fn families_excluding(&self, exclude_lowercase: &BTreeSet<String>) -> Vec<String> {
        let mut families = BTreeSet::new();
        for face in self.db.faces() {
            for (name, _) in &face.families {
                if !exclude_lowercase.contains(&name.to_lowercase()) {
                    families.insert(name.clone());
                }
            }
        }
        families.into_iter().collect()
    }
}

/// Merge curated labels (in curated-file order) with remaining system families.
pub(crate) fn compose_font_list(
    curated_labels: &[String],
    index: &dyn FamilyIndex,
) -> Vec<super::contract::FontListing> {
    use super::contract::FontListing;

    let mut listings: Vec<_> = curated_labels
        .iter()
        .map(|label| FontListing {
            label: label.clone(),
            curated: true,
        })
        .collect();
    let exclude: BTreeSet<_> = curated_labels.iter().map(|l| l.to_lowercase()).collect();
    for family in index.families_excluding(&exclude) {
        listings.push(FontListing {
            label: family,
            curated: false,
        });
    }
    listings
}
