# Plaque Forge

Plaque Forge replaces or adds artistic title text on moving video surfaces while preserving camera/surface motion and objects that cross in front of the title.

A **writing surface** may be a rectangle, rounded plaque, circle/ellipse, polygon, irregular mask, or an injected transparent PNG. Plaque Forge does as much as it can automatically before asking for small human corrections.

Normal use is **setup once → analyze once → render many → review**.

## 1. Install prerequisites

On CachyOS / Arch Linux:

```bash
sudo pacman -S --needed rustup clang opencv ffmpeg pkgconf fontconfig uv git
rustup default stable
```

Prepare the ML foreground/object worker once:

```bash
./scripts/setup_segmentation.sh
```

Everything Python/model-related lives under **`/tmp/plaque-forge-python`**, including its synthetic `$HOME`, virtualenv, cloned ML repositories, and model caches. Nothing is installed into your real home directory. If `/tmp` is cleared, rerun setup.

## 2. Analyze once

```bash
./scripts/analyze_assets.sh
```

This is the high-level analysis command. It builds Plaque Forge, detects/selects the writing surface, tracks it, reconstructs the writable region, finds foreground crossings, and **automatically invokes the Python segmentation worker when ML can sharpen those foreground masks**. Existing human-declared ML prompts are also materialized automatically.

Analyze selected assets by appending their stems:

```bash
./scripts/analyze_assets.sh 16_9_dungeon_spider_iron_plaque 9_16_swamp_wooden_plaque
```

Use `--force` to rebuild current Rust/scene caches, `--force-ml` to regenerate ML work too, or `--no-ml` only when you explicitly want the pure-Rust path. Run `./scripts/ml_status.sh` to see whether Python actually ran.

When automatic quality is insufficient, the partial cache is retained and an actionable `diagnostics/review.html` + `review.txt` is generated. Fix only the smallest item it identifies, then rerun analysis.

### Plaque-less videos

The included plaque-less sample videos are already configured to use:

```text
assets/plaques/holographic-default.png
```

so the normal `./scripts/analyze_assets.sh` command handles them too. For another plaque-less asset:

```bash
./scripts/place_plaque.sh my-video assets/plaques/holographic-default.png
./scripts/analyze_assets.sh my-video
```

## 3. Render as many titles/styles as you want

```bash
./scripts/render_assets.sh \
  --text 'Nós que aqui estamos, por vós - ansiosamente - esperamos!' \
  --font-family 'Noto Serif' \
  --style gold-shine
```

Outputs go to `output/*.hevc.mkv`. Append asset stems to render only those videos.

**Artistic line composition is the default**, using the largest safe title size. The default direct style also has a visible glow.

Try the bundled styles with `--style NAME`, including `classic-glow`, `bronze-relief`, `gold-shine`, `chrome-shine`, `holographic-foil`, `neon-flicker`, `liquid-wave`, `chromatic-glitch`, `velocity-trails`, `letterpress-wood`, `frosted-ice`, `living-fire`, `cosmic-nebula`, `halftone-pop`, `typewriter`, and `particle-dissolve`.

See [Text effects](docs/TEXT_EFFECTS.md) for the exact capability matrix and style format.

## 4. Review quality

Generate/rebuild the human quality reports after analysis/rendering:

```bash
./scripts/review_assets.sh
```

Open:

```text
output/review/index.html
```

Each asset report prioritizes what matters, shows the visual evidence, states whether Python ML participated, points to the exact refinement file when one exists, and gives the commands to rerun. This is the preferred place for detailed quality/debug guidance.

## Start analysis completely fresh

To delete **all generated scene-analysis caches** and rebuild them:

```bash
./scripts/reset_analysis.sh --yes
./scripts/analyze_assets.sh
```

To also regenerate human-prompted Python layer artifacts while keeping the downloaded models/runtime:

```bash
./scripts/reset_analysis.sh --yes
./scripts/analyze_assets.sh --force-ml
```

This does not delete source videos, human/project refinement intent, plaque PNGs, rendered output, or `/tmp/plaque-forge-python`. Legacy refinement-owned non-ML assets such as hand-reviewed/deterministically derived moss/shadow masks are intentionally preserved.

## Repository map

```text
src/                         Rust implementation
scripts/                     high-level setup/analyze/render/review operations
tools/                       optional external-tool adapters
styles/                      reusable typography/material/effect programs
assets/*.mp4                 source videos
assets/plaques/              reusable injected plaque images
assets/refinements/<name>/   sparse human intent/corrections + generated layer artifacts
assets/analysis/<name>/      generated reusable scene cache
output/                      rendered videos and quality-report index
docs/                        architecture and advanced workflows
```

More detail: [Glossary](docs/GLOSSARY.md) · [Architecture](docs/ARCHITECTURE.md) · [Refinements](docs/REFINEMENTS.md) · [Workflows](docs/WORKFLOWS.md) · [Validation](docs/VALIDATION.md) · [Safety](docs/SAFETY.md).
