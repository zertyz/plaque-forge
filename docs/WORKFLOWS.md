# Workflows

The README is the normal path. This document contains lower-level and exceptional operations.

## High-level analysis

```bash
./scripts/analyze_assets.sh [asset-stem ...]
```

It performs the complete automatic pass: surface proposal/selection, motion, canonical/writable reconstruction, Rust foreground discovery, and ML foreground refinement when useful. If Rust detects a persistent foreground crossing and no authoritative human foreground layer exists, the configured Python worker is automatically asked to sharpen the masks.

Useful controls:

```bash
./scripts/analyze_assets.sh --force asset       # rebuild scene cache; human-prompted ML artifacts are reused when valid
./scripts/analyze_assets.sh --force-ml asset    # regenerate human-prompted ML artifacts; automatic foreground ML is recomputed with forced scene analysis
./scripts/analyze_assets.sh --no-ml asset       # intentionally pure Rust
./scripts/ml_status.sh                           # active/recent Python workers
```

The ML runtime is a prerequisite for the default high-level command and lives entirely under `/tmp/plaque-forge-python`.

## Reset generated analysis

```bash
./scripts/reset_analysis.sh --yes
./scripts/analyze_assets.sh
```

Only `assets/analysis/*` is deleted. Source videos, refinements, injected plaque images, outputs and `/tmp/plaque-forge-python` are preserved. Use `./scripts/analyze_assets.sh --force-ml` after the reset when you also want to regenerate human-prompted Python layer artifacts; refinement-owned non-ML artifacts are preserved deliberately.

## Plaque-less / injected surfaces

The included plaque-less assets already have refinements referencing `assets/plaques/holographic-default.png`; simply run the normal analyzer.

For another video:

```bash
./scripts/place_plaque.sh my-video assets/plaques/holographic-default.png
./scripts/analyze_assets.sh my-video
```

`place_plaque.sh` proposes a quiet placement and writes a preview. `--bounds x,y,w,h` overrides it. `--motion screen|scene|auto` controls anchoring. Injection skips meaningless plaque detection/extraction, while foreground crossings still participate in analysis.

## Human refinement

Create a small refinement only when the quality report says automatic intent is wrong:

```bash
./target/release/plaque-forge refine --input assets/video.mp4
```

Schema 2 supports sparse normalized motion corrections and concise writable shapes. Dense generated tracks/masks belong under generated artifacts, not in the human editing loop. See [Refinements](REFINEMENTS.md).

## Export motion only for exceptional review

```bash
./target/release/plaque-forge export-motion --analysis assets/analysis/video
```

Prefer a few normalized `[[plaques.motion]]` anchors over editing dense generated motion.

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

## Quality reports

```bash
./scripts/review_assets.sh
```

This creates/rebuilds each asset's actionable `diagnostics/review.html`/`review.txt` and writes the browsable `output/review/index.html`. It uses a complete analysis when available, otherwise the newest retained partial. Reports include prioritized failure reasons, visual evidence, exact rerun/refinement guidance, ML/Python participation, typography provenance, and verification data when available.

## Validation

```bash
./scripts/check.sh
TITLE_TEXT='Validation title' ./scripts/validate_assets.sh
```

Optional worker-only syntax check:

```bash
./scripts/check_segmentation.sh
```
