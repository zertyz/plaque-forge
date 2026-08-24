//! System font discovery and curated-label composition shared by every media
//! backend.
//!
//! Installed faces are read through fontdb, which is already in the dependency
//! graph for typography; no additional crates are pulled in.

use std::collections::BTreeSet;
use std::path::Path;

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

/// File stem of a listing path, shared by every media backend.
pub(crate) fn path_stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(path)
        .to_string()
}

/// One curated-font entry as seen by a catalog backend, shaped for shared
/// labeling so both backends compose display names through one rule.
pub(crate) struct CuratedListingItem<'a> {
    /// Pattern exactly as written in the curated list.
    pub pattern: &'a str,
    /// True when the entry pins a repository file below `fonts/`.
    pub repository_file: bool,
    /// Bundle-relative location of the embedded bytes (repository entries).
    pub bundle_path: &'a str,
    /// Family name recorded when the entry was embedded, if any.
    pub resolved_family: Option<&'a str>,
}

/// Display labels for curated entries, in curated-list order.
///
/// A pinned file lists under its file stem; a family entry lists under its
/// recorded build-time name when one exists, then under this machine's
/// resolution, then under the raw pattern. Cross-kind label collisions keep
/// their first occurrence so every listed entry stays unambiguous.
pub(crate) fn compose_curated_labels<'a>(
    items: impl IntoIterator<Item = CuratedListingItem<'a>>,
    families: &dyn FamilyIndex,
) -> Vec<String> {
    let mut labels = Vec::new();
    let mut seen = BTreeSet::new();
    for item in items {
        let label = if item.repository_file {
            path_stem(item.bundle_path)
        } else {
            item.resolved_family
                .map(str::to_string)
                .or_else(|| families.match_pattern(item.pattern))
                .unwrap_or_else(|| item.pattern.to_string())
        };
        if seen.insert(label.to_lowercase()) {
            labels.push(label);
        }
    }
    labels
}
