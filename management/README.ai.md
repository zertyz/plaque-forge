# AI Management Operating Map

This document serves as the operational navigation guide for autonomous coding agents working in the Plaque Forge repository.


## Read Order

1. [management/MANAGEMENT.md](file:///root/opencode/plaque-forge/management/MANAGEMENT.md) for top-level governance procedures and sources of truth.
2. [management/DEFINITION_OF_READY_DONE.md](file:///root/opencode/plaque-forge/management/DEFINITION_OF_READY_DONE.md) for state gate rules and visual non-regression criteria.
3. Relevant requirement document:
   - [management/PRODUCT.md](file:///root/opencode/plaque-forge/management/PRODUCT.md) for user-facing, visual, and stylistic requirements.
   - [management/ENGINEERING.md](file:///root/opencode/plaque-forge/management/ENGINEERING.md) for architecture, tracking, segmentation, rendering, and test requirements.
   - [management/OPERATIONS.md](file:///root/opencode/plaque-forge/management/OPERATIONS.md) for execution environments, Python ML worker setup, and CI/CD pipelines.
4. [management/BUGS.md](file:///root/opencode/plaque-forge/management/BUGS.md) for open defects and reported issues (notably `BUG-0001` on cross-asset regressions).
5. Corresponding backlog file: [management/PRODUCT.backlog.md](file:///root/opencode/plaque-forge/management/PRODUCT.backlog.md), [management/ENGINEERING.backlog.md](file:///root/opencode/plaque-forge/management/ENGINEERING.backlog.md), or [management/OPERATIONS.backlog.md](file:///root/opencode/plaque-forge/management/OPERATIONS.backlog.md).
6. [management/TRACEABILITY.md](file:///root/opencode/plaque-forge/management/TRACEABILITY.md) to check existing verification evidence.
7. Supporting ledgers: [management/DECISIONS.md](file:///root/opencode/plaque-forge/management/DECISIONS.md) and [management/RISKS.md](file:///root/opencode/plaque-forge/management/RISKS.md).


## Cardinal Rule: Video Asset Non-Regression

The central operational challenge in Plaque Forge is that modifying machine learning models or segmentation heuristics to fix one video scene can easily break other video scenes that were previously accepted.

When working on models, tracking, or rendering:
1. **Never change global segmentation heuristics to tune for a single scene.** Hyperparameters, prompts, and filters must be scoped in the scene's `scene.toml`.
2. **Always execute the continuous homologation sentinel suite** before committing:
   ```bash
   ./scripts/check_homologated_assets.sh
   ```
3. **Run the segmentation bake-off** before proposing any model change:
   ```bash
   ./scripts/bakeoff_segmentation_matrix.sh
   ```
4. **Ensure analysis caches remain valid**:
   ```bash
   ./scripts/check_analysis_cache.sh
   ```


## Primary Verification Commands

- `cargo test --all-targets`: Run Rust unit and integration tests.
- `./scripts/check_analysis_cache.sh`: Validate committed analysis caches across all 19 video assets.
- `./scripts/check_segmentation.sh`: Run Python segmentation test suite (27 unit/integration tests).
- `./scripts/check_homologated_assets.sh`: Run fast continuous homologation sentinels (5 diverse assets).
- `./scripts/run_homologation_matrix.sh`: Run full 19-scene homologation matrix (pre-release gate).
- `./scripts/cleanup_work.sh`: Prune ephemeral video render outputs under `output/`.


## Agent Write Discipline

1. **Follow TDD**: Write failing tests before writing production code, adhering to [docs/engineering-policy.md](file:///root/opencode/plaque-forge/docs/engineering-policy.md).
2. **Maintain Traceability**: When advancing a work item towards `Merged`, ensure its corresponding requirement, work item ID, and concrete evidence file paths are recorded in [management/TRACEABILITY.md](file:///root/opencode/plaque-forge/management/TRACEABILITY.md).
3. **Preserve Immutability**: Never overwrite or delete prior state trails or accepted decisions; append new states or superseding decisions.
