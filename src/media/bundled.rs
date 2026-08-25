//! Media catalog over the data embedded by the `bundle-media` feature.
//!
//! This module is thin glue: `build.rs` concatenates every media file into
//! one raw blob, turns it into an object file with plain binutils, and adds
//! that object to the link. Only kilobytes of offset tables ever pass through
//! rustc, so payload size does not affect compiler memory. All behavior lives
//! in [`crate::media::index`]. Homologation evidence is intentionally not
//! embedded; it remains an on-disk, CI-gated responsibility over a repository
//! checkout.

use std::sync::Arc;

use anyhow::Result;

use super::contract::{
    FontListing, MediaCatalog, PlaqueListing, StyleListing, TextureListing, VideoListing,
};
use super::fonts::{FamilyIndex, SystemFonts};
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
        if end < start || len > isize::MAX as usize {
            // Build invariant: `build.rs` links the blob with `ld -r -b binary`.
            // A corrupted link is unrecoverable, but as a library we must not
            // abort the process. Return an empty blob and let callers surface
            // a diagnostic `Err` when they find no entries.
            eprintln!("error: bundle blob symbols are inconsistent; returning empty blob");
            return &[];
        }
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
pub struct BundledMedia {
    families: Arc<dyn FamilyIndex>,
}

impl BundledMedia {
    /// One system-font scan per process, shared by every listing request.
    pub fn production() -> Self {
        Self {
            families: Arc::new(SystemFonts::load()),
        }
    }
}

impl MediaCatalog for BundledMedia {
    fn videos(&self) -> Result<Vec<VideoListing>> {
        Ok(index().inventory(self.families.as_ref())?.videos)
    }

    fn styles(&self) -> Result<Vec<StyleListing>> {
        Ok(index().inventory(self.families.as_ref())?.styles)
    }

    fn textures(&self) -> Result<Vec<TextureListing>> {
        Ok(index().inventory(self.families.as_ref())?.textures)
    }

    fn plaques(&self) -> Result<Vec<PlaqueListing>> {
        Ok(index().inventory(self.families.as_ref())?.plaques)
    }

    fn fonts(&self) -> Result<Vec<FontListing>> {
        Ok(index().inventory(self.families.as_ref())?.fonts)
    }
}

/// Materializer bound to this binary's exact bundle identity.
pub fn production_materializer() -> Result<Materializer> {
    Materializer::for_bundle(&index())
}
