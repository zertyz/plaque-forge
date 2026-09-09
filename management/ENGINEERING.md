# Architecture & Layering "Arch"

Plaque Forge is organized into modular pipelines with clear separation of concerns, strict dependency directions, and robust replaceability per [docs/engineering-policy.md](file:///root/opencode/plaque-forge/docs/engineering-policy.md).


## 01) Modular Pipeline Separation

The codebase is partitioned into distinct subsystems:
- **Application Layer ([src/application.rs](file:///root/opencode/plaque-forge/src/application.rs))**: Interface-independent workflow orchestrator (`analyze`, `render`, `verify`, `homologate`).
- **Surface Analysis & Tracking ([src/analyze/](file:///root/opencode/plaque-forge/src/analyze), [src/analysis.rs](file:///root/opencode/plaque-forge/src/analysis.rs))**: Feature extraction, planar homography, and trajectory calculation.
- **Scene Declarations & Layering ([src/scene.rs](file:///root/opencode/plaque-forge/src/scene.rs), [src/layers.rs](file:///root/opencode/plaque-forge/src/layers.rs))**: Manifest configuration, layer ordering, and manual anchor overrides.
- **Segmentation Subsystem ([src/segmentation.rs](file:///root/opencode/plaque-forge/src/segmentation.rs), [tools/segmentation_worker.py](file:///root/opencode/plaque-forge/tools/segmentation_worker.py))**: Machine-learning foreground occluder isolation.
- **Rendering & Compositing ([src/render/](file:///root/opencode/plaque-forge/src/render))**: Typography layout ([src/render/typography.rs](file:///root/opencode/plaque-forge/src/render/typography.rs)), material shaders ([src/render/effects.rs](file:///root/opencode/plaque-forge/src/render/effects.rs)), and linear-light compositing.
- **Verification & Homologation ([src/verify/](file:///root/opencode/plaque-forge/src/verify), [src/homologation.rs](file:///root/opencode/plaque-forge/src/homologation.rs))**: Objective visual quality scorecards and executable acceptance contracts.
- **Media Catalog ([src/media/](file:///root/opencode/plaque-forge/src/media))**: Media asset discovery from filesystem or binary-embedded resources.
- **Atomic Safety & Staging ([src/staged_output.rs](file:///root/opencode/plaque-forge/src/staged_output.rs))**: Lease-held atomic staging ensuring zero output file corruption.


## 02) Dependency Inversion & Replaceability

Heavy external boundaries (video decoding, filesystem I/O, ML subprocesses) must be mediated by explicit abstractions ([src/infrastructure.rs](file:///root/opencode/plaque-forge/src/infrastructure.rs)). Unit tests must run using lightweight in-memory fakes and synthetic video frames without requiring OpenCV hardware acceleration or external model servers.


## 03) Atomic Lease-Held Staging

All rendered video outputs, analysis caches, and diagnostic reports must be staged through temporary lease-held files and atomically swapped into place upon successful completion. Crashes, panics, or pipeline cancellations must never leave partial or corrupt files in destination directories.



# Surface Analysis & Camera Tracking "Trk"

Accurate surface tracking provides the spatial foundation for plaque compositing.


## 01) Geometric Tracking Stability

Planar homography estimation ([src/geometry.rs](file:///root/opencode/plaque-forge/src/geometry.rs), [src/surface.rs](file:///root/opencode/plaque-forge/src/surface.rs)) must calculate stable 4-point projective transformations across continuous video frames using robust feature matching (FAST, ORB, RANSAC).


## 02) Trajectory Smoothing & Motion Filtering

Camera trajectories must incorporate temporal smoothing to suppress subpixel high-frequency jitter while faithfully tracking rapid pans, tilts, and perspective scale changes without spatial drift.


## 03) Tracking Determinism

Given identical video input, scene parameters, and cache data, tracking trajectory calculations must produce deterministic output across repeated executions on the same platform.



# Machine Learning Segmentation & Worker Bridge "Seg"

Foreground occluder isolation bridges the Rust core with modern vision foundation models.


## 01) IPC Worker Protocol & Process Boundary

Communication between the Rust engine and Python ML models is mediated via standard JSON-lines over standard I/O pipes or local sockets using protocol `plaque-forge.segmentation-request/2`. Heavy frameworks (PyTorch, Hugging Face, SAM 2, Florence-2) must remain isolated in separate worker processes, shielding the Rust binary from Python interpreter crashes, GIL constraints, or CUDA memory leaks.


## 02) Model Version Pinning & Immutability

All vision foundation models (SAM 2, SAM 2.1, Florence-2, Cutie, ViTMatte) must specify pinned commit hashes in [tools/segmentation_runtime.py](file:///root/opencode/plaque-forge/tools/segmentation_runtime.py). The runtime must refuse to load unpinned floating tags in production workflows.


## 03) Scene-Specific Strategy & Parameter Isolation

To prevent model improvements for one video from breaking another, segmentation strategies, text prompts, box prompts, dilation kernels, and confidence thresholds must be declared per-scene in `scene.toml` and [src/segmentation_strategy.rs](file:///root/opencode/plaque-forge/src/segmentation_strategy.rs). Global parameter mutations that silently affect all scenes are strictly prohibited.


## 04) Multi-Model Bake-off & Pre-Promotion Gate

Any proposed change to a segmentation model, checkpoint weights, or default post-processing filter must pass the automated bake-off harness ([scripts/bakeoff_segmentation_matrix.sh](file:///root/opencode/plaque-forge/scripts/bakeoff_segmentation_matrix.sh)) across all representative capability assets. Promotion to default requires proving that existing homologated masks do not suffer boundary degradation or dropout frames.


## 05) Temporal Boundary Smoothing

Segmentation masks must undergo temporal consistency checks and edge-smoothing filters to eliminate high-frequency perimeter chatter (flicker) on delicate occluders across video frames.



# Rendering & Compositing Engine "Rnd"

The rendering pipeline produces photo-realistic plaque visuals and composites them with subpixel precision.


## 01) Linear-Light Color Processing

All color blending, lighting evaluation, specular reflection, and alpha compositing must execute in linear RGB color space ([src/color.rs](file:///root/opencode/plaque-forge/src/color.rs)). Conversion to/from non-linear sRGB / Rec.709 transfer curves must occur only at ingestion and final output encoding, eliminating dark border halos around translucent edges.


## 02) Dynamic Material Shading

Material shaders (`gold-shine`, `classic-glow`, stone, wood) must compute physically plausible surface highlights, anisotropic brushed streaks, and ambient illumination responses based on estimated surface normals and light vector trajectories.


## 03) Typography Rasterization & Fitting

Text must be laid out and shaped via `cosmic-text`, supporting font fallback, automatic multi-line wrapping, and proportional scale fitting to fill designated rectangular or non-rectangular writable regions without glyph truncation.



# Verification & Homologation Protection "Hom"

Homologation is the authoritative gate preventing visual regressions across all video assets.


## 01) Dual-Witness Acceptance Contracts

Every accepted video asset must have an executable contract ([assets/homologation/<scene>/contract.toml](file:///root/opencode/plaque-forge/assets/homologation)) defining:
- `source_preservation`: Max allowed difference outside the plaque bounding box, verifying that untouched footage remains pristine.
- `title_visibility`: Min allowed contrast and valid pixel count inside the plaque region, verifying that the title is sharp and legible.
- Evaluated keyframes, tolerances, and output diagnostics paths.


## 02) Full Asset Contract Coverage

All 19 scenes in the project asset library must have an authoritative `contract.toml`. Uncontracted assets represent technical debt and must not remain unprotected against accidental regressions.


## 03) CI Homologation Sentinels

Continuous integration must execute a fast, behaviorally diverse sentinel suite ([scripts/check_homologated_assets.sh](file:///root/opencode/plaque-forge/scripts/check_homologated_assets.sh)) covering planar tracking, moving perspective, foreground occlusions, portrait geometry, and temporary retraction on every push to `main`.


## 04) Full Matrix Regression Verification

Pre-release validation and major refactorings must execute the complete homologation matrix across all 19 video assets ([scripts/run_homologation_matrix.sh](file:///root/opencode/plaque-forge/scripts/run_homologation_matrix.sh)), ensuring zero regressions on untouched scenes.



# Test Architecture & Governance "Tst"

Testing practices enforce engineering policy compliance and rapid defect diagnosis.


## 01) TDD Discipline

All new features and defect corrections must begin with an executable automated test demonstrating the requirement or reproducing the defect before production code changes are introduced, complying with [docs/engineering-policy.md](file:///root/opencode/plaque-forge/docs/engineering-policy.md).


## 02) Fast Headless Unit & Subsystem Tests

Unit and subsystem tests in `tests/` must execute rapidly in headless environments without requiring physical GPUs, display servers, or external network access. Synthetic video fixtures ([tests/support/synthetic.rs](file:///root/opencode/plaque-forge/tests/support/synthetic.rs)) must be used to verify complex tracking, geometry, and rendering logic.


## 03) Diagnostic Diff Tracing on Regression

When a verification or homologation contract fails, the system must generate diagnostic diff images and metric logs highlighting the exact spatial coordinates, frame numbers, and threshold violations that caused the failure.
