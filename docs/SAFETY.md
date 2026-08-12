# Filesystem safety

Plaque Forge treats source videos, refinements, and analysis caches as different classes of data.

## Render

`render` reads source video, analysis, and refinements. It writes only its requested output video, render manifest, and optional diagnostics.

`scripts/render_assets.sh` renders into a private temporary directory under `output/` and moves completed files into place. Its cleanup trap removes only that temporary directory. It does not delete analysis or refinement data.

## Analyze and segment

Existing analysis and segmentation outputs are not replaced unless `--force` is explicit. New results are built in sibling staging directories. Replacement happens only after the operation succeeds and validates its output.

The shared staged-output helper checks ownership before recursive deletion. A path outside the staging/output root is rejected.

## Optional Python environment

`scripts/setup_segmentation.sh` uses `/tmp/plaque-forge-python` as an isolated environment. An existing environment is preserved by default. Recursive deletion occurs only with the explicit `--reinstall` option and only for that fixed directory.

The Python segmentation worker may remove its own temporary frame and model-cache directories after inference. It resolves both paths and rejects deletion unless the child is inside the worker output directory.

## Network access

Normal analysis, rendering, and verification do not require network access. `scripts/setup_segmentation.sh` does require network access because it installs Python packages, clones pinned model repositories, and downloads model weights.

## Explicit full analysis reset

`./scripts/reset_analysis.sh --yes` is the only high-level command intended to wipe all generated scene-analysis caches. It resolves and verifies the deletion root as this repository's `assets/analysis/` and does not delete `assets/refinements`, source videos, plaque assets, `output/`, or `/tmp/plaque-forge-python`.
