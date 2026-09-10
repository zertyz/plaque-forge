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

/// The sample-video publisher assigns every bundled asset exactly one style.
/// The classification rule itself lives in `scripts/render_sample_videos.sh`;
/// this contract pins it through representative stems covering every scene
/// class (wooden/iron/holographic plaques, moving plaques, foreground
/// occluders, backgrounds, plaque-less scenes, both aspect families), and
/// asserts the plan stays a bijection over the bundled assets.
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

    let shipped = Command::new("bash")
        .arg("-c")
        .arg(r#"source "$1/scripts/common.sh"; pf_asset_cases cases; printf '%s\n' "${cases[@]}""#)
        .arg("pf-asset-cases")
        .arg(root)
        .output()
        .expect("failed to enumerate bundled assets via scripts/common.sh");
    assert!(
        shipped.status.success(),
        "asset enumeration failed: {}",
        String::from_utf8_lossy(&shipped.stderr)
    );
    let shipped_stems: Vec<String> = String::from_utf8(shipped.stdout)
        .unwrap()
        .lines()
        .map(|line| line.to_string())
        .collect();
    assert!(!shipped_stems.is_empty(), "no bundled assets/*.mp4 found");

    const GOLD_SHINE_STEMS: &[&str] = &[
        "16_9_swamp_wooden_plaque",
        "16_9_dungeon_spider_iron_plaque",
        "16_9_holographic_datacenter_static_plaque",
        "16_9_scrapyard_iron_plaque_foreground_chains",
        "16_9_swamp_wooden_plaque_foreground_vines_and_lizard",
        "moving-holographic-plaque",
        "9_16_scrappy_datacenter_holographic_plaque",
        "9_16_lonely_ogre_holographic_static_plaque",
    ];
    const CLASSIC_GLOW_STEMS: &[&str] = &[
        "16_9_background_digifall",
        "9_16_background_ogre_dear",
        "16_9_plaqueless_mountain_top_night",
        "9_16_plaqueless_datacenter_lab",
    ];
    for (expected_style, stems) in [
        ("gold-shine", GOLD_SHINE_STEMS),
        ("classic-glow", CLASSIC_GLOW_STEMS),
    ] {
        for stem in stems {
            assert_eq!(
                plan.get(*stem).map(String::as_str),
                Some(expected_style),
                "bundled asset {stem} must use the {expected_style} style"
            );
        }
    }
    for stem in &shipped_stems {
        assert!(
            plan.contains_key(stem),
            "bundled asset {stem} missing from the sample-video plan"
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

/// Release staging flattens the per-style render batch into unique asset names:
/// delivery videos become "<stem>.<style>.hevc.mkv", batch-root MP4 previews
/// pass through under their published name, and render sidecars (text mask,
/// manifests, decision traces) never reach the release. The root-level previews
/// are exactly what a per-style directory walk alone would miss.
#[test]
fn stage_release_flattens_batch_and_keeps_previews() {
    let root = repository_root();
    let scratch = support::temp_root("stage-release");
    let batch = scratch.join("batch");
    let dest = scratch.join("stage");

    fs::create_dir_all(batch.join("gold-shine")).unwrap();
    fs::create_dir_all(batch.join("classic-glow")).unwrap();
    fs::write(batch.join("gold-shine/swamp.hevc.mkv"), "hevc-a").unwrap();
    fs::write(batch.join("classic-glow/digifall.hevc.mkv"), "hevc-b").unwrap();
    // Sidecars belong to the transactional render bundle, not to the release.
    fs::write(
        batch.join("gold-shine/swamp.hevc.render-manifest.json"),
        "{}",
    )
    .unwrap();
    fs::write(batch.join("swamp.gold-shine.preview.mp4"), "preview").unwrap();

    let output = Command::new("bash")
        .arg(root.join("scripts/render_sample_videos.sh"))
        .arg("--stage-release")
        .arg(&batch)
        .arg(&dest)
        .current_dir(root)
        .output()
        .expect("failed to execute render_sample_videos.sh --stage-release");
    assert!(
        output.status.success(),
        "--stage-release failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut staged: Vec<String> = fs::read_dir(&dest)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    staged.sort();
    assert_eq!(
        staged,
        vec![
            "digifall.classic-glow.hevc.mkv".to_string(),
            "swamp.gold-shine.hevc.mkv".to_string(),
            "swamp.gold-shine.preview.mp4".to_string(),
        ],
        "staged release assets mismatch"
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

#[test]
fn cleanup_work_prunes_renders_and_preserves_test_summaries() {
    let root = repository_root();
    let cleanup_script = root.join("scripts/cleanup_work.sh");
    assert!(cleanup_script.is_file(), "cleanup_work.sh missing");

    let test_output = support::temp_root("cleanup-work-test");
    let regressions = test_output.join("regressions");
    fs::create_dir_all(&regressions).expect("failed to create output/regressions");

    // Create mock intermediate render MKV
    let mkv_path = test_output.join("16_9_test_scene.hevc.mkv");
    fs::write(&mkv_path, b"mock hevc video data of significant size")
        .expect("failed to write mock mkv");

    // Create mock structured test reports and diffs that MUST be preserved
    let report_path = test_output.join("16_9_test_scene.homologation.json");
    fs::write(&report_path, b"{\"passed\": true}").expect("failed to write mock report");

    let trace_path = test_output.join("16_9_test_scene.hevc.decision-trace.json");
    fs::write(&trace_path, b"{\"trace\": []}").expect("failed to write mock trace");

    let manifest_path = test_output.join("16_9_test_scene.hevc.render-manifest.json");
    fs::write(&manifest_path, b"{\"manifest\": true}").expect("failed to write mock manifest");

    let coverage_path = test_output.join("homologation-coverage.json");
    fs::write(&coverage_path, b"{\"coverage\": 1.0}").expect("failed to write mock coverage");

    let mask_path = test_output.join("16_9_test_scene.hevc.text-mask.png");
    fs::write(&mask_path, b"\x89PNG\r\n\x1a\n").expect("failed to write mock mask");

    let diff_path = regressions.join("16_9_test_scene.diff.png");
    fs::write(&diff_path, b"\x89PNG\r\n\x1a\n").expect("failed to write mock diff");

    // 1. Dry run: should report 1 render to prune without deleting anything
    let dry_run = Command::new("bash")
        .arg(&cleanup_script)
        .arg("--prune-renders")
        .arg("--output")
        .arg(&test_output)
        .output()
        .expect("failed to execute cleanup_work.sh dry run");
    assert!(dry_run.status.success(), "dry run failed: {dry_run:?}");
    let dry_stdout = String::from_utf8_lossy(&dry_run.stdout);
    assert!(
        dry_stdout.contains("ephemeral video renders to prune: 1"),
        "unexpected dry run output: {dry_stdout}"
    );
    assert!(mkv_path.is_file(), "dry run must not delete mkv");
    assert!(report_path.is_file(), "report missing after dry run");

    // 2. Applied cleanup: deletes mkv but preserves json reports and diffs
    let applied = Command::new("bash")
        .arg(&cleanup_script)
        .arg("--yes")
        .arg("--prune-renders")
        .arg("--output")
        .arg(&test_output)
        .output()
        .expect("failed to execute cleanup_work.sh applied");
    assert!(applied.status.success(), "applied cleanup failed: {applied:?}");
    let applied_stdout = String::from_utf8_lossy(&applied.stdout);
    assert!(
        applied_stdout.contains("pruned 1 ephemeral render"),
        "unexpected applied output: {applied_stdout}"
    );

    // Assert that the heavy MKV was pruned
    assert!(!mkv_path.exists(), "intermediate mkv was not pruned");

    // Assert that all structured JSON reports, manifests, masks, and regression diffs remain intact
    assert!(report_path.is_file(), "homologation.json must be preserved");
    assert!(trace_path.is_file(), "decision-trace.json must be preserved");
    assert!(manifest_path.is_file(), "render-manifest.json must be preserved");
    assert!(coverage_path.is_file(), "homologation-coverage.json must be preserved");
    assert!(mask_path.is_file(), "text-mask.png must be preserved");
    assert!(diff_path.is_file(), "regression diff.png must be preserved");
}

#[test]
fn homologation_matrix_plan_covers_all_contracted_assets() {
    let root = repository_root();
    let matrix_script = root.join("scripts/run_homologation_matrix.sh");
    assert!(matrix_script.is_file(), "run_homologation_matrix.sh missing");

    let output = Command::new("bash")
        .arg(&matrix_script)
        .arg("--print-plan")
        .output()
        .expect("failed to execute run_homologation_matrix.sh --print-plan");
    assert!(output.status.success(), "print-plan failed: {output:?}");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(
        lines.len(),
        19,
        "expected exactly 19 contracted asset plans, got {}",
        lines.len()
    );

    // Verify key assets are present in the plan
    assert!(stdout.contains("16_9_swamp_wooden_plaque"));
    assert!(stdout.contains("16_9_dungeon_spider_iron_plaque"));
    assert!(stdout.contains("moving-holographic-plaque"));
    assert!(stdout.contains("9_16_dungeon_spider_iron_plaque"));
}
