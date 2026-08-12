# Filesystem safety

Plaque Forge treats source videos, refinements, and analysis caches as different classes of data.

## Render

`render` reads source video, analysis, and refinements. It writes only its requested output bundle: video, canonical text mask, render manifest, and optional contact sheet/diagnostics.

Both direct rendering and `scripts/render_assets.sh` build in private directories under `/tmp/plaque-forge/`. Existing output members are backed up until every new member is ready; the manifest is installed last as the bundle commit marker. Cleanup is restricted to the owned temporary stage. Rendering does not delete analysis or refinement data.

## Analyze and segment

Existing analysis and segmentation outputs are not replaced unless `--force` is explicit. New results are built under `/tmp/plaque-forge/work/`. Replacement happens only after the operation succeeds and validates its output; a same-filesystem rename is used when possible and a validated copy is used across filesystems.

The shared staged-output helper checks ownership before recursive deletion. A path outside the staging/output root is rejected.

Failed in-progress trees are deleted automatically. Analysis retains only its compact summary/diagnostics under `/tmp/plaque-forge/failures/<asset>/`, bounded to three runs and seven days. Every later Plaque Forge staging operation reaps expired/excess failure evidence across all assets. Stale work older than 24 hours is also reaped, but Linux process ownership is checked first so an unusually long active run is not mistaken for debris. Destination-side incoming/backup names are deterministic, so a later run can restore or remove them after an interrupted publication. `scripts/cleanup_work.sh --yes` safely removes legacy `assets/analysis/*.partial-*` directories, old PID-suffixed publication siblings, and obsolete completed-worker request files.

## Optional Python environment

`scripts/setup_segmentation.sh` uses `/tmp/plaque-forge-python` as an isolated environment. An existing environment is preserved by default. Recursive deletion occurs only with the explicit `--reinstall` option and only for that fixed directory.

The Python segmentation worker may remove its own temporary frame and model-cache directories after inference. It resolves both paths and rejects deletion unless the child is inside the worker output directory.

## Network access

Normal analysis, rendering, and verification do not require network access. `scripts/setup_segmentation.sh` does require network access because it installs Python packages, clones pinned model repositories, and downloads model weights.

The setup records exact source commits, package versions, model revisions, and implementation hashes in a path-free runtime manifest. Worker requests carry content identities but runtime paths are never persisted in project artifacts.

## Explicit full analysis reset

`./scripts/reset_analysis.sh --yes` is the only high-level command intended to wipe all generated scene-analysis caches. It resolves and verifies the deletion root as this repository's `assets/analysis/` and does not delete `assets/refinements`, source videos, plaque assets, `output/`, or `/tmp/plaque-forge-python`.
