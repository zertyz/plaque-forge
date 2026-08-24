//! Behavior over an embedded media index, independent of how it was built.
//!
//! [`EmbeddedIndex`] carries every listing, lookup, and extraction decision;
//! the `bundle-media` feature only supplies the generated tables and the
//! linked blob. Asset bytes live in one raw object file appended to the link
//! (never inside `rustc` as literals), so compiling the payload costs the
//! compiler nothing regardless of media size. This keeps the entire behavior
//! surface testable with tiny synthetic indexes.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

use super::contract::{FontListing, PlaqueListing, StyleListing, TextureListing, VideoListing};
use super::fonts::{
    CuratedListingItem, FamilyIndex, compose_curated_labels, compose_font_list, path_stem,
};
use super::plaques::PlaqueCatalog;

/// Canonical repository roots that a bundled build serves internally.
pub const CANONICAL_ROOTS: [&str; 3] = ["assets", "styles", "fonts"];

/// One media file carried inside a bundled binary.
#[derive(Debug)]
pub struct EmbeddedAsset {
    /// Repository-relative, forward-separated path.
    pub path: &'static str,
    /// Offset of this asset's bytes inside the bundle blob.
    pub offset: usize,
    /// Length of this asset's bytes inside the bundle blob.
    pub len: usize,
}

/// Provenance of one curated font entry inside a bundled binary.
#[derive(Debug)]
pub struct CuratedFontEmbedding {
    /// Curated-font pattern as written in `styles/curated_fonts`.
    pub pattern: &'static str,
    /// True for repository-pinned `fonts/<file>` entries.
    pub repository_file: bool,
    /// Bundle-relative location of the embedded font bytes.
    pub bundle_path: &'static str,
    /// SHA-256 of the embedded bytes (build-machine resolution provenance).
    pub sha256: &'static str,
    /// Family name fontconfig answered on the building machine; listing
    /// labels prefer it over re-resolving on whichever machine runs. `None`
    /// for pinned repository files, which list under their file stem.
    pub resolved_family: Option<&'static str>,
}

/// Read-only view of one binary's embedded media.
#[derive(Debug, Clone, Copy)]
pub struct EmbeddedIndex {
    pub entries: &'static [EmbeddedAsset],
    pub curated_fonts: &'static [CuratedFontEmbedding],
    pub bundle_id: &'static str,
    /// The concatenated media payload backing every entry.
    pub blob: &'static [u8],
}

/// Overrides the cache location for materialized embedded media.
pub const CACHE_DIR_ENV: &str = "PLAQUE_FORGE_BUNDLE_CACHE";

/// Writes embedded byte ranges below a fixed mirror root, idempotently.
///
/// The rendering pipeline (OpenCV capture, ffmpeg, texture loading) consumes
/// real file paths, so a bundled build materializes exactly the embedded
/// files a workflow touches into a content-stable cache directory mirroring
/// the repository layout. Explicit user paths always win over extraction.
pub struct Materializer {
    root: PathBuf,
}

impl EmbeddedIndex {
    /// Binary search for one embedded file by bundle-relative path.
    pub fn find(&self, bundle_path: &str) -> Option<&'static EmbeddedAsset> {
        let position = self
            .entries
            .binary_search_by(|asset| asset.path.cmp(bundle_path))
            .ok()?;
        Some(&self.entries[position])
    }

    /// The static bytes of one entry, sliced out of the bundle blob.
    pub fn asset_bytes(&self, asset: &EmbeddedAsset) -> &'static [u8] {
        let start = asset.offset;
        let end = start
            .checked_add(asset.len)
            .filter(|end| *end <= self.blob.len())
            .unwrap_or_else(|| {
                panic!(
                    "embedded asset {} [{}..{}] escapes the {}-byte blob",
                    asset.path,
                    start,
                    start + asset.len,
                    self.blob.len()
                )
            });
        &self.blob[start..end]
    }

    /// Embedded assets whose path lies under `prefix`, which must end in `/`.
    pub fn entries_under(&self, prefix: &str) -> impl Iterator<Item = &'static EmbeddedAsset> {
        let start = self
            .entries
            .partition_point(|asset| asset.path.as_bytes() < prefix.as_bytes());
        self.entries[start..]
            .iter()
            .take_while(move |asset| asset.path.starts_with(prefix))
    }

    /// Bundle-relative form of a user path when it names something inside
    /// the canonical layout.
    ///
    /// Only plain name components after the root are accepted, so a bundled
    /// build can never be tricked into naming files outside its bundle.
    pub fn normalize_relative(&self, raw: &Path) -> Option<String> {
        let mut components = raw.components().peekable();
        while let Some(component) = components.peek() {
            let name = component.as_os_str().to_string_lossy().into_owned();
            if CANONICAL_ROOTS.contains(&name.as_str()) {
                break;
            }
            components.next();
        }
        let mut relative = Vec::new();
        for component in components {
            match component {
                Component::Normal(part) => relative.push(part.to_string_lossy().into_owned()),
                _ => return None,
            }
        }
        let joined = relative.join("/");
        if joined.is_empty() {
            None
        } else {
            Some(joined)
        }
    }

    /// Map a user-supplied path onto the embedded layout.
    pub fn lookup(&self, raw: &Path) -> Option<&'static EmbeddedAsset> {
        self.find(&self.normalize_relative(raw)?)
    }

    /// Extract one embedded file into the mirror.
    pub fn extract(&self, materializer: &Materializer, asset: &EmbeddedAsset) -> Result<PathBuf> {
        materializer.extract(self, asset)
    }

    /// Extract every asset under a directory prefix; empty prefixes produce
    /// no destinations.
    pub fn extract_prefix(
        &self,
        materializer: &Materializer,
        prefix: &str,
    ) -> Result<Vec<PathBuf>> {
        materializer.extract_all(self, self.entries_under(prefix).collect::<Vec<_>>())
    }

    /// Extract one user-facing read path if it names an embedded asset.
    pub fn remap_file(&self, materializer: &Materializer, raw: &Path) -> Result<PathBuf> {
        match self.lookup(raw) {
            Some(asset) => materializer.extract(self, asset),
            None => Ok(raw.to_path_buf()),
        }
    }

    /// Curated labels through the shared composition contract: recorded
    /// build-time families win, then this machine's resolution, then the raw
    /// pattern so an unresolvable entry never silently disappears.
    fn curated_labels(&self, index: &dyn FamilyIndex) -> Vec<String> {
        compose_curated_labels(
            self.curated_fonts
                .iter()
                .map(|embedding| CuratedListingItem {
                    pattern: embedding.pattern,
                    repository_file: embedding.repository_file,
                    bundle_path: embedding.bundle_path,
                    resolved_family: embedding.resolved_family,
                }),
            index,
        )
    }

    fn sorted_stems<'a>(&self, assets: impl Iterator<Item = &'a EmbeddedAsset>) -> Vec<String> {
        let mut stems: Vec<String> = assets.map(|asset| path_stem(asset.path)).collect();
        stems.sort();
        stems.dedup();
        stems
    }

    /// Inventory over this index with system families supplied separately.
    pub fn inventory(&self, families: &dyn FamilyIndex) -> Result<MediaInventoryView> {
        Ok(MediaInventoryView {
            videos: self
                .sorted_stems(self.entries_under("assets/").filter(|asset| {
                    matches!(
                        asset.path.strip_prefix("assets/"),
                        Some(relative)
                            if relative.ends_with(".mp4") && !relative.contains('/')
                    )
                }))
                .into_iter()
                .map(|stem| VideoListing { stem })
                .collect(),
            styles: self
                .sorted_stems(
                    self.entries_under("styles/")
                        .filter(|asset| asset.path.ends_with(".toml")),
                )
                .into_iter()
                .map(|name| StyleListing { name })
                .collect(),
            textures: self
                .sorted_stems(
                    self.entries_under("assets/textures/")
                        .filter(|asset| asset.path.ends_with(".png")),
                )
                .into_iter()
                .map(|name| TextureListing { name })
                .collect(),
            plaques: match self.find("assets/plaques/catalog.toml") {
                None => Vec::new(),
                Some(catalog) => {
                    let source = std::str::from_utf8(self.asset_bytes(catalog))
                        .context("embedded plaque catalog is not UTF-8")?;
                    PlaqueCatalog::parse(source)?
                        .plaques
                        .into_iter()
                        .map(|plaque| PlaqueListing {
                            id: plaque.id,
                            name: plaque.name,
                            video_aspect: plaque.video_aspect,
                            pixel_size: plaque.pixel_size,
                        })
                        .collect()
                }
            },
            fonts: compose_font_list(&self.curated_labels(families), families),
        })
    }
}

/// Everything [`EmbeddedIndex`] can list, shaped like application requests.
#[derive(Debug, Default)]
pub struct MediaInventoryView {
    pub videos: Vec<VideoListing>,
    pub styles: Vec<StyleListing>,
    pub plaques: Vec<PlaqueListing>,
    pub textures: Vec<TextureListing>,
    pub fonts: Vec<FontListing>,
}

impl Materializer {
    /// Cache root bound to one exact bundle identity, so different builds
    /// never serve each other's stale extractions.
    pub fn for_bundle(index: &EmbeddedIndex) -> Result<Self> {
        let base = match std::env::var_os(CACHE_DIR_ENV) {
            Some(override_root) => PathBuf::from(override_root),
            None => system_cache_base()?.join("plaque-forge/materialized"),
        };
        Self::over(base.join(index.bundle_id))
    }

    /// Build a materializer over an explicit root (test seam).
    pub fn over(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&root)
            .with_context(|| format!("failed to create {}", root.display()))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Absolute path where `asset` lives once extracted.
    pub fn destination(&self, asset: &EmbeddedAsset) -> PathBuf {
        self.root.join(asset.path)
    }

    /// Write `asset` unless the mirror already holds a copy.
    pub fn extract(&self, index: &EmbeddedIndex, asset: &EmbeddedAsset) -> Result<PathBuf> {
        self.write(asset.path, index.asset_bytes(asset))
    }

    /// Write one bundle-relative file unless the mirror already holds one.
    ///
    /// Existence wins over content: the mirror has two producers — bundle
    /// extraction seeding it, and workflows writing regenerated outputs such
    /// as rebuilt analysis caches. Once a file exists it belongs to whichever
    /// producer wrote it last, so re-extraction must never clobber newer
    /// workflow output with older embedded bytes (that would make every
    /// `--if-needed` workflow rebuild forever). Writes stay atomic via a
    /// temporary plus rename, so an existing file is always complete.
    pub fn write(&self, relative: &str, bytes: &[u8]) -> Result<PathBuf> {
        let destination = self.root.join(relative);
        if destination.is_file() {
            return Ok(destination);
        }
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let temporary = destination.with_extension(format!(
            "{}.{:x}.part",
            destination
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("dat"),
            std::process::id()
        ));
        std::fs::write(&temporary, bytes)
            .with_context(|| format!("failed to write {}", temporary.display()))?;
        std::fs::rename(&temporary, &destination)
            .with_context(|| format!("failed to finalize {}", destination.display()))?;
        Ok(destination)
    }

    /// Extract every provided asset, returning their absolute destinations.
    pub fn extract_all<'a>(
        &self,
        index: &EmbeddedIndex,
        assets: impl IntoIterator<Item = &'a EmbeddedAsset>,
    ) -> Result<Vec<PathBuf>> {
        assets
            .into_iter()
            .map(|asset| self.extract(index, asset))
            .collect()
    }
}

fn system_cache_base() -> Result<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(xdg));
    }
    let home = std::env::var_os("HOME")
        .context("cannot locate a materialization cache: set PLAQUE_FORGE_BUNDLE_CACHE or HOME")?;
    Ok(Path::new(&home).join(".cache"))
}
