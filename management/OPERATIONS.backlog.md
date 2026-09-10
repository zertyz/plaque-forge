# Planned

## ON.CiCd.02-001 -- Configure scheduled nightly full-matrix GPU homologation workflow
1. Create a scheduled CI workflow running on a dedicated self-hosted GPU runner every night.
2. Execute `scripts/run_homologation_matrix.sh` across all 19 assets with dual-witness contract verification.
3. Automatically upload regression failure diagnostics and notify the engineering team upon threshold violations.
   ==> Ops. Planned: 2026-09-09;


# Started

## OR.PyEnv.01-001 -- Audit and pin Python virtualenv lockfile with exact package hashes
1. Review `scripts/setup_segmentation.sh` package installation commands.
2. Generate an authoritative `requirements.lock` with explicit package versions and hashes for PyTorch, torchvision, and huggingface-hub.
3. Verify clean reproducible installation in a fresh container.
   ==> Ops. Planned: 2026-09-08; Started: 2026-09-09;


# "In Code Review"


# "Integrated"


# QA


# Merged


# Rolled Out

## ON.AstSto.02-001 -- Automate disk cleanup of ephemeral MKV renders on CI runners
1. Add an automated post-run cleanup hook in CI workflows invoking `scripts/cleanup_work.sh`.
2. Ensure intermediate HEVC renders under `output/` are pruned while preserving test summary JSONs and regression diffs.
   ==> Ops. Planned: 2026-09-09; Started: 2026-09-10; Merged: 2026-09-10; Rolled Out: 2026-09-10;

## ON.HwEnv.02-001 -- Enforce strict FP32 determinism flags in Python worker CI test harness
1. In `tools/segmentation_worker.py`, enforce `torch.use_deterministic_algorithms(True)` and `torch.backends.cuda.matmul.allow_tf32 = False` during test and contract evaluation runs.
2. Verify that running on NVIDIA Ampere vs Ada Lovelace vs CPU produces identical mask thresholds for `16_9_dungeon_spider_iron_plaque`.
   ==> Ops. Planned: 2026-09-09; Started: 2026-09-10; Merged: 2026-09-10; Rolled Out: 2026-09-10;

## ON.CiCd.01-001 -- Establish fast 5-case continuous homologation sentinel gate in CI
1. Implement `scripts/check_homologated_assets.sh` executing 5 behaviorally diverse sentinel assets.
2. Assert both `source_preservation` and `title_visibility` in under 3 minutes.
   ==> Ops. Planned: 2026-07-01; Started: 2026-07-03; Merged: 2026-07-16; Rolled Out: 2026-07-16;

## ON.PyEnv.01-001 -- Automated Python environment bootstrap script
1. Provide `scripts/setup_segmentation.sh` supporting virtualenv creation, PyTorch installation, and offline model caching.
2. Include diagnostic commands in `scripts/ml_status.sh`.
   ==> Ops. Planned: 2026-06-01; Started: 2026-06-05; Merged: 2026-06-20; Rolled Out: 2026-06-20;

## ON.AstSto.01-001 -- Analysis cache verification script
1. Provide `scripts/check_analysis_cache.sh` validating manifest integrity across all 19 committed scene caches.
2. Gate pull requests against uncommitted or corrupt trajectory files.
   ==> Ops. Planned: 2026-06-15; Started: 2026-06-18; Merged: 2026-07-01; Rolled Out: 2026-07-01;

## ON.RelPkg.01-001 -- Standalone binary build profile with bundle-media support
1. Configure `Cargo.toml` release profile with fat LTO, symbol stripping, and single codegen unit.
2. Provide `bundle-media` feature flag for self-contained distribution binaries.
   ==> Ops. Planned: 2026-07-05; Started: 2026-07-10; Merged: 2026-07-22; Rolled Out: 2026-07-22;
