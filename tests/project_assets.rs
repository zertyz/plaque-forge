use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path},
};

use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaqueCatalog {
    schema_version: u32,
    plaques: Vec<PlaqueEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaqueEntry {
    id: String,
    name: String,
    video_aspect: String,
    path: String,
    pixel_size: [u32; 2],
    writable_inset: [f64; 4],
    sha256: String,
}

fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn plaque_catalog_is_complete_portable_and_matches_the_pngs() {
    let catalog_path = repository_root().join("assets/plaques/catalog.toml");
    let catalog: PlaqueCatalog = toml::from_str(
        &fs::read_to_string(&catalog_path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", catalog_path.display())),
    )
    .expect("plaque catalog is invalid TOML");
    assert_eq!(catalog.schema_version, 1);
    assert!(!catalog.plaques.is_empty());

    let mut ids = BTreeSet::new();
    let mut paths = BTreeSet::new();
    let mut families: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for plaque in &catalog.plaques {
        assert!(ids.insert(&plaque.id), "duplicate plaque id {}", plaque.id);
        assert!(
            paths.insert(&plaque.path),
            "duplicate plaque path {}",
            plaque.path
        );
        assert!(!plaque.name.trim().is_empty());
        assert!(matches!(plaque.video_aspect.as_str(), "16:9" | "9:16"));
        families
            .entry(&plaque.name)
            .or_default()
            .insert(&plaque.video_aspect);

        let relative = Path::new(&plaque.path);
        assert!(
            !relative.is_absolute(),
            "absolute plaque path: {}",
            plaque.path
        );
        assert!(!plaque.path.contains('\\'), "non-portable plaque path");
        assert!(
            relative
                .components()
                .all(|component| matches!(component, Component::Normal(_))),
            "plaque path must remain inside its catalog: {}",
            plaque.path
        );
        let path = catalog_path.parent().unwrap().join(relative);
        let bytes = fs::read(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        assert_eq!(Sha256::digest(&bytes)
    .iter()
    .map(|b| format!("{:02x}", b))
    .collect::<String>(), plaque.sha256);
        let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            .unwrap_or_else(|error| panic!("failed to decode {}: {error}", path.display()))
            .to_rgba8();
        assert_eq!([image.width(), image.height()], plaque.pixel_size);
        let (mut minimum_alpha, mut maximum_alpha, mut has_soft_alpha) = (255, 0, false);
        for alpha in image.pixels().map(|pixel| pixel.0[3]) {
            minimum_alpha = minimum_alpha.min(alpha);
            maximum_alpha = maximum_alpha.max(alpha);
            has_soft_alpha |= (1..255).contains(&alpha);
        }
        assert_eq!(
            minimum_alpha, 0,
            "{} has no transparent exterior",
            plaque.id
        );
        assert!(
            maximum_alpha >= 192,
            "{} is almost entirely transparent",
            plaque.id
        );
        assert!(
            has_soft_alpha,
            "{} has no anti-aliased/soft alpha",
            plaque.id
        );

        let [left, top, right, bottom] = plaque.writable_inset;
        assert!(
            [left, top, right, bottom]
                .iter()
                .all(|value| (0.0..=0.45).contains(value))
        );
        assert!(left + right < 1.0 && top + bottom < 1.0);
    }

    let expected = BTreeSet::from(["16:9", "9:16"]);
    for (name, aspects) in families {
        assert_eq!(
            aspects, expected,
            "plaque family {name} is missing an aspect"
        );
    }
}

#[test]
fn project_asset_text_does_not_contain_workstation_paths() {
    let root = repository_root().join("assets");
    if !root.is_dir() {
        return;
    }
    let mut pending = vec![root];
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(&path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            assert_ne!(
                path.file_name().and_then(|name| name.to_str()),
                Some("owner"),
                "private transaction owner marker was published: {}",
                path.display()
            );
            if !matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("toml" | "json" | "html" | "txt" | "md") | None
            ) {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            for forbidden in ["/home/", "/Users/", "file:///", "C:\\", "C:/"] {
                assert!(
                    !text.contains(forbidden),
                    "{} contains workstation path marker {forbidden:?}",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn every_video_has_a_valid_scene_with_the_matching_relative_source() {
    let assets = repository_root().join("assets");
    for entry in fs::read_dir(&assets).unwrap() {
        let video = entry.unwrap().path();
        if video.extension().and_then(|value| value.to_str()) != Some("mp4") {
            continue;
        }
        let stem = video.file_stem().unwrap();
        let scene_path = assets.join("scenes").join(stem).join("scene.toml");
        let scene = plaque_forge::scene::Scene::load(&scene_path)
            .unwrap_or_else(|error| panic!("{} is invalid: {error:#}", scene_path.display()));
        let source = plaque_forge::scene::resolve_relative(&scene_path, &scene.source);
        assert_eq!(
            fs::canonicalize(&source).unwrap(),
            fs::canonicalize(&video).unwrap(),
            "{} names the wrong source",
            scene_path.display()
        );
        for surface in &scene.surfaces {
            if surface.space == plaque_forge::scene::SurfaceSpace::ScenePlane {
                assert!(
                    surface.tracking_bounds().is_some(),
                    "physical surface {:?} in {} has no tracking bounds",
                    surface.id,
                    scene_path.display()
                );
            }
        }
    }
}

#[test]
fn local_documentation_links_resolve() {
    let mut documents = vec![repository_root().join("README.md")];
    let mut pending = vec![repository_root().join("docs")];
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(&path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|value| value.to_str()) == Some("md") {
                documents.push(path);
            }
        }
    }

    for document in documents {
        let text = fs::read_to_string(&document).unwrap();
        let mut remainder = text.as_str();
        while let Some(start) = remainder.find("](") {
            remainder = &remainder[start + 2..];
            let Some(end) = remainder.find(')') else {
                panic!("unterminated Markdown link in {}", document.display());
            };
            let raw_target = remainder[..end].trim().trim_matches(['<', '>']);
            remainder = &remainder[end + 1..];
            let target = raw_target.split('#').next().unwrap_or_default();
            if target.is_empty()
                || target.starts_with('#')
                || target.contains("://")
                || target.starts_with("mailto:")
            {
                continue;
            }
            let resolved = document.parent().unwrap().join(target);
            assert!(
                resolved.exists(),
                "{} links to missing local target {raw_target:?}",
                document.display()
            );
        }
    }
}
