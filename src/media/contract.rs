//! The media-source contract every listing backend implements.
//!
//! Workflows and the CLI depend on [`MediaCatalog`], never on where the
//! catalog reads from: a repository checkout (`filesystem`) or data embedded
//! by `bundle-media` (`bundled`). Listing order is part of the contract so
//! both backends present media identically.

use anyhow::Result;
use serde::Serialize;

/// Input videos available to this build, sorted by stem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VideoListing {
    pub stem: String,
}

/// Reusable text-style programs (`styles/<name>.toml`), sorted by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StyleListing {
    pub name: String,
}

/// Style texture images (`assets/textures/<name>.png`), sorted by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextureListing {
    pub name: String,
}

/// One standalone plaque as declared by the plaque catalog, in catalog order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlaqueListing {
    pub id: String,
    pub name: String,
    pub video_aspect: String,
    pub pixel_size: [u32; 2],
}

/// One typeface family. Curated entries come first, in `curated_fonts` order,
/// followed by the remaining installed system families alphabetically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FontListing {
    pub label: String,
    pub curated: bool,
}

/// Read-only inventory of every media kind this build can name.
pub trait MediaCatalog {
    fn videos(&self) -> Result<Vec<VideoListing>>;
    fn styles(&self) -> Result<Vec<StyleListing>>;
    fn plaques(&self) -> Result<Vec<PlaqueListing>>;
    fn textures(&self) -> Result<Vec<TextureListing>>;
    fn fonts(&self) -> Result<Vec<FontListing>>;
}
