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

Everything Python/model-related lives under **`/tmp/plaque-forge-python`**, including its virtualenv, cloned ML repositories, and model caches. Nothing is installed into your user Python environment or user caches. If `/tmp` is cleared, rerun setup.

Plaque Forge requires Rust 1.89 or newer, FFmpeg/FFprobe, OpenCV, Clang, and fontconfig. The optional worker uses a setup-managed Python 3.10 environment with exact package, source-commit, and model-revision identities.

## 2. Analyze once

```bash
./scripts/analyze_assets.sh
```

This is the high-level analysis command. It builds Plaque Forge, detects/selects the writing surface, tracks it, reconstructs the writable region, finds foreground crossings, and **automatically invokes the Python segmentation worker when ML can sharpen those foreground masks**. Existing human-declared ML prompts are also materialized automatically.

The bundled source surfaces are intentionally text-free. The high-level script makes that assertion explicitly. Direct CLI analysis must also include `--source-is-text-free`: Plaque Forge composites new typography, but does not remove or inpaint an existing title.

Analyze selected assets by appending their stems:

```bash
./scripts/analyze_assets.sh 16_9_dungeon_spider_iron_plaque 9_16_swamp_wooden_plaque
```

Use `--force` to rebuild current Rust/scene caches, `--force-ml` to regenerate ML work too, or `--no-ml` only when you explicitly want the pure-Rust path. Run `./scripts/ml_status.sh` to see whether Python actually ran.

When automatic quality is insufficient, the incomplete cache is deleted. Only compact diagnostics are retained under `/tmp/plaque-forge/failures/<asset>/`, limited to the newest three failures and seven days. Fix only the smallest item identified by `review.html`, then rerun analysis; a successful run removes retained failures for that asset.

### Plaque-less videos

The included plaque-less sample videos use the aspect-specific **Aetherglass Aurora** pair:

```text
assets/plaques/aetherglass-aurora-16_9.png
assets/plaques/aetherglass-aurora-9_16.png
```

An additional **Prismwraith Reliquary** pair is available for both aspect ratios. See `assets/plaques/catalog.toml` for dimensions, writable insets, and hashes. For another plaque-less asset:

```bash
./scripts/place_plaque.sh my-video assets/plaques/aetherglass-aurora-16_9.png
./scripts/analyze_assets.sh my-video
```

## 3. Render as many titles/styles as you want

```bash
./scripts/render_assets.sh \
  --text 'Nós que aqui estamos, por vós - ansiosamente - esperamos!' \
  --font-family 'Noto Serif' \
  --style gold-shine
```

Outputs go to `output/*.hevc.mkv`. Append asset stems to render only those videos. Each video, canonical text mask, optional contact sheet, and render manifest is published as a transactional bundle; an interrupted render cannot replace a previously complete bundle with partial files.

**Artistic line composition is the default**, using the largest safe title size. The default direct style also has a visible glow.

Try the bundled styles with `--style NAME`. In addition to the existing glow, metal, holographic, liquid, glitch, trail, ice, fire, nebula, halftone, typewriter, and dissolve presets, the renderer now includes `art-deco-arc`, `orbital-text`, `texture-mapped`, `scramble-reveal`, `split-flap`, `confetti-converge`, `laser-burn-wood`, `scene-emboss`, `blueprint`, and `paper-collage`.

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

Each asset report prioritizes what matters, shows the visual evidence, states whether Python ML participated, points to the exact scene file when one exists, and gives the commands to rerun. This is the preferred place for detailed quality/debug guidance.

## Start analysis completely fresh

To delete **all generated scene-analysis caches** and rebuild them:

```bash
./scripts/reset_analysis.sh --yes
./scripts/analyze_assets.sh
```

To also regenerate every Python ML layer while keeping the downloaded models/runtime:

```bash
./scripts/reset_analysis.sh --yes
./scripts/analyze_assets.sh --force-ml
```

This does not delete source videos, human/project scene intent, plaque PNGs, rendered output, or `/tmp/plaque-forge-python`. Small reviewed scene masks such as moss/shadow geometry are intentionally preserved.

To remove obsolete pre-0.8 partial directories and completed worker request files without touching complete caches:

```bash
./scripts/cleanup_work.sh --yes
```

Generated manifests use only portable relative paths. Incompatible caches are rejected and must be regenerated; they are never silently relabelled.

## Repository map

```text
src/                         Rust implementation
scripts/                     high-level setup/analyze/render/review operations
tools/                       optional external-tool adapters
styles/                      reusable typography/material/effect programs
assets/*.mp4                 source videos
assets/plaques/              reusable injected plaque images
assets/scenes/<name>/        sparse human intent + small reviewed source masks
assets/analysis/<name>/      generated, reproducible scene cache (never human intent)
output/                      rendered videos and quality-report index
docs/                        architecture and advanced workflows
```

The project, including its bundled assets, is MIT-licensed. More detail: [Glossary](docs/GLOSSARY.md) · [Architecture](docs/ARCHITECTURE.md) · [Scenes](docs/SCENES.md) · [Workflows](docs/WORKFLOWS.md) · [Validation](docs/VALIDATION.md) · [Performance](docs/PERFORMANCE.md) · [Security](docs/SECURITY.md) · [Safety](docs/SAFETY.md).
