# Plaque Forge 0.3.0

Plaque Forge adds typography to a moving, text-free plaque. It analyzes plaque motion and occlusion once, renders a lossless master, and verifies the result.

## Layout

```text
assets/*.mp4                 source videos
assets/refinements/<name>/   reviewed geometry and alpha masks
assets/analysis/<name>/      generated analysis cache
output/*.hevc.mkv            inspection videos
```

`assets/analysis/` and `output/` are disposable. Refinements are source data.

## Build

```bash
sudo pacman -S --needed rustup clang opencv ffmpeg pkgconf fontconfig uv git
rustup default stable
./scripts/check.sh
cargo build --release
```

## Render all assets

```bash
./scripts/render_assets.sh
```

This deletes the generated analysis cache, analyzes all six sources, verifies lossless renders, and writes HEVC inspection videos to `output/`.

Optional overrides:

```bash
FONT=/path/to/font.ttf \
TITLE_TEXT='Custom title' \
./scripts/render_assets.sh
```

Pass source stems to render only selected assets:

```bash
./scripts/render_assets.sh swamp-rusty-plaque swamp-wooden-plaque-with-foreground-objects
```

## CLI

```text
refine         create an editable refinement proposal
analyze        generate or replace an analysis cache
export-motion  export analyzed motion for review
segment        generate a refinement layer with the Python worker
render         analyze when needed, render, and verify
verify         verify an existing lossless render
```

Defaults are derived from `--input`:

```text
assets/<name>.mp4
assets/refinements/<name>/refinement.toml
assets/analysis/<name>/
output/<name>.mkv
```

Example:

```bash
FONT="$(fc-match -f '%{file}\n' 'DejaVu Sans' | head -n 1)"

./target/release/plaque-forge render \
  --input assets/swamp-rusty-plaque.mp4 \
  --text 'Custom title' \
  --font "$FONT" \
  --reanalyze
```

The CLI output is lossless FFV1 because verification checks scene preservation. `scripts/render_assets.sh` creates the HEVC copies used for playback.

## Refinements

Automatic analysis is used when no refinement exists. A refinement can fix plaque bounds, selected motion frames, writing surface, foreground alpha, and shadow alpha. See [REFINEMENTS.md](REFINEMENTS.md).

Optional SAM 2, Cutie, ViTMatte, and MatAnyone2 support is isolated under `/tmp/plaque-forge-python`:

```bash
./scripts/setup_segmentation.sh
```

The checked-in refined masks are sufficient for normal rendering; Python is needed only to regenerate segmentation layers.

## Scope

The validated target is one planar, text-free plaque in a constant-frame-rate shot. The six included videos are the current acceptance set. General material synthesis and automatic handling of arbitrary new scene classes remain future work.
