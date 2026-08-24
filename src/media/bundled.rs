//! Media catalog over the data embedded by the `bundle-media` feature.
//!
//! This module is thin glue: `build.rs` concatenates every media file into
//! one raw blob, turns it into an object file with plain binutils, and adds
//! that object to the link. Only kilobytes of offset tables ever pass through
//! rustc, so payload size does not affect compiler memory. All behavior lives
//! in [`crate::media::index`]. Homologation evidence is intentionally not
//! embedded; it remains an on-disk, CI-gated responsibility over a repository
//! checkout.

use anyhow::Result;

use super::contract::{
    FontListing, MediaCatalog, PlaqueListing, StyleListing, TextureListing, VideoListing,
};
use super::fonts::SystemFonts;
use super::index::{EmbeddedIndex, Materializer};

include!(concat!(env!("OUT_DIR"), "/bundled_media.rs"));

// Produced by `ld -r -b binary bundle_blob.bin` in build.rs; symbol names
// derive from the fixed file name.
unsafe extern "C" {
    static _binary_bundle_blob_bin_start: u8;
    static _binary_bundle_blob_bin_end: u8;
}

/// The embedded media payload linked into this binary.
pub fn blob() -> &'static [u8] {
    unsafe {
        let start = std::ptr::addr_of!(_binary_bundle_blob_bin_start);
        let end = std::ptr::addr_of!(_binary_bundle_blob_bin_end);
        let len = end.offset_from(start) as usize;
        assert!(
            end >= start && len <= isize::MAX as usize,
            "bundle blob symbols are inconsistent"
        );
        std::slice::from_raw_parts(start, len)
    }
}

/// The embedded media carried by this binary.
pub fn index() -> EmbeddedIndex {
    EmbeddedIndex {
        entries: ENTRIES,
        curated_fonts: CURATED_FONT_EMBEDDINGS,
        bundle_id: BUNDLE_ID,
        blob: blob(),
    }
}

/// Lists media from the data carried inside this binary.
pub struct BundledMedia;

impl BundledMedia {
    pub fn production() -> Self {
        Self
    }

    fn families() -> SystemFonts {
        SystemFonts::load()
    }
}

impl MediaCatalog for BundledMedia {
    fn videos(&self) -> Result<Vec<VideoListing>> {
        Ok(index().inventory(&BundledMedia::families())?.videos)
    }

    fn styles(&self) -> Result<Vec<StyleListing>> {
        Ok(index().inventory(&BundledMedia::families())?.styles)
    }

    fn textures(&self) -> Result<Vec<TextureListing>> {
        Ok(index().inventory(&BundledMedia::families())?.textures)
    }

    fn plaques(&self) -> Result<Vec<PlaqueListing>> {
        Ok(index().inventory(&BundledMedia::families())?.plaques)
    }

    fn fonts(&self) -> Result<Vec<FontListing>> {
        let system = SystemFonts::load();
        Ok(index().inventory(&system)?.fonts)
    }
}

/// Materializer bound to this binary's exact bundle identity.
pub fn production_materializer() -> Result<Materializer> {
    Materializer::for_bundle(&index())
}
