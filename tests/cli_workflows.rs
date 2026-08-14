use std::{fs, path::Path};

fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn all_shipped_style_files_are_valid_toml() {
    let styles_dir = repository_root().join("styles");
    assert!(styles_dir.is_dir(), "styles directory missing");

    let mut count = 0;
    for entry in fs::read_dir(&styles_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
            continue;
        }

        let content = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read style {}: {err}", path.display()));

        let parsed: toml::Value = toml::from_str(&content)
            .unwrap_or_else(|err| panic!("invalid TOML in {}: {err}", path.display()));

        assert!(
            parsed.is_table(),
            "style file {} must be a TOML table",
            path.display()
        );
        count += 1;
    }

    assert!(
        count >= 4,
        "expected at least 4 shipped styles, found {count}"
    );
}

#[test]
fn scene_trajectories_round_trip_cleanly() {
    let assets = repository_root().join("assets/scenes");
    if !assets.is_dir() {
        return;
    }

    for entry in fs::read_dir(&assets).unwrap() {
        let entry = entry.unwrap();
        let scene_path = entry.path().join("scene.toml");
        if !scene_path.is_file() {
            continue;
        }

        let scene = plaque_forge::scene::Scene::load(&scene_path)
            .unwrap_or_else(|err| panic!("failed to load {}: {err:#}", scene_path.display()));

        for surface in &scene.surfaces {
            if let Some(trajectory_path) = &surface.trajectory {
                let resolved = plaque_forge::scene::resolve_relative(&scene_path, trajectory_path);
                if resolved.is_file() {
                    let trajectory = plaque_forge::scene::SurfaceTrajectory::load(&resolved)
                        .unwrap_or_else(|err| {
                            panic!("failed to load trajectory {}: {err:#}", resolved.display())
                        });
                    assert_eq!(
                        trajectory.surface,
                        surface.id,
                        "trajectory surface id mismatch in {}",
                        resolved.display()
                    );
                }
            }
        }
    }
}
