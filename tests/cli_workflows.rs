mod support;

use std::collections::BTreeMap;
use std::fs;
use std::process::Command;

use support::repository_root;

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

/// The sample-video publisher must assign every bundled asset exactly one style:
/// real plaque surfaces carry the golden shine, everything else (plaque-less and
/// background videos) carries the classic glow.
#[test]
fn sample_video_plan_assigns_every_asset_one_declared_style() {
    let root = repository_root();

    let output = Command::new("bash")
        .arg(root.join("scripts/render_sample_videos.sh"))
        .arg("--print-plan")
        .current_dir(root)
        .output()
        .expect("failed to execute render_sample_videos.sh --print-plan");
    assert!(
        output.status.success(),
        "--print-plan failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut plan: BTreeMap<String, String> = BTreeMap::new();
    for line in String::from_utf8(output.stdout).unwrap().lines() {
        let (style, stem) = line
            .split_once('\t')
            .unwrap_or_else(|| panic!("plan line {line:?} is not a <style>\\t<stem> pair"));
        assert!(
            plan.insert(stem.to_string(), style.to_string()).is_none(),
            "asset stem {stem} planned more than once"
        );
    }
    assert!(!plan.is_empty(), "plan is empty");

    let shipped_stems: Vec<String> = fs::read_dir(root.join("assets"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("mp4"))
        .map(|path| path.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(!shipped_stems.is_empty(), "no bundled assets/*.mp4 found");
    for stem in &shipped_stems {
        let style = plan
            .get(stem)
            .unwrap_or_else(|| panic!("bundled asset {stem} missing from the sample-video plan"));
        let expected = if stem.contains("plaque") && !stem.contains("plaqueless") {
            "gold-shine"
        } else {
            "classic-glow"
        };
        assert_eq!(
            style, expected,
            "asset {stem} must use the {expected} style"
        );
    }
    assert_eq!(
        plan.len(),
        shipped_stems.len(),
        "plan contains stems that are not bundled assets: {:?}",
        plan.keys().collect::<Vec<_>>()
    );

    for style in plan.values() {
        assert!(
            root.join("styles").join(format!("{style}.toml")).is_file(),
            "planned style {style} has no styles/{style}.toml preset"
        );
    }
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
