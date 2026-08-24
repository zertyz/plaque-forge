//! Embedded-media behavior: listings, path normalization, and extraction.
//!
//! Every behavior of a `bundle-media` binary is exercised here against tiny
//! synthetic indexes, so none of these tests require compiling or linking the
//! real media payload; the feature itself only supplies the generated tables
//! plus the linked blob.

mod support;

use std::path::Path;

use plaque_forge::media::index::{
    CuratedFontEmbedding, EmbeddedAsset, EmbeddedIndex, Materializer,
};

use support::{FakeFamilies, temp_root};

const VIDEO: &[u8] = b"synthetic-video";
const TEXTURE: &[u8] = b"synthetic-texture";
const PIN_FONT: &[u8] = b"pinned-font-bytes";
const RESOLVED_FONT: &[u8] = b"resolved-font-bytes";
const STYLE: &[u8] = b"style-toml";
const CATALOG: &[u8] = b"schema_version = 1\n\
    [[plaques]]\n\
    id = \"only-plaque\"\n\
    name = \"Only Plaque\"\n\
    video_aspect = \"16:9\"\n\
    path = \"only.png\"\n\
    pixel_size = [10, 20]\n\
    writable_inset = [0.1, 0.1, 0.1, 0.1]\n\
    sha256 = \"cafe\"\n";

static CURATED: &[CuratedFontEmbedding] = &[
    CuratedFontEmbedding {
        pattern: "fonts/Pin.ttf",
        repository_file: true,
        bundle_path: "fonts/Pin.ttf",
        sha256: "pin-digest",
        resolved_family: None,
    },
    CuratedFontEmbedding {
        pattern: "Curated One",
        repository_file: false,
        bundle_path: "fonts/resolved/curated-one.ttf",
        sha256: "resolved-digest",
        resolved_family: None,
    },
    CuratedFontEmbedding {
        pattern: "Build-Time Name",
        repository_file: false,
        bundle_path: "fonts/resolved/build-time-name.ttf",
        sha256: "recorded-digest",
        resolved_family: Some("Recorded Only"),
    },
];

/// Build a self-consistent index whose blob offsets match its entry table.
/// Leaking keeps the required `'static` lifetimes local to each test.
fn synthetic_index() -> EmbeddedIndex {
    let mut blob: Vec<u8> = Vec::new();
    let mut entries: Vec<EmbeddedAsset> = Vec::new();
    {
        let mut push = |path: &'static str, bytes: &[u8], blob: &mut Vec<u8>| {
            entries.push(EmbeddedAsset {
                path,
                offset: blob.len(),
                len: bytes.len(),
            });
            blob.extend_from_slice(bytes);
        };
        // Sorted by path: lookups binary-search the table.
        push("assets/a-video.mp4", VIDEO, &mut blob);
        push("assets/plaques/catalog.toml", CATALOG, &mut blob);
        push("assets/textures/zinc.png", TEXTURE, &mut blob);
        push("fonts/Pin.ttf", PIN_FONT, &mut blob);
        push(
            "fonts/resolved/build-time-name.ttf",
            RESOLVED_FONT,
            &mut blob,
        );
        push("fonts/resolved/curated-one.ttf", RESOLVED_FONT, &mut blob);
        push("styles/gold.toml", STYLE, &mut blob);
    }
    EmbeddedIndex {
        entries: Box::leak(entries.into_boxed_slice()),
        curated_fonts: CURATED,
        bundle_id: "test-bundle",
        blob: Box::leak(blob.into_boxed_slice()),
    }
}

#[test]
fn listings_mirror_the_embedded_layout() {
    let index = synthetic_index();
    let view = index.inventory(&FakeFamilies::new(&["Alpha"])).unwrap();

    assert_eq!(
        vec!["a-video".to_string()],
        view.videos
            .into_iter()
            .map(|video| video.stem)
            .collect::<Vec<_>>(),
        "nested assets (plaques/textures) must not list as videos"
    );
    assert_eq!(view.styles.len(), 1, "styles come from styles/*.toml");
    assert_eq!(view.styles[0].name, "gold");
    assert_eq!(
        vec!["zinc".to_string()],
        view.textures
            .into_iter()
            .map(|texture| texture.name)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        "only-plaque", view.plaques[0].id,
        "catalog parses from the blob"
    );
    assert_eq!([10, 20], view.plaques[0].pixel_size);

    assert_eq!(
        vec![
            (true, "Pin".to_string()),
            (true, "Curated One".to_string()),
            (true, "Recorded Only".to_string()),
            (false, "Alpha".to_string()),
        ],
        view.fonts
            .iter()
            .map(|font| (font.curated, font.label.clone()))
            .collect::<Vec<_>>(),
        "curated entries lead in file order: pinned files by stem, unrecorded \
         families by runtime resolution or raw pattern, recorded build-time \
         families by their recorded name; all are excluded from system names"
    );
}

#[test]
fn asset_bytes_slice_the_blob_at_declared_offsets() {
    let index = synthetic_index();
    let video = index.find("assets/a-video.mp4").unwrap();
    assert_eq!(VIDEO, index.asset_bytes(video), "video range");
    let style = index.find("styles/gold.toml").unwrap();
    assert_eq!(STYLE, index.asset_bytes(style), "style range");
}

#[test]
fn family_patterns_fall_back_to_their_raw_label_without_a_match() {
    // No resolver match at all: the raw pattern stays visible instead of the
    // entry silently disappearing from the listing.
    let view = synthetic_index()
        .inventory(&FakeFamilies::new(&[]))
        .unwrap();
    assert!(
        view.fonts
            .iter()
            .any(|font| font.curated && font.label == "Curated One"),
        "unresolvable curated patterns keep their raw label"
    );
}

#[test]
fn canonical_lookup_accepts_only_plain_names_under_known_roots() {
    let index = synthetic_index();

    for (raw, expected) in [
        ("assets/a-video.mp4", Some("assets/a-video.mp4")),
        ("./styles/gold.toml", Some("styles/gold.toml")),
        ("/fonts/Pin.ttf", Some("fonts/Pin.ttf")),
        ("checkout/assets/a-video.mp4", Some("assets/a-video.mp4")),
        ("../secrets.key", None),
        ("assets/../secrets.key", None),
        ("other/file.txt", None),
    ] {
        let normalized = index.normalize_relative(Path::new(raw));
        assert_eq!(expected, normalized.as_deref(), "normalization of {raw:?}");
    }

    assert!(index.lookup(Path::new("assets/a-video.mp4")).is_some());
    assert!(index.lookup(Path::new("assets/missing.mp4")).is_none());
}

#[test]
fn materialization_mirrors_layout_and_is_idempotent() {
    let root = temp_root("embedded-materialize");
    let cache = Materializer::over(root.clone()).unwrap();
    let index = synthetic_index();
    let asset = index.find("assets/a-video.mp4").unwrap();

    let first = index.extract(&cache, asset).unwrap();
    assert_eq!(root.join("assets/a-video.mp4"), first);
    assert_eq!(VIDEO, std::fs::read(&first).unwrap().as_slice());

    // A second extraction reuses the existing equally sized copy.
    let second = index.extract(&cache, asset).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        VIDEO,
        std::fs::read(&second).unwrap().as_slice(),
        "re-extraction must not corrupt an already-present copy"
    );
}

#[test]
fn prefix_extraction_and_remaps_cover_the_workflow_surface() {
    let root = temp_root("embedded-remap");
    let cache = Materializer::over(root).unwrap();
    let index = synthetic_index();

    // Nothing lives under this prefix; extracting it is a no-op.
    assert!(
        index
            .extract_prefix(&cache, "assets/scenes/")
            .unwrap()
            .is_empty()
    );

    let extracted = index.extract_prefix(&cache, "fonts/").unwrap();
    assert_eq!(3, extracted.len(), "every embedded font extracts");
    assert!(extracted.iter().all(|path| path.starts_with(cache.root())));

    let remapped = index
        .remap_file(&cache, Path::new("styles/gold.toml"))
        .unwrap();
    assert_eq!(STYLE, std::fs::read(&remapped).unwrap().as_slice());

    // Paths outside the bundle pass through untouched.
    let passthrough = index
        .remap_file(&cache, Path::new("output/render.mkv"))
        .unwrap();
    assert_eq!(Path::new("output/render.mkv"), passthrough);
}
