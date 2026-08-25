//! Shared test infrastructure used by integration test files.
#![allow(dead_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use plaque_forge::media::fonts::FamilyIndex;

pub mod synthetic;

/// Root of the repository, resolved from `CARGO_MANIFEST_DIR`.
pub fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Fresh scratch directory for fixture trees. Contents are intentionally left
/// behind for post-mortem inspection of failed assertions.
pub fn temp_root(name: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is before UNIX_EPOCH")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("plaque-forge-{name}-{nanos}"));
    std::fs::create_dir_all(&dir).expect("failed to create scratch directory");
    dir
}

/// Deterministic stand-in for workstation font discovery, shared by every
/// media test so typeface results never depend on installed fonts.
pub struct FakeFamilies {
    matches: BTreeMap<String, String>,
    installed: BTreeSet<String>,
}

impl FakeFamilies {
    pub fn new(installed: &[&str]) -> Self {
        Self {
            matches: BTreeMap::new(),
            installed: installed.iter().map(|name| name.to_string()).collect(),
        }
    }

    /// Make `pattern` resolve to `family` exactly.
    pub fn resolving(mut self, pattern: &str, family: &str) -> Self {
        self.matches
            .insert(pattern.to_lowercase(), family.to_string());
        self
    }
}

impl FamilyIndex for FakeFamilies {
    fn face_file_and_family(&self, _family: &str) -> Option<(std::path::PathBuf, String)> {
        None
    }

    fn match_pattern(&self, pattern: &str) -> Option<String> {
        self.matches.get(&pattern.to_lowercase()).cloned()
    }

    fn families_excluding(&self, exclude: &BTreeSet<String>) -> Vec<String> {
        self.installed
            .iter()
            .filter(|family| !exclude.contains(&family.to_lowercase()))
            .cloned()
            .collect()
    }
}
