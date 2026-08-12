# Workflows

The root README covers the common path. This document contains direct and less common commands.

## Render one asset directly

```bash
cargo build --release
./target/release/plaque-forge render \
  --input assets/swamp-rusty-plaque.mp4 \
  --text 'Custom title' \
  --font "$(fc-match -f '%{file}\n' 'Noto Serif' | head -n 1)"
```

`render` requires an existing analysis cache. It does not run analysis and does not modify refinements.


### Artistic line layout

`artistic` is the default. `maximize` and `balanced` remain available when you want their simpler wrapping behavior. `artistic` evaluates a bounded set of explicit word-boundary layouts, measures them with the selected font, and records the chosen line breaks in the render manifest. Its score considers fitted size, line balance/raggedness, weak final lines, and obvious stranded punctuation. Explicit newlines remain authoritative.

```bash
./scripts/render_assets.sh \
  --text 'Nós que aqui estamos, por vós - ansiosamente - esperamos!' \
  --font-family 'Noto Serif' \
  --max-lines 5 \
  --fit artistic \
  swamp-rusty-plaque
```

### Reusable text styles

Use direct flags for quick experiments. Use a versioned style file when the paint stack becomes reusable or complex:

```bash
./scripts/render_assets.sh \
  --text 'Custom title' \
  --font-family 'Noto Serif' \
  --style classic-glow \
  swamp-rusty-plaque
```

The style file owns reusable paint/material/effect/animation settings, including gold/gradient materials, extrusion, bevel, pulse, and shine. Layout flags such as `--fit`, `--max-lines`, and `--padding` remain independent.

## Create an automatic refinement proposal

```bash
./target/release/plaque-forge refine --input assets/video.mp4
```

This creates `assets/refinements/video/refinement.toml` by default. Review it before treating it as authoritative input.

## Export motion for review

```bash
./target/release/plaque-forge export-motion --analysis assets/analysis/video
```

The exported motion file is generated review material. Edit or retain only the keyframes that need human authority instead of treating every frame as something a person should type.

## Build an analysis cache

Prefer the high-level command; it verifies cache freshness and generates any missing prompted ML layers:

```bash
./scripts/analyze_assets.sh video
```

The lower-level equivalent is available for debugging:

```bash
./target/release/plaque-forge analyze --input assets/video.mp4
```

Use `--force` only when intentionally replacing an existing cache. Replacement is staged and the old cache remains in place until the new analysis succeeds.

## Generate a segmentation layer manually

Normally `analyze_assets.sh` generates missing prompted layers automatically. For debugging or explicit regeneration, use the direct low-level command:

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


## Review diagnostics

Build a human-oriented report after analysis or validation:

```bash
./scripts/review_assets.sh swamp-rusty-plaque
```

The report is written to `assets/analysis/swamp-rusty-plaque/diagnostics/review.html`. When render provenance and verification reports are available, it also shows the resolved line layout, style, font, verification thresholds, and failures.

## Validate

Run code checks:

```bash
./scripts/check.sh
```

The optional Python worker has a separate syntax check so normal Rust development does not require Python:

```bash
./scripts/check_segmentation.sh
```

Validate rendered behavior on the included assets:

```bash
TITLE_TEXT='Validation title' ./scripts/validate_assets.sh
```
