# Traceability

Traceability connects requirements, work items, implementation, and verification evidence. Only parsed rows in the single canonical `## Current Links` table supply authoritative management evidence.


## Current Links

| Requirement | Work Item | State | Evidence |
| --- | --- | --- | --- |
| `P.AugMsn.01` | `PN.AugMsn.01-001` | Rolled Out | [src/lib.rs](file:///root/opencode/plaque-forge/src/lib.rs), [src/application.rs](file:///root/opencode/plaque-forge/src/application.rs); [tests/cli_workflows.rs](file:///root/opencode/plaque-forge/tests/cli_workflows.rs) |
| `P.VisFid.01` | `PN.VisFid.01-001` | Rolled Out | [src/render/effects.rs](file:///root/opencode/plaque-forge/src/render/effects.rs); [tests/rendering_subsystem.rs](file:///root/opencode/plaque-forge/tests/rendering_subsystem.rs) |
| `P.VisFid.01` | `PR.VisFid.01-001` | Started | [src/render/effects.rs](file:///root/opencode/plaque-forge/src/render/effects.rs); active specular sweep review on swamp wooden plaque |
| `P.OccPrs.01` | `PN.OccPrs.01-001` | Rolled Out | [src/layers.rs](file:///root/opencode/plaque-forge/src/layers.rs), [src/render/layers.rs](file:///root/opencode/plaque-forge/src/render/layers.rs); [tests/scene_regression.rs](file:///root/opencode/plaque-forge/tests/scene_regression.rs) |
| `P.TypoLay.01` | `PN.TypoLay.01-001` | Rolled Out | [src/render/typography.rs](file:///root/opencode/plaque-forge/src/render/typography.rs); [tests/rendering_subsystem.rs](file:///root/opencode/plaque-forge/tests/rendering_subsystem.rs) |
| `P.VidFmt.01` | `PN.VidFmt.01-001` | Rolled Out | [src/video.rs](file:///root/opencode/plaque-forge/src/video.rs); 16:9 and 9:16 scenes in [assets/scenes/](file:///root/opencode/plaque-forge/assets/scenes) |
| `P.AstNrg.01` | `PN.AstNrg.01-001` | Rolled Out | [assets/homologation/](file:///root/opencode/plaque-forge/assets/homologation); [tests/homologation_contracts.rs](file:///root/opencode/plaque-forge/tests/homologation_contracts.rs) |
| `P.AstNrg.03` | `PN.AstNrg.03-001` | Rolled Out | [src/homologation.rs](file:///root/opencode/plaque-forge/src/homologation.rs); [tests/homologation_contracts.rs](file:///root/opencode/plaque-forge/tests/homologation_contracts.rs) |
| `E.Arch.01` | `EN.Arch.01-001` | Rolled Out | [src/application.rs](file:///root/opencode/plaque-forge/src/application.rs), [src/cli.rs](file:///root/opencode/plaque-forge/src/cli.rs); [tests/application_api.rs](file:///root/opencode/plaque-forge/tests/application_api.rs) |
| `E.Arch.03` | `EN.Arch.03-001` | Rolled Out | [src/staged_output.rs](file:///root/opencode/plaque-forge/src/staged_output.rs); [tests/application_api.rs](file:///root/opencode/plaque-forge/tests/application_api.rs) |
| `E.Trk.02` | `ER.Trk.02-001` | Started | [src/surface.rs](file:///root/opencode/plaque-forge/src/surface.rs); [tests/tracking_subsystem.rs](file:///root/opencode/plaque-forge/tests/tracking_subsystem.rs) |
| `E.Seg.01` | `EN.Seg.01-001` | Rolled Out | [src/segmentation.rs](file:///root/opencode/plaque-forge/src/segmentation.rs), [tools/segmentation_worker.py](file:///root/opencode/plaque-forge/tools/segmentation_worker.py); [tools/test_segmentation_service.py](file:///root/opencode/plaque-forge/tools/test_segmentation_service.py) |
| `E.Seg.02` | `EN.Seg.02-001` | Rolled Out | [tools/segmentation_runtime.py](file:///root/opencode/plaque-forge/tools/segmentation_runtime.py); [tools/test_segmentation_runtime.py](file:///root/opencode/plaque-forge/tools/test_segmentation_runtime.py) |
| `E.Seg.03` | `EF.Seg.03-001` | Rolled Out | [src/scene.rs](file:///root/opencode/plaque-forge/src/scene.rs), [src/segmentation.rs](file:///root/opencode/plaque-forge/src/segmentation.rs), [tools/segmentation_worker.py](file:///root/opencode/plaque-forge/tools/segmentation_worker.py); [scripts/check_homologated_assets.sh](file:///root/opencode/plaque-forge/scripts/check_homologated_assets.sh) |
| `E.Seg.04` | `EN.Seg.04-001` | Rolled Out | [tools/compare_segmentation_outputs.py](file:///root/opencode/plaque-forge/tools/compare_segmentation_outputs.py), [scripts/bakeoff_segmentation_matrix.sh](file:///root/opencode/plaque-forge/scripts/bakeoff_segmentation_matrix.sh), [scripts/verify_segmentation_non_regression.sh](file:///root/opencode/plaque-forge/scripts/verify_segmentation_non_regression.sh); [tools/test_compare_segmentation_outputs.py](file:///root/opencode/plaque-forge/tools/test_compare_segmentation_outputs.py) |
| `E.Seg.05` | `EF.Seg.05-001` | Rolled Out | [src/scene.rs](file:///root/opencode/plaque-forge/src/scene.rs), [src/segmentation.rs](file:///root/opencode/plaque-forge/src/segmentation.rs), [tools/segmentation_worker.py](file:///root/opencode/plaque-forge/tools/segmentation_worker.py); [tools/test_segmentation_worker_quality.py](file:///root/opencode/plaque-forge/tools/test_segmentation_worker_quality.py) |
| `E.Hom.01` | `EN.Hom.01-001` | Rolled Out | [src/homologation.rs](file:///root/opencode/plaque-forge/src/homologation.rs); [tests/homologation_contracts.rs](file:///root/opencode/plaque-forge/tests/homologation_contracts.rs) |
| `E.Hom.02` | `EN.Hom.02-001` | Rolled Out | [assets/homologation/](file:///root/opencode/plaque-forge/assets/homologation) (all 19 contracts homologated); [tests/homologation_contracts.rs](file:///root/opencode/plaque-forge/tests/homologation_contracts.rs) |
| `E.Hom.03` | `EN.Hom.03-001` | Rolled Out | [scripts/check_homologated_assets.sh](file:///root/opencode/plaque-forge/scripts/check_homologated_assets.sh); CI sentinel suite |
| `E.Tst.03` | `EN.Tst.03-001` | Rolled Out | [src/homologation.rs](file:///root/opencode/plaque-forge/src/homologation.rs); [src/homologation.rs](file:///root/opencode/plaque-forge/src/homologation.rs) (unit tests `failed_witness_diagnostics_are_self_contained`, `failed_title_visibility_diagnostics_highlight_unmodified_pixels`) |
| `E.Rnd.01` | `EN.Rnd.01-001` | Rolled Out | [src/color.rs](file:///root/opencode/plaque-forge/src/color.rs), [src/render/typography.rs](file:///root/opencode/plaque-forge/src/render/typography.rs); [tests/rendering_subsystem.rs](file:///root/opencode/plaque-forge/tests/rendering_subsystem.rs) |
| `O.CiCd.01` | `ON.CiCd.01-001` | Rolled Out | [scripts/check_homologated_assets.sh](file:///root/opencode/plaque-forge/scripts/check_homologated_assets.sh); passing 5 sentinels |
| `O.CiCd.02` | `ON.CiCd.02-001` | Planned | [scripts/run_homologation_matrix.sh](file:///root/opencode/plaque-forge/scripts/run_homologation_matrix.sh); scheduled nightly GPU workflow proposal |
| `O.PyEnv.01` | `ON.PyEnv.01-001` | Rolled Out | [scripts/setup_segmentation.sh](file:///root/opencode/plaque-forge/scripts/setup_segmentation.sh); [scripts/check_segmentation.sh](file:///root/opencode/plaque-forge/scripts/check_segmentation.sh) |
| `O.PyEnv.01` | `OR.PyEnv.01-001` | Started | [scripts/setup_segmentation.sh](file:///root/opencode/plaque-forge/scripts/setup_segmentation.sh); generating pinned lockfile |
| `O.AstSto.01` | `ON.AstSto.01-001` | Rolled Out | [scripts/check_analysis_cache.sh](file:///root/opencode/plaque-forge/scripts/check_analysis_cache.sh); verified 19 analysis caches |
| `O.RelPkg.01` | `ON.RelPkg.01-001` | Rolled Out | [Cargo.toml](file:///root/opencode/plaque-forge/Cargo.toml), [src/media/bundled.rs](file:///root/opencode/plaque-forge/src/media/bundled.rs); [tests/embedded_media.rs](file:///root/opencode/plaque-forge/tests/embedded_media.rs) |


## Unmapped Requirement Areas

The following requirement areas represent acknowledged forward capabilities whose work mapping or verification evidence is planned:
1. `P.VisFid.03` -- Dynamic scene illumination adaptation.
2. `P.OccPrs.03` -- Multi-layer shadow casting and complex depth layering.
3. `E.Trk.01` -- Extreme motion blur homography recovery.
4. `O.HwEnv.02` -- Strict cross-device FP32 determinism enforcement.


## Evidence Rules

1. Tests are evidence of verified functional behavior.
2. Homologation contracts and JSON reports are evidence of accepted visual output.
3. Automated check scripts (`check_homologated_assets.sh`, `check_analysis_cache.sh`, `check_segmentation.sh`) are evidence of subsystem integrity.
4. Code in `main` is evidence of actual current behavior.
5. Code outside `main` is evidence of behavior under development.
6. A row's requirement must exactly match its work item's governing requirement.
7. Missing evidence does not invalidate a requirement or prove that code is absent; it represents an audit gap to be closed.
