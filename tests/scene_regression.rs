mod support;

use std::fs;

use plaque_forge::scene::{LayerRole, Scene, SurfaceSpace};

use support::repository_root;

#[test]
fn declared_foreground_layers_have_explicit_depth_and_evidence() {
    let assets = repository_root().join("assets/scenes");
    assert!(
        assets.is_dir(),
        "required scene regression fixtures are missing: {}",
        assets.display()
    );

    let mut scenes = 0usize;
    let mut foreground_layers = 0usize;
    for entry in fs::read_dir(&assets).unwrap() {
        let entry = entry.unwrap();
        let scene_path = entry.path().join("scene.toml");
        if !scene_path.is_file() {
            continue;
        }
        scenes += 1;
        let scene = Scene::load(&scene_path)
            .unwrap_or_else(|error| panic!("failed to load {}: {error:#}", scene_path.display()));
        for layer in scene
            .layers
            .iter()
            .filter(|layer| layer.role == LayerRole::Foreground)
        {
            foreground_layers += 1;
            assert!(
                layer.in_front_of.is_some(),
                "foreground layer {} in {} must declare in_front_of",
                layer.id,
                scene_path.display()
            );
            assert!(
                !layer.prompts.is_empty() || layer.artifact.is_some(),
                "foreground layer {} in {} must have prompts or an artifact",
                layer.id,
                scene_path.display()
            );
        }
    }

    assert!(scenes > 0, "no scene regression fixtures were exercised");
    assert!(
        foreground_layers > 0,
        "no foreground-layer regression fixtures were exercised"
    );
}

#[test]
fn analyzed_foreground_layers_have_no_frame_dropouts() {
    let analysis_root = repository_root().join("assets/analysis");
    assert!(
        analysis_root.is_dir(),
        "required generated-analysis regression fixtures are missing: {}",
        analysis_root.display()
    );

    let scenes_with_active_occlusion = [
        "16_9_dungeon_spider_iron_plaque",
        "swamp-wooden-plaque-with-foreground-objects",
    ];

    for scene_name in scenes_with_active_occlusion {
        let scene_analysis = analysis_root.join(scene_name);
        let manifest_path = scene_analysis.join("manifest.toml");
        assert!(
            manifest_path.is_file(),
            "required analysis manifest is missing for regression scene {scene_name}: {}",
            manifest_path.display()
        );

        let layers_dir = scene_analysis.join("layers");
        assert!(
            layers_dir.is_dir(),
            "required foreground-layer fixtures are missing for regression scene {scene_name}: {}",
            layers_dir.display()
        );

        let mut tested_layers = 0usize;
        for entry in fs::read_dir(&layers_dir).unwrap() {
            let entry = entry.unwrap();
            let layer_dir = entry.path();
            if !layer_dir.is_dir() {
                continue;
            }
            tested_layers += 1;

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
                total_frames > 0,
                "Layer {} in {} contains no PNG mask frames",
                layer_dir.display(),
                scene_name
            );
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

        assert!(
            tested_layers > 0,
            "no foreground layers were exercised for regression scene {scene_name}"
        );
    }
}

#[test]
fn every_scene_has_valid_surfaces_and_sources() {
    let assets = repository_root().join("assets/scenes");
    assert!(
        assets.is_dir(),
        "required scene fixtures are missing: {}",
        assets.display()
    );

    let mut scenes = 0usize;
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
        scenes += 1;

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

    assert!(scenes > 0, "no scene fixtures were exercised");
}
