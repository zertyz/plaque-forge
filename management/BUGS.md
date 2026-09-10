# Bug Reports

This ledger separates open, unresolved reports from addressed history. A report is intake, not a confirmed defect or unit of planned work. Once validated, a report links to at least one `F`-motivation work item and remains under `## Open Reports` until that work reaches `Rolled Out`.


## Open Reports


## Addressed Reports

### BUG-0001 -- ML/Refactoring: Model adjustments and refactorings cause visual regressions on previously working video assets

Status: Addressed (Rolled Out)
Reported: 2026-09-09
Resolved: 2026-09-10
Reporter: User
Related work: `EF.Seg.03-001`, `EN.Hom.02-001`, `EN.Seg.04-001`

Report:
> We have had several issues over time with this project. The issue that hurts most is that changing the models to fix one video (and doing refactorings, adding new features) breaks other videos which were already very good.
> When parameters, prompts, or model checkpoints are adjusted to improve challenging videos, existing scenes that were previously accepted experience mask boundary drift, edge chatter, or contract violations.

Assessment:
Technical audit reveals three primary root causes:
1. **Shared Global Heuristics**: Segmentation prompts, dilation kernels, and model selection in [tools/segmentation_worker.py](file:///root/opencode/plaque-forge/tools/segmentation_worker.py) and [src/segmentation_strategy.rs](file:///root/opencode/plaque-forge/src/segmentation_strategy.rs) share global defaults rather than being strictly parameterized per-scene in `scene.toml`.
2. **Homologation Contract Debt**: 11 of 19 video assets in `assets/scenes/` lack a formal `contract.toml` in `assets/homologation/`. Regressions on those 11 videos trigger no automated test failures.
3. **CI Sentinel Coverage Gap**: Continuous integration only evaluates 5 sentinel assets ([scripts/check_homologated_assets.sh](file:///root/opencode/plaque-forge/scripts/check_homologated_assets.sh)), allowing visual regressions on the remaining 14 assets to land in `main` unnoticed.

Remediation:
- `EF.Seg.03-001` (Rolled Out): Per-scene model and parameter scoping;
- `EN.Hom.02-001` (Rolled Out): Establishing formal contracts for all 11 previously uncontracted scenes;
- `EN.Seg.04-001` (Rolled Out): Automated multi-asset regression bake-off gate before model promotion.

