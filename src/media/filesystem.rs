//! Media catalog backed by a repository checkout on disk.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

use super::contract::{
    FontListing, MediaCatalog, PlaqueListing, StyleListing, TextureListing, VideoListing,
};
use super::curated::{CuratedFont, parse_curated_fonts};
use super::fonts::{FamilyIndex, SystemFonts, compose_font_list};
use super::plaques::PlaqueCatalog;

/// Location of the curated font list inside the styles directory.
pub const CURATED_FONTS_FILE: &str = "styles/curated_fonts";

/// Lists media by reading the conventional repository directories.
///
/// Missing conventional directories list as empty so partial checkouts stay
/// usable; unreadable directories are errors.
pub struct FilesystemCatalog {
    root: PathBuf,
    curated: Vec<CuratedFont>,
    families: Arc<dyn FamilyIndex>,
}

impl FilesystemCatalog {
    /// Catalog rooted at the working directory, matching script conventions.
    pub fn production() -> Result<Self> {
        Self::new(PathBuf::from("."), Arc::new(SystemFonts::load()))
    }

    /// Build a catalog over an explicit root with an injectable family index.
    pub fn new(root: PathBuf, families: Arc<dyn FamilyIndex>) -> Result<Self> {
        let curated = match std::fs::read_to_string(root.join(CURATED_FONTS_FILE)) {
            Ok(source) => parse_curated_fonts(&source)
                .with_context(|| format!("invalid {}", CURATED_FONTS_FILE))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read {}", CURATED_FONTS_FILE));
            }
        };
        Ok(Self {
            root,
            curated,
            families,
        })
    }

    /// Display labels for curated entries, in curated-file order.
    ///
    /// A pinned file lists under its file stem; a family pattern lists under
    /// its canonical resolved family name (the raw pattern when this machine
    /// has no matching face). Cross-kind label collisions keep their first
    /// occurrence so every listed entry stays unambiguous.
    fn curated_labels(&self) -> Vec<String> {
        let mut labels = Vec::new();
        let mut seen = BTreeSet::new();
        for entry in &self.curated {
            let label = match entry {
                CuratedFont::Repository { path } => Path::new(path)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or(path)
                    .to_string(),
                CuratedFont::Family { pattern } => self
                    .families
                    .match_pattern(pattern)
                    .unwrap_or_else(|| pattern.clone()),
            };
            if seen.insert(label.to_lowercase()) {
                labels.push(label);
            }
        }
        labels
    }

    fn sorted_stems(&self, directory: &str, extension: &str) -> Result<Vec<String>> {
        let mut names = Vec::new();
        if let Some(entries) = self.optional_entries(directory)? {
            for entry in entries {
                let entry = entry.context(format!("failed to read {}", directory))?;
                if entry.path().extension().and_then(|value| value.to_str()) == Some(extension)
                    && let Some(stem) = entry.path().file_stem().and_then(|stem| stem.to_str())
                {
                    names.push(stem.to_string());
                }
            }
        }
        names.sort();
        Ok(names)
    }

    fn optional_entries(&self, directory: &str) -> Result<Option<std::fs::ReadDir>> {
        match std::fs::read_dir(self.root.join(directory)) {
            Ok(entries) => Ok(Some(entries)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).context(format!("failed to read {}", directory)),
        }
    }
}

impl MediaCatalog for FilesystemCatalog {
    fn videos(&self) -> Result<Vec<VideoListing>> {
        Ok(self
            .sorted_stems("assets", "mp4")?
            .into_iter()
            .map(|stem| VideoListing { stem })
            .collect())
    }

    fn styles(&self) -> Result<Vec<StyleListing>> {
        Ok(self
            .sorted_stems("styles", "toml")?
            .into_iter()
            .map(|name| StyleListing { name })
            .collect())
    }

    fn textures(&self) -> Result<Vec<TextureListing>> {
        Ok(self
            .sorted_stems("assets/textures", "png")?
            .into_iter()
            .map(|name| TextureListing { name })
            .collect())
    }

    fn plaques(&self) -> Result<Vec<PlaqueListing>> {
        let catalog_path = self.root.join("assets/plaques/catalog.toml");
        if !catalog_path.is_file() {
            return Ok(Vec::new());
        }
        let catalog = PlaqueCatalog::load(&catalog_path)?;
        Ok(catalog
            .plaques
            .into_iter()
            .map(|plaque| PlaqueListing {
                id: plaque.id,
                name: plaque.name,
                video_aspect: plaque.video_aspect,
                pixel_size: plaque.pixel_size,
            })
            .collect())
    }

    fn fonts(&self) -> Result<Vec<FontListing>> {
        Ok(compose_font_list(
            &self.curated_labels(),
            self.families.as_ref(),
        ))
    }
}
