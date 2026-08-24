//! Authoritative serde model of `assets/plaques/catalog.toml`.
//!
//! Production listing and the repository asset tests share this single
//! definition so catalog knowledge cannot drift between them.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// Catalog of text-free plaque PNGs available for placement into scenes.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaqueCatalog {
    pub schema_version: u32,
    pub plaques: Vec<PlaqueEntry>,
}

/// One standalone plaque image with its art-directed metadata.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaqueEntry {
    pub id: String,
    pub name: String,
    pub video_aspect: String,
    pub path: String,
    pub pixel_size: [u32; 2],
    pub writable_inset: [f64; 4],
    pub sha256: String,
}

impl PlaqueCatalog {
    /// Parse a catalog from TOML text.
    pub fn parse(source: &str) -> Result<Self> {
        toml::from_str(source).context("plaque catalog is invalid TOML")
    }

    /// Load and parse the catalog at `path`.
    pub fn load(path: &Path) -> Result<Self> {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        Self::parse(&source).with_context(|| format!("in {}", path.display()))
    }
}
