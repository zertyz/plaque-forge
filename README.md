# Plaque Forge

Plaque Forge places new typography onto title surfaces in video while preserving camera/plaque motion and objects that pass in front of the title.

A **writing surface** is the planar video area that receives text. It may be rectangular, rounded, elliptical/circular, polygonal, or an irregular mask. Plaque Forge tries to discover it automatically; human refinement is the fallback when confidence is insufficient.

The workflow is intentionally two commands after setup:

1. **Analyze once**: do the expensive scene work and cache it.
2. **Render many times**: reuse that cache for any title/style.

## 1. Install prerequisites

On CachyOS / Arch Linux:

```bash
sudo pacman -S --needed rustup clang opencv ffmpeg pkgconf fontconfig uv git
rustup default stable
```

For the complete analyzer, prepare the optional ML object-segmentation worker once:

```bash
./scripts/setup_segmentation.sh
```

Its Python interpreter, packages, model caches, cloned ML repositories, synthetic `$HOME`, and temporary files all live under **`/tmp/plaque-forge-python`**. It does not install Python packages into your real home directory. Because `/tmp` may be cleared by the OS, rerun setup when that directory no longer exists.

## 2. Analyze the videos

```bash
./scripts/analyze_assets.sh
```

That one command does everything Plaque Forge can do automatically before asking for human input: builds the Rust program, validates/reuses current caches, detects and tracks writing surfaces, reconstructs writable masks, analyzes foreground occlusion, and materializes any declared ML segmentation layers that are still missing.

Analyze only selected assets by appending their stems:

```bash
./scripts/analyze_assets.sh 16_9_dungeon_spider_iron_plaque 16_9_swamp_wooden_plaque
```

Use `--force` only when you intentionally want to rebuild a current cache. Use `--no-ml` when you explicitly want the pure-Rust path.

If an automatic quality gate still fails, Plaque Forge retains the partial analysis and immediately builds `diagnostics/review.html` plus a terminal-friendly `review.txt`. They tell you what deserves attention first; a small refinement should correct only the intended surface, the motion frames that are actually wrong, or a missed foreground object. See [Refinements](docs/REFINEMENTS.md).

## 3. Render titles

```bash
./scripts/render_assets.sh \
  --text 'Nós que aqui estamos, por vós - ansiosamente - esperamos!' \
  --font-family 'Noto Serif' \
  --style classic-glow
```

Outputs go to `output/*.hevc.mkv`. Append asset stems to render only selected videos.

**Artistic line composition is the default.** It chooses explicit word-boundary line layouts and uses the largest safe size. The built-in direct style also has a visible glow by default.

Reusable styles currently support flat/gradient/gold-bronze fills, stroke, glow, shadow, extrusion, bevel, pulse, and moving shine. Examples:

```bash
# Bronze/gold relief close to engraved/raised lettering on dark iron.
./scripts/render_assets.sh \
  --text 'Vendo o que ninguém mais vê' \
  --font-family 'Noto Serif' \
  --style bronze-relief \
  16_9_dungeon_spider_iron_plaque

# Strong animated neon.
./scripts/render_assets.sh \
  --text 'Seeing what others cannot see!' \
  --font-family 'Noto Serif' \
  --style neon-pulse
```

Run `./scripts/render_assets.sh --help` for common overrides. Existing environment variables such as `TITLE_TEXT`, `FONT`, `MAX_LINES`, and `GLOW_RADIUS` remain supported.

## Repository map

```text
src/                         Rust implementation
scripts/                     high-level setup/analyze/render/review operations
tools/                       optional external-tool adapters
styles/                      reusable typography/material/effect programs
assets/*.mp4                 source videos
assets/refinements/<name>/   sparse human-reviewed intent/corrections
assets/analysis/<name>/      generated reusable scene cache
output/                      rendered videos and reports
docs/                        detailed architecture and advanced workflows
```

## More detail

- [Glossary](docs/GLOSSARY.md): tracker, canonical surface, occluder, mask, and related terminology.
- [Architecture](docs/ARCHITECTURE.md): Rust/module boundaries and external-tool isolation.
- [Refinements](docs/REFINEMENTS.md): concise rectangle, rounded rectangle, ellipse/circle, polygon, and mask declarations.
- [Text effects](docs/TEXT_EFFECTS.md): effect pipeline, style-file format, and implemented/future effects.
- [Workflows](docs/WORKFLOWS.md): lower-level CLI commands for debugging and exceptional cases.
- [Validation](docs/VALIDATION.md): automated and visual quality checks.
- [Safety](docs/SAFETY.md): cache/output replacement and filesystem deletion rules.
