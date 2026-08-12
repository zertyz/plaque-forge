# Plaque Forge

Plaque Forge places new typography onto a moving, text-free plaque in a video while preserving the plaque motion and objects that pass in front of it.

It does this in two phases:

1. **Analyze once:** detect the plaque, track it through the shot, recover its writable surface, and record foreground occlusion.
2. **Render many times:** reuse that analysis cache for different titles and typography settings.

The included assets already have analysis caches, so you can render immediately after building.

## Quick start

On CachyOS / Arch Linux:

```bash
sudo pacman -S --needed rustup clang opencv ffmpeg pkgconf fontconfig
rustup default stable
```

Render all included videos:

```bash
./scripts/render_assets.sh \
  --text 'Nós que aqui estamos, por vós - ansiosamente - esperamos!' \
  --font-family 'Noto Serif' \
  --max-lines 5 \
  --padding 0 \
  --stroke-width 0.05 \
  --glow-radius 10
```

Rendered videos are written to `output/*.hevc.mkv`. Pass one or more asset stems at the end to render only those assets:

```bash
./scripts/render_assets.sh --text 'A new title' --font-family 'Noto Serif' swamp-rusty-plaque
```

For title-oriented line breaking, add `--fit artistic`. For reusable paint/effect stacks, use `--style-file styles/classic-glow.toml`. Run `./scripts/render_assets.sh --help` for common typography options. Existing environment variables such as `TITLE_TEXT`, `FONT`, `MAX_LINES`, and `GLOW_RADIUS` are still accepted.

## Main workflows

### Build or rebuild scene analysis

An **analysis cache** is generated scene data that is expensive to compute but reusable across titles. Build missing caches with:

```bash
./scripts/analyze_assets.sh
```

Rebuild selected caches only when the input video, refinements, or analysis semantics changed:

```bash
./scripts/analyze_assets.sh --force swamp-rusty-plaque
```

### Refine a difficult scene

A **refinement** is reviewed input that corrects or supplements automatic analysis, for example plaque bounds, motion constraints, or foreground masks.

```bash
cargo build --release
./target/release/plaque-forge refine --input assets/my-video.mp4
```

See [docs/REFINEMENTS.md](docs/REFINEMENTS.md).

### Detect a foreground object

Some foreground layers use Python-only ML models. Install that optional environment once:

```bash
./scripts/setup_segmentation.sh
```

Then generate a declared layer from the prompts in its refinement file:

```bash
./scripts/detect_objects.sh my-video foreground --force
```

Python is not required for normal rendering or analysis of the checked-in assets.

### Review visual diagnostics

Create an HTML triage page from existing analysis and verification data:

```bash
./scripts/review_assets.sh swamp-rusty-plaque
```

Open `assets/analysis/<name>/diagnostics/review.html`. It puts the important confidence metrics and diagnostic images in one place so you can decide whether tracking, occlusion, reconstruction, or typography needs attention first.

## Repository layout

```text
src/                         Rust implementation
scripts/                     high-level build/analyze/render operations
tools/                       optional external-tool adapters
assets/*.mp4                 source videos
assets/refinements/<name>/   reviewed scene corrections and layer artifacts
assets/analysis/<name>/      generated reusable scene analysis
output/                      rendered videos and reports
docs/                        concepts, architecture, refinement and validation docs
```

## Documentation

Start with:

- [Glossary](docs/GLOSSARY.md) for scene-analysis terminology such as tracker, canonical plaque, occluder, and alpha mask.
- [Architecture](docs/ARCHITECTURE.md) for module boundaries and external-tool isolation.
- [Workflows](docs/WORKFLOWS.md) for direct CLI commands and less common operations.
- [Refinements](docs/REFINEMENTS.md) for the reviewed-input format.
- [Validation](docs/VALIDATION.md) for automated and visual quality checks.
- [Text effects](docs/TEXT_EFFECTS.md) for the text-rendering stages, style files, and effect-extension rules.
- [Safety](docs/SAFETY.md) for filesystem replacement and deletion rules.

## Supported source contract

The validated target is one planar, text-free plaque in a constant-frame-rate shot. The six included videos are the current acceptance set.
