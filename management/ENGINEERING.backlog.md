# Planned

## EN.Hom.02-001 -- Close contract debt for the 11 uncontracted video assets
1. Audit the 11 video assets currently lacking `contract.toml` in `assets/homologation/`:
   - `16_9_holographic_datacenter_static_plaque`
   - `16_9_plaqueless_mountain_top_night`
   - `16_9_plaqueless_swamp`
   - `16_9_swamp_iron_plaque`
   - `16_9_swamp_wooden_plaque_foreground_vines_and_lizard`
   - `9_16_background_ogre_dear`
   - `9_16_lonely_ogre_holographic_static_plaque`
   - `9_16_plaqueless_datacenter_lab`
   - `9_16_plaqueless_neon_datacenter_ground_hole`
   - `9_16_scrappy_datacenter_holographic_plaque`
   - `9_16_swamp_wooden_plaque`
2. Run baseline renders with standard text and policy styles.
3. Review rendered outputs, confirm visual quality, and generate formal `contract.toml` files with empirical tolerances for `source_preservation` and `title_visibility`.
4. Add all newly contracted assets to the automated full regression suite, preventing future silent degradation.
   ==> Team. Planned: 2026-09-09;


# Started

## ER.Trk.02-001 -- Enhance trajectory Kalman smoothing under erratic camera pans
1. Test trajectory stability on `16_9_mountain_top_day_hummingbird_cloudy_plaque` and `moving-holographic-plaque`.
2. Tune velocity process noise covariance in `src/surface.rs` to better balance jitter suppression with rapid camera motion responsiveness.
3. Ensure tracking homographies remain continuous without single-frame spatial jumps.
   ==> Team. Planned: 2026-09-08; Started: 2026-09-09;


# "In Code Review"


# "Integrated"


# QA


# Merged


# Rolled Out

## EF.Seg.05-001 -- Implement temporal boundary smoothing to eliminate occluder edge chatter
1. Resolve edge flickering on thin occluder boundaries (spider legs, vines, chains) across consecutive video frames.
2. In `tools/segmentation_worker.py`, implement an optical-flow or exponential moving average temporal smoothing filter for alpha boundary transitions.
3. Verify that boundary chatter is eliminated on `16_9_dungeon_spider_iron_plaque` without causing motion lag or ghosting.
   ==> Team. Planned: 2026-09-09; Started: 2026-09-09; Merged: 2026-09-10; Rolled Out: 2026-09-10;

## EN.Tst.03-001 -- Generate visual diff diagnostic artifacts on contract failure
1. When `plaque-forge homologate` reports a violation of `source_preservation` or `title_visibility`, output side-by-side visual diff images.
2. Highlight violating pixels in bright magenta with bounding coordinates and frame timestamps in `output/regressions/`.
3. Provide immediate visual feedback for engineers diagnosing regression causes.
   ==> Team. Planned: 2026-09-09; Started: 2026-09-09; Merged: 2026-09-09; Rolled Out: 2026-09-09;

## EN.Seg.04-001 -- Automated multi-asset regression diffing and model bake-off harness
1. Enhance `scripts/bakeoff_segmentation_matrix.sh` and `tools/compare_segmentation_outputs.py` to evaluate candidate models across all representative scenes before promotion.
2. Implement automated computation of Intersection-over-Union (mIoU) and boundary drift against frozen golden mask references.
3. Establish a pre-promotion gate script that blocks model upgrades if any existing homologated asset suffers mask degradation.
   ==> Team. Planned: 2026-09-09; Started: 2026-09-09; Merged: 2026-09-09; Rolled Out: 2026-09-09;

## EF.Seg.03-001 -- Isolate model versions, checkpoints, and hyperparameters per scene
1. Investigate cross-scene regressions caused by shared segmentation defaults when tuning models for difficult assets.
2. Extend `scene.toml` schema to allow scene-specific model selection (e.g. `sam2.1-hiera-small` vs `sam2.1-hiera-large`), prompt definitions, and post-processing kernels.
3. Update `src/segmentation_strategy.rs` and `tools/segmentation_worker.py` to strictly scope parameters to the active scene request.
4. Verify that tuning hyperparameters for `16_9_dungeon_spider_iron_plaque` causes zero mask or byte divergence on `16_9_swamp_wooden_plaque_foreground_vines_and_lizard`.
   ==> Team. Planned: 2026-09-09; Started: 2026-09-09; Merged: 2026-09-09; Rolled Out: 2026-09-09;

## EN.Arch.01-001 -- Modular pipeline architecture separation
1. Restructure codebase into clean domain pipelines: `analyze`, `render`, `segmentation`, `homologation`, `media`, and `verify`.
2. Centralize application entry points in `src/application.rs` with CLI as a thin adapter.
   ==> Team. Planned: 2026-05-01; Started: 2026-05-03; Merged: 2026-05-18; Rolled Out: 2026-05-18;

## EN.Arch.03-001 -- Lease-held atomic file staging
1. Implement `src/staged_output.rs` providing RAII lease locks and atomic temporary-to-destination replacement.
2. Ensure pipeline cancellation or process panic leaves no half-written video or cache files.
   ==> Team. Planned: 2026-05-15; Started: 2026-05-18; Merged: 2026-05-24; Rolled Out: 2026-05-24;

## EN.Seg.01-001 -- JSON-lines IPC protocol for Python segmentation worker
1. Define `plaque-forge.segmentation-request/2` and `plaque-forge.segmentation-result/2` protocols.
2. Isolate Python, PyTorch, and SAM2 runtime within external worker subprocesses.
   ==> Team. Planned: 2026-05-20; Started: 2026-05-25; Merged: 2026-06-08; Rolled Out: 2026-06-08;

## EN.Seg.02-001 -- SAM 2.1 model commit hash pinning
1. Create `tools/segmentation_runtime.py` specifying exact commit revisions for `sam2.1-hiera-small` and `sam2.1-hiera-large`.
2. Enforce offline cache loading without unpinned Hugging Face Hub queries.
   ==> Team. Planned: 2026-06-05; Started: 2026-06-10; Merged: 2026-06-20; Rolled Out: 2026-06-20;

## EN.Hom.01-001 -- Dual-witness contract runner
1. Implement `plaque-forge homologate` asserting `source_preservation` and `title_visibility` against `contract.toml`.
2. Support configurable metric thresholds, frame sampling, and structured JSON reporting.
   ==> Team. Planned: 2026-06-15; Started: 2026-06-20; Merged: 2026-07-05; Rolled Out: 2026-07-05;

## EN.Hom.03-001 -- Fast continuous homologation sentinels in CI
1. Implement `scripts/check_homologated_assets.sh` running 5 diverse capability sentinels.
2. Wire sentinel suite into automated pull-request validation gates.
   ==> Team. Planned: 2026-07-01; Started: 2026-07-06; Merged: 2026-07-16; Rolled Out: 2026-07-16;

## EN.Rnd.01-001 -- Linear-light color pipeline and typography rasterization
1. Implement linear RGB color math in `src/color.rs` and `src/render/effects.rs`.
2. Integrate `cosmic-text` for line wrapping and anti-aliased font rendering.
   ==> Team. Planned: 2026-06-01; Started: 2026-06-05; Merged: 2026-06-18; Rolled Out: 2026-06-18;
