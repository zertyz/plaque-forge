# Changelog

## 0.8.0

- Fixed the severe artistic-fit performance regression by measuring shaped candidates
  before rasterization and composing the full text material only once; the worst bundled
  long-title fit dropped from minutes to seconds without reducing supersampling.
- Added explicit plaque occlusion policy and active depth-layer semantics, preserved
  holes in thin foreground mattes, derived visibility from in-frame geometry, and stopped
  trailing occlusions from freezing title motion.
- Made cache migration serialization-only and bumped analysis compatibility to
  `analysis-v9`; migrations can no longer attach current provenance to stale geometry.
- Added automatic Python/ML foreground scene after the Rust occlusion pass finds a useful crossing; authored prompted layers remain authoritative and automatic ML failures fall back to the Rust masks instead of destroying otherwise-useful analysis.
- Added ML configuration and automatic-foreground participation to analysis provenance/cache identity and made review reports state whether Python participated.
- Repaired writing-surface selection after the global "largest plausible rectangle" regression: preserve the strongest hypothesis, rescue clear compact plaques from broad enclosures, and use area only as a guarded escape from small high-contrast props.
- Added minimal schema-2 project intent for every bundled sample asset, including the repeatedly ambiguous circular/cloud/wood/spider surfaces and the previously validated holographic plaques.
- Renamed the pre-existing holographic plaque to the aspect-explicit **Aetherglass Aurora** family, added its `9:16` counterpart, added the **Prismwraith Reliquary** pair, and introduced a portable hash catalog.
- Added advanced production text effects/materials: chrome, holographic, fire, ice, nebula, liquid, halftone, chromatic split, trails, letterpress, flicker, wave/wobble, typewriter and deterministic dissolve, plus 13 new style presets.
- Kept the capability matrix explicit about what remains approximate or unimplemented: per-glyph arc/orbit, arbitrary external texture mapping, scramble/split-flap, real particle simulation, and physically correct engraving/protrusion.
- Restored quality-report generation as a first-class README workflow and made `review_assets.sh` build a single `output/review/index.html` over complete analyses and compact retained failure evidence.
- Added `scripts/reset_analysis.sh --yes`, constrained to generated `assets/analysis/` state; source videos, scenes, plaque assets, outputs and `/tmp/plaque-forge-python` are preserved.
- Added path-free analysis/render/verification schemas, a validated cache migration command, transactional `/tmp` staging, bounded failure retention, and a legacy-work cleanup command.
- Upgraded the worker protocol with lossless PNG frames/masks, exact alpha seed masks, pinned source/model identities, semantic cache validation, and strict Rust-side output checks.
- Corrected compositing to linear-light premultiplied alpha, hardened alpha-aware verification/provenance, and made direct render publication transactional.
- Made the source text-free limitation explicit and rejected unsupported HDR/BT.2020 compositing instead of silently producing misleading output.
- Bumped analysis cache compatibility to `analysis-v8` for the hardened analysis/ML semantics and schema 2.

## 0.7.0

- Implemented human-scene schema 2 while retaining schema-1 compatibility.
- Added sparse `[[plaques.motion]]` corrections directly in `scene.toml`; normalized coordinates are the human-oriented default and are converted to source-pixel motion constraints internally.
- Added normalized segmentation prompts (`coordinates = "normalized"`) and conversion at the external-worker boundary; legacy prompts still default to source pixels.
- Moved new dense motion exports under `artifacts/motion.toml` and new implicit prompted ML outputs under `artifacts/layers/`, with legacy artifact-path fallback.
- Simplified automatically generated scene manifests by removing commented machine-candidate dumps; alternatives now belong in diagnostics/review.
- Made `plaque-forge review` work on partial analysis directories, including failures that have only early-stage diagnostics.
- Added prioritized "Focus first" triage, a plain-text `review.txt`, candidate-alternative comparison, current-scene summary, and a browser-only coordinate click helper.
- Made `analyze_assets.sh` automatically generate human review reports whenever a quality gate fails; `review_assets.sh` falls back to the newest partial analysis when no complete cache exists.
- Kept analysis-cache compatibility unchanged: scene algorithms/cache formats are not redefined by this human-interface pass; scene semantic provenance still invalidates dependent caches when human intent changes.

## 0.6.0

- Added first-class injected plaque surfaces for plaque-less videos. `scripts/place_plaque.sh` copies a transparent PNG, proposes a quiet placement, writes a preview/scene, and lets normal analysis/rendering handle the result.
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
- Kept the analyzer cache compatibility identifier unchanged because existing cache files retain the same meaning/format; new scene semantics trigger rebuilds through scene provenance instead of unrelated source-version churn.

## 0.4.0

- Split text paint/mask effects out of typography shaping and added a versioned TOML style-file boundary.
- Added drop shadows while preserving the existing direct stroke/glow/fill controls.
- Improved stroke morphology from square dilation to circular dilation while keeping glow behavior compatible.
- Added opt-in `--fit artistic`, which evaluates bounded word-boundary line layouts and records the selected line breaks.
- Added render provenance for title text, font path/hash, resolved style, and resolved line layout.
- Added `plaque-forge review` plus `scripts/review_assets.sh` for human-oriented analysis, typography, and verification triage.
- Documented the future text pipeline around reusable prepared typography, mask effects, materials, deterministic animation, and plaque-surface effects.
- Documented a human-scene v2 direction that separates sparse human intent from dense generated tracks and masks.
- Centralized generic streaming SHA-256 provenance hashing outside video/scene layers.


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
- Added portable scene manifests for plaque geometry, motion constraints, soft foreground alpha, writing surfaces, and shadows.
- Added optional SAM 2, Cutie, ViTMatte, and MatAnyone2 segmentation through an isolated Python worker.
- Validated automatic rendering on three holographic plaques and refined rendering on the rusty chain and two swamp plaques.
