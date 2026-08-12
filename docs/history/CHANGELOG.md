# Changelog

## 0.6.0

- Added first-class injected plaque surfaces for plaque-less videos. `scripts/place_plaque.sh` copies a transparent PNG, proposes a quiet placement, writes a preview/refinement, and lets normal analysis/rendering handle the result.
- Added injected-surface cache provenance: plaque image content, placement/writable intent, and motion policy participate in cache freshness while title/style changes remain analysis-independent.
- Added `auto`, `screen`, and `scene` motion policies for injected surfaces; auto scene anchoring falls back safely to screen-fixed placement when evidence is insufficient.
- Preserved automatic foreground analysis for injected plaques so moving scene objects can be restored in front of the plaque/title; authored segmentation remains available for ambiguous/static foregrounds.
- Fixed loop closure so inability to seek-decode the nominal final frame searches backward and disables only the optional loop heuristic instead of aborting tracking.
- Added a structureless-surface path for broad, screen-stationary writing regions such as soft clouds and dark/circular graphic canvases, avoiding an invalid requirement for stable internal texture.
- Added broad-arc proposals and changed automatic selection to prefer the **largest independently plausible writing-surface hypothesis**, preventing small high-contrast props such as magnifying glasses from stealing the intended plaque.
- Added explicit ML/Python observability: skip/cache/launch/PID/exit messages, persistent run history under `/tmp/plaque-forge-python/worker-runs.jsonl`, `scripts/ml_status.sh`, and `--force-ml` for intentional ML cache regeneration.
- Kept Python bytecode/check artifacts out of the repository by routing them through temporary storage.
- Bumped the analyzer cache compatibility identifier to `analysis-v6` so v0.5 caches affected by candidate-selection/tracking semantics are not silently reused.

## 0.5.0

- Generalized human writing-surface intent beyond rectangles with rounded-rectangle, ellipse/circle, polygon, and arbitrary-mask declarations while retaining rectangular planar tracking internally.
- Broadened automatic candidate proposals to consider oval/large surfaces and bright low-saturation surfaces such as cloud plaques; ellipse-border evidence now complements rectangular border evidence.
- Fixed loop detection so failed/empty OpenCV frame decodes are reported instead of reaching `cvtColor` with an empty image.
- Made `artistic` typography fitting the default and changed it to use the largest safe size after choosing the best line composition.
- Strengthened the default/classic glow so it is visibly intentional.
- Added linear-gradient and procedural gold/bronze materials, extrusion, bevel, animated pulse, and moving shine without adding production dependencies.
- Added `styles/bronze-relief.toml`, `styles/gold-shine.toml`, and `styles/neon-pulse.toml`, plus concise `--style NAME` preset selection in the high-level render scripts.
- Made the high-level analysis script validate/reuse current caches, automatically materialize missing prompted ML layers, and keep the entire optional Python/ML runtime under `/tmp/plaque-forge-python`.
- Kept the analyzer cache compatibility identifier unchanged because existing cache files retain the same meaning/format; new refinement semantics trigger rebuilds through refinement provenance instead of unrelated source-version churn.

## 0.4.0

- Split text paint/mask effects out of typography shaping and added a versioned TOML style-file boundary.
- Added drop shadows while preserving the existing direct stroke/glow/fill controls.
- Improved stroke morphology from square dilation to circular dilation while keeping glow behavior compatible.
- Added opt-in `--fit artistic`, which evaluates bounded word-boundary line layouts and records the selected line breaks.
- Added render provenance for title text, font path/hash, resolved style, and resolved line layout.
- Added `plaque-forge review` plus `scripts/review_assets.sh` for human-oriented analysis, typography, and verification triage.
- Documented the future text pipeline around reusable prepared typography, mask effects, materials, deterministic animation, and plaque-surface effects.
- Documented a human-refinement v2 direction that separates sparse human intent from dense generated tracks and masks.
- Centralized generic streaming SHA-256 provenance hashing outside video/refinement layers.


## 0.3.1

- Replaced whole-source `build.rs` hashing with explicit analysis-cache compatibility versioning.
- Reduced the root README to a runnable quick start and moved detailed concepts into `docs/`.
- Added glossary, architecture, workflow, and filesystem-safety documentation.
- Added high-level Bash helpers for analysis-cache generation and optional object segmentation.
- Kept Python optional and behind the segmentation-worker boundary; normal code checks no longer require it.
- Removed generated Python bytecode and a duplicated worker statement.
- Preserved production render manifests instead of deleting them with temporary render staging.

## 0.3.0

- Added automatic plaque detection, root-anchored adaptive tracking, structural locking, smoothing, and occlusion recovery.
- Added irregular-mask typography fitting and strict scene, tracking, temporal, occlusion, and loop verification.
- Added portable refinement manifests for plaque geometry, motion constraints, soft foreground alpha, writing surfaces, and shadows.
- Added optional SAM 2, Cutie, ViTMatte, and MatAnyone2 segmentation through an isolated Python worker.
- Validated automatic rendering on three holographic plaques and refined rendering on the rusty chain and two swamp plaques.
