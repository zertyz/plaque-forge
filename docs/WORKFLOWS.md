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
./scripts/analyze_assets.sh --force-ml asset    # regenerate authored and automatic ML artifacts even if still valid
./scripts/analyze_assets.sh --no-ml asset       # intentionally pure Rust
./scripts/ml_status.sh                           # active/recent Python workers
```

The ML runtime is a prerequisite for the default high-level command and lives entirely under `/tmp/plaque-forge-python`.

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

Only `assets/analysis/*` is deleted. Source videos, refinements, injected plaque images, outputs and `/tmp/plaque-forge-python` are preserved. Use `./scripts/analyze_assets.sh --force-ml` after the reset when you also want to regenerate refinement-owned prompted Python artifacts; refinement-owned non-ML artifacts are preserved deliberately.

Failed work normally cleans itself. To remove debris from older versions:

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

Custom `--encoder-arg` values must be self-contained settings, not workstation paths or file-backed filters. Arguments are persisted in the portable render manifest; fonts and style files are represented by basename plus content hash instead of an absolute path.

## Quality reports

```bash
./scripts/review_assets.sh
```

This creates/rebuilds each asset's actionable `diagnostics/review.html`/`review.txt` and writes the browsable `output/review/index.html`. It uses a complete analysis when available, otherwise the newest compact retained failure. Reports include prioritized failure reasons, visual evidence, exact rerun/refinement guidance, ML/Python participation, typography provenance, and verification data when available. When the current lossless rendered-video verification passes, the report treats that outcome as authoritative instead of asking for unnecessary refinement solely because a low-texture surface has low raw feature confidence.

## Portable cache migration

Audit first, then apply if older generated manifests contain workstation paths or schema/build identities:

```bash
cargo run -- migrate-analysis --root assets/analysis
cargo run -- migrate-analysis --root assets/analysis --apply
```

Migration does not rerun tracking or ML. It validates every upgraded cache and deterministically refreshes injected plaque derivatives when needed.

## Validation

```bash
./scripts/check.sh
TITLE_TEXT='Validation title' ./scripts/validate_assets.sh
```

Optional worker-only syntax check:

```bash
./scripts/check_segmentation.sh
```
