# Project Risks

Risks remain active until closed. Each risk links to related requirements, work items, decisions, and mitigation strategies.


## Open Risks

### RISK-0001 -- Cross-Asset Regression When Updating ML Models or Segmentation Heuristics

Status: Closed (Mitigated)
Closed: 2026-09-10
Owner: Engineering Lead
Related: `P.AstNrg.01`, `E.Seg.03`, `EF.Seg.03-001`, `BUG-0001`, `DEC-0007`

Risk:
Modifying vision foundation models (e.g. SAM 2 checkpoints), prompt definitions, or post-processing filters to resolve an occlusion defect on one challenging video inadvertently degrades segmentation boundaries, introduces edge chatter, or drops frames on previously approved video assets.

Impact:
Loss of engineering velocity, erosion of visual quality on delivered assets, and frustration from whack-a-mole regression cycles.

Mitigation:
1. Isolate model selection and hyperparameters per scene in `scene.toml` (`EF.Seg.03-001`, Rolled Out).
2. Establish frozen golden masks and require an automated multi-scene bake-off (`scripts/bakeoff_segmentation_matrix.sh`) before any global model change is merged (`EN.Seg.04-001`, Rolled Out).
3. Expand automated dual-witness contracts to cover all 19 video assets (`EN.Hom.02-001`, Rolled Out).


### RISK-0002 -- Homologation Contract Debt (11 Uncontracted Video Scenes)

Status: Closed (Mitigated)
Closed: 2026-09-10
Owner: Homologation Lead
Related: `E.Hom.02`, `EN.Hom.02-001`

Risk:
Currently, only 8 of the 19 video assets in `assets/scenes/` possess an authoritative `contract.toml`. The remaining 11 assets have no automated visual quality or regression assertions.

Impact:
Refactorings or dependency updates can break 58% of the project's video catalog without triggering a single automated failure.

Mitigation:
Executed `EN.Hom.02-001`: Generated candidate renders, conducted human review, and authored authoritative `contract.toml` files for all 11 uncontracted scenes. All 19 assets are now verified in `tests/homologation_contracts.rs`.


### RISK-0003 -- Continuous Integration Coverage Blindspot (5-Sentinel Subset)

Status: Open
Owner: Operations Lead
Related: `O.CiCd.01`, `O.CiCd.02`, `ON.CiCd.02-001`

Risk:
To maintain fast pull-request feedback (<5 min), CI executes only 5 sentinel assets (`ci = true` in `scripts/check_homologated_assets.sh`). Regressions on the remaining 14 assets can merge into `main` undetected until manual audits.

Impact:
Subtle visual regressions accumulate in `main` between release cycles.

Mitigation:
Deploy a scheduled nightly CI job on a dedicated GPU worker (`ON.CiCd.02-001`) that runs `scripts/run_homologation_matrix.sh` across all 19 scenes, notifying the team of any regression within 24 hours.


### RISK-0004 -- Hardware Floating-Point & Precision Divergence Across Devices

Status: Closed (Mitigated)
Closed: 2026-09-10
Owner: Infrastructure Lead
Related: `O.HwEnv.02`, `ON.HwEnv.02-001`

Risk:
Segmentation models running on NVIDIA GPUs (with TF32 enabled), Intel XPUs (BF16), and CPUs (FP32) compute slightly different logit tensors, leading to 1-2 pixel mask boundary variations on subtle gradients.

Impact:
A contract that passes on a developer's GPU machine may fail on a CPU CI runner or vice-versa.

Mitigation:
Enforced `--precision fp32` determinism flags in `tools/segmentation_worker.py` and `tools/segmentation-worker` (`ON.HwEnv.02-001`), disabling TF32 math on CUDA matmul and cuDNN, forcing cuDNN deterministic algorithms, setting `CUBLAS_WORKSPACE_CONFIG=:4096:8`, and applying `torch.use_deterministic_algorithms(True, warn_only=True)`. Verified via `scripts/compare_segmentation_devices.sh` and unit tests in `tools/test_segmentation_worker_quality.py`.


### RISK-0005 -- Disk Exhaustion from Video Render Artifacts and Analysis Caches

Status: Open
Owner: Operations
Related: `O.AstSto.02`, `ON.AstSto.02-001`

Risk:
High-resolution HEVC video encodes, uncompressed frame caches, and diagnostic image dumps can rapidly consume hundreds of gigabytes of disk space on CI agents and developer workstations.

Impact:
Failed CI builds due to disk-full errors; degraded build cache performance.

Mitigation:
Incorporate automated cleanup via `scripts/cleanup_work.sh` into CI post-run steps, retaining only structured JSON summaries and regression diff images.
