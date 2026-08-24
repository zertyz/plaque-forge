//! Media-catalog behavior: listings, ordering, and curated-font composition.
//!
//! The filesystem backend is exercised through real directories built as
//! fixtures; the family index is injected so typeface results never depend on
//! whichever fonts happen to be installed on the host running the tests.

mod support;

use std::{fs, path::PathBuf, sync::Arc};

use plaque_forge::media::{FilesystemCatalog, FontListing, MediaCatalog};

use support::{FakeFamilies, temp_root};

fn write(path: PathBuf) -> PathBuf {
    write_bytes(path, b"fixture")
}

fn write_bytes(path: PathBuf, bytes: &[u8]) -> PathBuf {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("fixture parent directory");
    }
    fs::write(&path, bytes).expect("fixture file");
    path
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn build() -> Self {
        let root = temp_root("media-catalog");
        write(root.join("assets/b-video.mp4"));
        write(root.join("assets/a-video.mp4"));
        write(root.join("assets/notes.txt"));
        write(root.join("styles/silver.toml"));
        write(root.join("styles/gold.toml"));
        write(root.join("assets/textures/zinc.png"));
        write(root.join("assets/textures/amber.png"));
        write_bytes(
            root.join("assets/plaques/catalog.toml"),
            b"schema_version = 1\n\
              [[plaques]]\n\
              id = \"beta-plaque\"\n\
              name = \"Beta Plaque\"\n\
              video_aspect = \"16:9\"\n\
              path = \"beta.png\"\n\
              pixel_size = [1500, 420]\n\
              writable_inset = [0.08, 0.12, 0.08, 0.12]\n\
              sha256 = \"deadbeef\"\n\
              [[plaques]]\n\
              id = \"alpha-plaque\"\n\
              name = \"Alpha Plaque\"\n\
              video_aspect = \"9:16\"\n\
              path = \"alpha.png\"\n\
              pixel_size = [1000, 280]\n\
              writable_inset = [0.10, 0.16, 0.10, 0.16]\n\
              sha256 = \"feedface\"\n",
        );
        write_bytes(
            root.join("styles/curated_fonts"),
            b"# curated\n\
             fonts/Pin.ttf\n\
             Curated One\n\
             Missing On Purpose\n",
        );
        write(root.join("fonts/Pin.ttf"));
        Self { root }
    }

    fn catalog(&self, families: FakeFamilies) -> FilesystemCatalog {
        FilesystemCatalog::new(self.root.clone(), Arc::new(families)).expect("catalog construction")
    }

    fn labels(fonts: &[FontListing]) -> Vec<(bool, String)> {
        fonts
            .iter()
            .map(|font| (font.curated, font.label.clone()))
            .collect()
    }
}

#[test]
fn lists_videos_styles_and_textures_sorted_by_name() {
    let fixture = Fixture::build();
    let catalog = fixture.catalog(FakeFamilies::new(&[]));

    assert_eq!(
        catalog.videos().unwrap(),
        vec![
            plaque_forge::media::VideoListing {
                stem: "a-video".into()
            },
            plaque_forge::media::VideoListing {
                stem: "b-video".into()
            },
        ],
        "videos must be sorted by stem and ignore non-mp4 files"
    );
    assert_eq!(
        catalog.styles().unwrap(),
        vec![
            plaque_forge::media::StyleListing {
                name: "gold".into()
            },
            plaque_forge::media::StyleListing {
                name: "silver".into()
            },
        ],
        "styles must be sorted by name"
    );
    assert_eq!(
        catalog.textures().unwrap(),
        vec![
            plaque_forge::media::TextureListing {
                name: "amber".into()
            },
            plaque_forge::media::TextureListing {
                name: "zinc".into()
            },
        ],
        "textures must be sorted by name"
    );
}

#[test]
fn surfaces_plaque_catalog_metadata_in_catalog_order() {
    let fixture = Fixture::build();
    let catalog = fixture.catalog(FakeFamilies::new(&[]));

    let plaques = catalog.plaques().unwrap();
    assert_eq!(2, plaques.len(), "both catalog entries are listed");
    assert_eq!("beta-plaque", plaques[0].id, "catalog order is preserved");
    assert_eq!("Beta Plaque", plaques[0].name);
    assert_eq!("16:9", plaques[0].video_aspect);
    assert_eq!([1500, 420], plaques[0].pixel_size);
    assert_eq!("alpha-plaque", plaques[1].id);
    assert_eq!([1000, 280], plaques[1].pixel_size);
}

#[test]
fn fonts_list_curated_first_then_remaining_system_families() {
    let fixture = Fixture::build();
    let families = FakeFamilies::new(&["Alpha", "curated one", "Beta"])
        .resolving("Curated One", "Curated One");
    let catalog = fixture.catalog(families);

    assert_eq!(
        Fixture::labels(&catalog.fonts().unwrap()),
        vec![
            (true, "Pin".to_string()),
            (true, "Curated One".to_string()),
            (true, "Missing On Purpose".to_string()),
            (false, "Alpha".to_string()),
            (false, "Beta".to_string()),
        ],
        "curated fonts lead in curated-file order; resolved families are excluded \
         from the system section; unresolvable patterns keep their raw label"
    );
}

#[test]
fn cross_kind_curated_label_collisions_keep_the_first_entry() {
    let root = temp_root("media-catalog-dedupe");
    write_bytes(root.join("styles/curated_fonts"), b"fonts/Pin.ttf\nPin\n");
    write(root.join("fonts/Pin.ttf"));
    let families = FakeFamilies::new(&["Gamma"]).resolving("Pin", "Pin");
    let catalog = FilesystemCatalog::new(root, Arc::new(families)).expect("catalog construction");

    assert_eq!(
        Fixture::labels(&catalog.fonts().unwrap()),
        vec![(true, "Pin".to_string()), (false, "Gamma".to_string()),],
        "a family resolving onto an already-listed curated label must not repeat"
    );
}

#[test]
fn missing_conventional_directories_list_as_empty_instead_of_failing() {
    let root = temp_root("media-catalog-empty");
    let catalog = FilesystemCatalog::new(root, Arc::new(FakeFamilies::new(&["Solo"]))).unwrap();

    assert!(catalog.videos().unwrap().is_empty(), "no assets directory");
    assert!(catalog.styles().unwrap().is_empty(), "no styles directory");
    assert!(
        catalog.textures().unwrap().is_empty(),
        "no textures directory"
    );
    assert!(catalog.plaques().unwrap().is_empty(), "no plaque catalog");
    assert_eq!(
        Fixture::labels(&catalog.fonts().unwrap()),
        vec![(false, "Solo".to_string())],
        "without a curated list every installed family lists as system"
    );
}

#[test]
fn inventory_requests_select_only_requested_kinds() {
    use plaque_forge::application::{ListRequest, list};

    let fixture = Fixture::build();
    let families = FakeFamilies::new(&["Alpha"]).resolving("Curated One", "Curated One");
    let catalog = fixture.catalog(families);

    let fonts = list(
        ListRequest {
            kind: plaque_forge::application::MediaKind::Fonts,
        },
        &catalog,
    )
    .unwrap();
    assert!(
        fonts.videos.is_empty() && fonts.styles.is_empty(),
        "unrequested kinds stay empty"
    );
    let curated = fonts.fonts.iter().filter(|font| font.curated).count();
    assert_eq!(
        3, curated,
        "every curated_fonts entry lists under the request"
    );
    assert!(
        matches!(fonts.fonts.first(), Some(font) if font.label == "Pin" && font.curated),
        "the pinned repository font leads the listing"
    );

    let everything = list(ListRequest::all(), &catalog).unwrap();
    assert!(
        !everything.videos.is_empty(),
        "all-kind requests populate videos"
    );
    assert!(
        !everything.styles.is_empty(),
        "all-kind requests populate styles"
    );
    assert!(
        !everything.plaques.is_empty(),
        "all-kind requests populate plaques"
    );
    assert!(
        !everything.textures.is_empty(),
        "all-kind requests populate textures"
    );
    assert!(
        !everything.fonts.is_empty(),
        "all-kind requests populate fonts"
    );
}
