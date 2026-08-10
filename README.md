# Plaque Forge 0.3.0

Plaque Forge analyzes a moving, text-free plaque once and reuses that analysis for typography rendering.

## Layout

```text
assets/*.mp4                 source videos
assets/refinements/<name>/   reviewed geometry and alpha masks
assets/analysis/<name>/      generated analysis cache
output/*.hevc.mkv            production videos
```

Refinements are source data. Production rendering never writes refinements or analysis.

## Build

```bash
sudo pacman -S --needed rustup clang opencv ffmpeg pkgconf fontconfig uv git
rustup default stable
./scripts/check.sh
cargo build --release
```

## Production render

Render every asset with another title:

```bash
TITLE_TEXT='Custom title' FONT="$(fc-match -f '%{file}\n' 'DejaVu Sans' | head -n 1)" ./scripts/render_assets.sh
```

The complete typography interface is:

```bash
TITLE_TEXT='Custom title' \
FONT="$(fc-match -f '%{file}\n' 'DejaVu Sans' | head -n 1)" \
FIT=balanced \
FONT_SIZE=96 \
SUPERSAMPLING=4 \
TARGET_FILL=0.82 \
MAX_LINES=3 \
PADDING=0.05 \
LINE_HEIGHT=1.16 \
STROKE_WIDTH=0.025 \
TEXT_COLOR='#EBFFFFFF' \
STROKE_COLOR='#03181ED2' \
GLOW_COLOR='#69F2FA48' \
GLOW_RADIUS=4 \
TEXT_ALIGN=center \
VERTICAL_ALIGN=center \
./scripts/render_assets.sh
```

Colors use `#RRGGBBAA`. `FIT` is `maximize`, `balanced`, or `fixed`; `fixed` requires `FONT_SIZE`. The available effects are fill color, stroke, and glow.

Pass source stems to render only selected assets:

```bash
TITLE_TEXT='Custom title' ./scripts/render_assets.sh swamp-rusty-plaque swamp-wooden-plaque-with-foreground-objects
```

Production renders directly to HEVC, does not analyze or verify, and replaces only the selected `output/*.hevc.mkv` files.

## Analysis and validation

Create a missing analysis cache:

```bash
./target/release/plaque-forge analyze --input assets/swamp-rusty-plaque.mp4
```

Replace an existing cache only when its source or refinements changed:

```bash
./target/release/plaque-forge analyze --input assets/swamp-rusty-plaque.mp4 --force
```

`--force` explicitly replaces that analysis directory after the new analysis succeeds. Validate all cached assets before production:

```bash
./scripts/validate_assets.sh
```

Validation creates lossless temporary renders and replaces the corresponding `output/*.verification.json` reports. It never rebuilds analysis.

## CLI

```text
refine         create an editable refinement proposal
analyze        generate or explicitly replace an analysis cache
export-motion  export analyzed motion for review
segment        generate a refinement layer with the Python worker
render         render from an existing analysis cache
verify         verify an existing lossless render
```

Direct lossless render:

```bash
./target/release/plaque-forge render \
  --input assets/swamp-rusty-plaque.mp4 \
  --text 'Custom title' \
  --font "$(fc-match -f '%{file}\n' 'DejaVu Sans' | head -n 1)"
```

## Refinements

Automatic analysis is used when no refinement exists. A refinement can fix plaque bounds, motion, writing surface, foreground alpha, and shadow alpha. See [REFINEMENTS.md](REFINEMENTS.md).

SAM 2, Cutie, ViTMatte, and MatAnyone2 are isolated under `/tmp/plaque-forge-python`:

```bash
./scripts/setup_segmentation.sh
```

The setup command preserves an existing or incomplete environment. To deliberately delete and reinstall it:

```bash
./scripts/setup_segmentation.sh --reinstall
```

The checked-in refinement masks are sufficient for rendering. Python is needed only to regenerate them.

## Scope

The validated target is one planar, text-free plaque in a constant-frame-rate shot. The six included videos are the acceptance set.
