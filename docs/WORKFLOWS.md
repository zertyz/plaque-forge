# Workflows

The README is the normal path. This document contains lower-level and exceptional operations.

## High-level analysis

```bash
./scripts/analyze_assets.sh [asset-stem ...]
```

It performs the complete automatic pass: surface proposal/selection, motion, canonical/writable reconstruction, Rust foreground discovery, and ML foreground scene when useful. If Rust detects a persistent foreground crossing and no authoritative human foreground layer exists, the configured Python worker is automatically asked to sharpen the masks.

Useful controls:

```bash
./scripts/analyze_assets.sh --force asset       # rebuild scene cache; human-prompted ML artifacts are reused when valid
./scripts/analyze_assets.sh --force-ml asset    # regenerate authored and automatic ML artifacts even if still valid
./scripts/analyze_assets.sh --no-ml asset       # intentionally no Python; reusable cached prompted layers are optional
./scripts/ml_status.sh                           # active/recent Python workers
```

The ML runtime is a prerequisite for the default high-level command and lives entirely under `/tmp/plaque-forge-python`. `--no-ml` is a real degradation mode: it may reuse a valid cached prompted artifact, but an absent or stale prompted artifact is skipped rather than treated as an installation failure. Reviewed static/art-authored layer artifacts remain usable without pretending to have generated-worker device/request provenance.

Runtime setup is recoverable and independently verifiable:

```bash
./scripts/setup_segmentation.sh                         # XPU profile by default; repairs an interrupted compatible install
./scripts/setup_segmentation.sh --verify                # offline verification only
./scripts/setup_segmentation.sh --torch-profile cpu     # suitable for a CPU-only host/CI runner
./scripts/setup_segmentation.sh --reinstall             # explicit destructive rebuild
```

The SAM2 checkpoint loader resolves the repository's pinned commit and cached checkpoint explicitly before calling SAM2's local builder. This avoids an upstream `from_pretrained` path that can otherwise resolve an unpinned default Hub ref during offline verification.

A forced scene rebuild can reuse the prior automatic foreground sequence when its
source, seed masks, model, worker, runtime, and request identity all still validate.
`--force-ml` remains the explicit way to regenerate every ML layer; changing an
automatic request invalidates that generated sequence without user intervention.

The script asserts `--source-is-text-free` for this sample project. If your source plaque already contains lettering, remove it in an external inpainting/clean-plate workflow first; analysis intentionally refuses to imply that Plaque Forge can erase it.

## Reset generated analysis

```bash
./scripts/reset_analysis.sh --yes
./scripts/analyze_assets.sh
```

Only `assets/analysis/*` is deleted. Source videos, scenes, injected plaque images, outputs and `/tmp/plaque-forge-python` are preserved. Prompted Python artifacts live inside analysis, so the reset removes them; `--force-ml` also bypasses a still-valid previous ML cache during an in-place rebuild.

Failed work normally cleans itself. To remove stale debris explicitly:

```bash
./scripts/cleanup_work.sh --yes
```

Current work lives under `/tmp/plaque-forge/work`; compact failure evidence lives under `/tmp/plaque-forge/failures` and is automatically bounded.

## Plaque-less / injected surfaces

The included plaque-less assets already reference the aspect-matched Aetherglass Aurora pair; simply run the normal analyzer. The catalog also provides the alternative Prismwraith Reliquary pair.

For another video:

```bash
./scripts/place_plaque.sh my-video assets/plaques/aetherglass-aurora-16_9.png
./scripts/analyze_assets.sh my-video
```

`place_plaque.sh` proposes a quiet placement and writes a preview. `--bounds x,y,w,h` overrides it. `--space screen-canvas|scene-plane` controls anchoring. Injection skips meaningless plaque detection/extraction, while foreground crossings still participate in analysis.

## Human scene

Create a small scene only when the quality report says automatic intent is wrong:

```bash
./target/release/plaque-forge create-scene --input assets/video.mp4
```

The strict scene format supports sparse normalized trajectory corrections and concise writable shapes. Dense generated tracks/masks belong under generated analysis, not in the human editing loop. See [Scenes](SCENES.md).

## Export motion only for exceptional review

```bash
./target/release/plaque-forge export-trajectory --analysis assets/analysis/video
```

Prefer a few normalized `[[surfaces.anchors]]` entries over editing a dense generated trajectory.

## Manual segmentation debugging

The normal analyzer invokes ML automatically when appropriate. Direct segmentation exists for diagnosing/regenerating one authored prompted layer:

```bash
./target/release/plaque-forge segment \
  --input assets/video.mp4 \
  --layer foreground \
  --worker tools/segmentation-worker \
  --backend sam2-cutie-vitmatte \
  --model facebook/sam2.1-hiera-large \
  --device auto \
  --force
```

## Rendering

```bash
./scripts/render_assets.sh \
  --text 'Custom title' \
  --font-family 'Noto Serif' \
  --style gold-shine \
  swamp-rusty-plaque
```

`artistic` fitting is the default. Rendering requires a complete current analysis cache and does not silently run expensive analysis.

Custom `--encoder-arg` values must be self-contained settings, not workstation paths or file-backed filters. Arguments are persisted in the portable render manifest; fonts and style files are represented by basename plus content hash instead of an absolute path.

## Programmatic Rust API

The CLI is not required when Plaque Forge is embedded in another Rust tool. Major workflows accept
interface-independent request types:

```rust
use plaque_forge::application::{RenderRequest, TitleSource};

let request = RenderRequest::new(
    "assets/example.mp4",
    "assets/analysis/example",
    "output/example.mkv",
    TitleSource::Text("Custom title".into()),
    "/path/to/font.ttf",
);
plaque_forge::application::render(request)?;
```

Tests and host applications that need deterministic process boundaries can use
`ApplicationServices::new(...)` together with `analyze_with`, `render_with`, `verify_with`, or
`homologate_with`. Only dependencies with a genuine independent lifecycle are abstracted; the API
does not introduce interfaces for ordinary pure Rust operations.

## Quality reports

```bash
./scripts/review_assets.sh
```

This creates/rebuilds each asset's actionable `diagnostics/review.html`/`review.txt` and writes the browsable `output/review/index.html`. It uses a complete analysis when available, otherwise the newest compact retained failure. Reports include prioritized failure reasons, visual evidence, exact rerun/scene guidance, ML/Python participation, typography provenance, and verification data when available. When the current lossless rendered-video verification passes, the report treats that outcome as authoritative instead of asking for unnecessary scene solely because a low-texture surface has low raw feature confidence.

## Validation

```bash
./scripts/check.sh
TITLE_TEXT='Validation title' ./scripts/validate_assets.sh
```

Optional worker-only syntax check:

```bash
./scripts/check_segmentation.sh
```

## Continuous-integration analysis gates

`./scripts/analysis_change_scope.sh BASE HEAD` conservatively classifies a change as affecting analysis, ML-produced analysis, and/or the ML runtime. The main GitHub Actions workflow uses those outputs to avoid paying for heavyweight gates on unrelated commits.

When analysis behavior changes, `./scripts/check_analysis_regressions.sh` exercises the concrete `--no-ml` regression witnesses. When segmentation setup/worker requirements change, CI builds a CPU-profile runtime and then verifies the completed runtime with Hugging Face/Transformers forced offline.

Validation CI is intentionally read-only. It detects and rejects regressions; it does not silently rewrite the branch. A future generated-analysis producer should be a distinct trusted workflow with an explicit canonical execution profile and independent acceptance before committing generated bytes. See `docs/CI.md`.
