use std::{fs, path::Path};

use plaque_forge::scene::{LayerRole, Scene, SurfaceSpace};

fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn scenes_with_foreground_objects_declare_foreground_layers() {
    let assets = repository_root().join("assets/scenes");

    // Scenes known to have foreground occluding objects passing over the title surface
    let scenes_with_foreground = [
        "swamp-wooden-plaque-with-foreground-objects",
        "16_9_dungeon_spider_iron_plaque",
    ];

    for scene_name in scenes_with_foreground {
        let scene_path = assets.join(scene_name).join("scene.toml");
        assert!(
            scene_path.is_file(),
            "scene manifest missing: {}",
            scene_path.display()
        );

        let scene = Scene::load(&scene_path)
            .unwrap_or_else(|error| panic!("failed to load {}: {error:#}", scene_path.display()));

        let foreground_layers: Vec<_> = scene
            .layers
            .iter()
            .filter(|layer| layer.role == LayerRole::Foreground)
            .collect();

        assert!(
            !foreground_layers.is_empty(),
            "Scene {} contains foreground occluding objects in video but declares 0 foreground layers in scene.toml",
            scene_name
        );

        for layer in &foreground_layers {
            assert!(
                layer.in_front_of.is_some(),
                "foreground layer {} in {} must declare in_front_of",
                layer.id,
                scene_name
            );
            assert!(
                !layer.prompts.is_empty() || layer.artifact.is_some(),
                "foreground layer {} in {} must have prompts or artifact specified",
                layer.id,
                scene_name
            );
        }
    }
}

#[test]
fn analyzed_foreground_layers_have_no_frame_dropouts() {
    let analysis_root = repository_root().join("assets/analysis");
    if !analysis_root.is_dir() {
        return;
    }

    let scenes_with_active_occlusion = [
        "16_9_dungeon_spider_iron_plaque",
        "swamp-wooden-plaque-with-foreground-objects",
    ];

    for scene_name in scenes_with_active_occlusion {
        let scene_analysis = analysis_root.join(scene_name);
        let manifest_path = scene_analysis.join("manifest.toml");
        if !manifest_path.is_file() {
            continue;
        }

        let layers_dir = scene_analysis.join("layers");
        if !layers_dir.is_dir() {
            continue;
        }

        for entry in fs::read_dir(&layers_dir).unwrap() {
            let entry = entry.unwrap();
            let layer_dir = entry.path();
            if !layer_dir.is_dir() {
                continue;
            }

            let mut total_frames = 0;
            let mut empty_frames = Vec::new();

            for frame_entry in fs::read_dir(&layer_dir).unwrap() {
                let frame_entry = frame_entry.unwrap();
                let path = frame_entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("png") {
                    continue;
                }

                total_frames += 1;
                let img = image::open(&path)
                    .unwrap_or_else(|err| panic!("failed to open mask {}: {err}", path.display()));

                let non_zero_count = match img {
                    image::DynamicImage::ImageLuma8(gray) => {
                        gray.pixels().filter(|p| p.0[0] > 0).count()
                    }
                    image::DynamicImage::ImageLuma16(gray16) => {
                        gray16.pixels().filter(|p| p.0[0] > 0).count()
                    }
                    other => other.to_luma8().pixels().filter(|p| p.0[0] > 0).count(),
                };

                if non_zero_count == 0 {
                    empty_frames.push(path.file_name().unwrap().to_string_lossy().into_owned());
                }
            }

            assert!(
                empty_frames.is_empty(),
                "Layer {} in {} has {} empty mask frame dropouts out of {} frames: {:?}",
                layer_dir.display(),
                scene_name,
                empty_frames.len(),
                total_frames,
                empty_frames
            );
        }
    }
}

#[test]
fn every_scene_has_valid_surfaces_and_sources() {
    let assets = repository_root().join("assets/scenes");
    if !assets.is_dir() {
        return;
    }

    for entry in fs::read_dir(&assets).unwrap() {
        let entry = entry.unwrap();
        let scene_dir = entry.path();
        if !scene_dir.is_dir() {
            continue;
        }
        let scene_path = scene_dir.join("scene.toml");
        if !scene_path.is_file() {
            continue;
        }

        let scene = Scene::load(&scene_path)
            .unwrap_or_else(|error| panic!("invalid scene at {}: {error:#}", scene_path.display()));

        assert!(
            !scene.surfaces.is_empty(),
            "{} has no surfaces",
            scene_path.display()
        );

        for surface in &scene.surfaces {
            if surface.space == SurfaceSpace::ScenePlane {
                assert!(
                    surface.tracking_bounds().is_some(),
                    "scene plane surface {} in {} has no tracking bounds",
                    surface.id,
                    scene_path.display()
                );
            }
        }
    }
}
