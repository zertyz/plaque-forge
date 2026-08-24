//! Media inventory: what this build can name, list, and (when bundled) carry.
//!
//! The [`MediaCatalog`] contract abstracts where media comes from: a
//! repository checkout on disk (`filesystem`), or data embedded by the
//! `bundle-media` feature (`bundled`). Workflows and the CLI depend only on
//! the contract, so listing behaves identically in both builds apart from the
//! source of truth. Embedded-index behavior ([`index`]) and system font
//! discovery ([`fonts::FamilyIndex`]) are shared by both backends.

pub mod contract;
pub mod curated;
pub mod filesystem;
pub mod fonts;
pub mod plaques;

pub mod index;

#[cfg(feature = "bundle-media")]
pub mod bundled;

pub use contract::{
    FontListing, MediaCatalog, PlaqueListing, StyleListing, TextureListing, VideoListing,
};
pub use filesystem::{CURATED_FONTS_FILE, FilesystemCatalog};
