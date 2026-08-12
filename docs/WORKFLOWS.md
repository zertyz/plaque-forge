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

This creates a short schema-2 `assets/refinements/video/refinement.toml` by default. Candidate alternatives stay in diagnostics rather than being dumped into the editable file.

## Export motion for review

```bash
./target/release/plaque-forge export-motion --analysis assets/analysis/video
```

The exported dense motion file is generated review material and now defaults to `assets/refinements/<video>/artifacts/motion.toml`. For ordinary corrections, prefer a few normalized `[[plaques.motion]]` anchors directly in `refinement.toml` instead of editing the dense export.

## Add a plaque to a plaque-less asset

```bash
./scripts/place_plaque.sh 16_9_plaqueless_swamp my-plaque.png
./scripts/analyze_assets.sh 16_9_plaqueless_swamp
```

The first command proposes a quiet placement and writes a preview plus sparse refinement intent. Use `--bounds x,y,w,h` to override placement and `--motion auto|screen|scene` to control anchoring. Rendering afterward is the same as for a plaque already present in video.

## Build an analysis cache

Prefer the high-level command; it verifies cache freshness and generates any missing prompted ML layers:

```bash
./scripts/analyze_assets.sh video
```

The lower-level equivalent is available for debugging:

```bash
./target/release/plaque-forge analyze --input assets/video.mp4
```

Use `--force` only when intentionally replacing an existing Rust analysis cache. Prompted ML artifacts are an independent expensive cache and are reused by default. Use `--force-ml` when you explicitly want Python inference to regenerate them.

The analyzer emits `[ml]` events for enabled/disabled, no-refinement, no-prompts, cache-hit, worker launch/PID, completion, and failure states. Inspect the temporary runtime at any time with:

```bash
./scripts/ml_status.sh
```

Replacement is staged and the old scene cache remains in place until the new analysis succeeds.

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

For a complete analysis the report is written to `assets/analysis/swamp-rusty-plaque/diagnostics/review.html`; if only a failed partial analysis exists, the script automatically uses the newest partial directory. `review.txt` beside it gives the same prioritized actions in terminal-friendly form. The HTML also compares candidate alternatives and includes a click helper that reports normalized/source-pixel coordinates for sparse corrections.

`analyze_assets.sh` invokes this review generation automatically after a failed quality gate, so a normal user does not need a separate diagnostic command merely to learn what needs attention.

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
