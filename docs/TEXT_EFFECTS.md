# Text effects architecture

Plaque Forge treats title rendering as a small rendering pipeline rather than a list of unrelated switches. This matters because effects such as glow, wobble, gold, and engraving operate on different kinds of data.

## Pipeline boundaries

```text
text + font
   |
   v
shape and line layout
   |        layout effects: wave, wobble, arc, per-glyph transforms
   v
canonical glyph geometry / coverage
   |        mask effects: stroke, glow, shadow, trails
   v
fill/material shading
   |        flat color, gradient, texture, metal, foil, animated shine
   v
canonical title layer
   |        scene-surface effects: engraving, emboss/protrusion, plaque-aware lighting
   v
plaque warp + foreground restoration
   |
   v
video frame
```

Animation is not a separate rendering layer. Time is an input that may drive parameters in any stage. A pulse may alter fill opacity or scale; a moving shine alters material shading; a wobble alters glyph transforms.

## Why the stages are separate

A single `TextEffect::apply(RGBA)` abstraction would be convenient initially but wrong for several desired effects:

- **Wave / wobble / arc** need glyph positions before the title becomes one bitmap.
- **Glow / outline / shadow** naturally derive layers from glyph coverage.
- **Gold / chrome / holographic / texture fill / shine** need coordinates and material shading inside the glyphs.
- **Engraving / letterpress / convincing protrusion** need the plaque surface itself, not merely an RGBA overlay.
- **Pulse / flicker / moving shine** require deterministic time input without forcing scene analysis to know anything about typography animation.

The scene-analysis cache therefore remains independent of title style. Changing a text effect must not invalidate plaque tracking or extraction caches.

## Current implementation

`src/render/typography.rs` owns shaping, fitting, line layout, and production of the base glyph coverage.

`src/render/effects.rs` owns effects that operate on that already-shaped coverage. The current built-ins are:

- stroke;
- glow;
- drop shadow;
- flat fill.

The existing CLI paint flags remain supported. For combinations that would make the command line noisy, use a TOML style file:

```bash
plaque-forge render \
  --input assets/example.mp4 \
  --text 'A title' \
  --font /path/to/font.ttf \
  --style-file styles/classic-glow.toml
```

A style file currently supports:

```toml
version = 1
fill = "#EBFFFFFF"

[[effects]]
type = "shadow"
offset_x = 0.025     # fraction of fitted font size
offset_y = 0.035
blur_radius = 5      # final-output pixels
color = "#00000078"

[[effects]]
type = "stroke"
width = 0.035        # fraction of fitted font size
color = "#03181ED2"

[[effects]]
type = "glow"
radius = 8           # final-output pixels
color = "#69F2FA48"
```

When `--style-file` is present, it defines the paint/effect stack and the direct `--text-color`, `--stroke-*`, `--glow-*`, and `--shadow-*` values are ignored. Style files are schema-versioned so future material/animation additions can evolve deliberately. Layout flags such as `--fit`, `--max-lines`, and `--padding` still apply.
The render manifest records the resolved style and, when a style file is used, its SHA-256. Future styles that reference texture/material assets must likewise record content hashes for those assets rather than relying on paths alone.


## Static layout, frame-varying presentation

The current renderer can cache a complete canonical title layer because every implemented style is static. Animated effects require a more precise boundary rather than moving shaping into the video loop.

The target split is:

- **prepared typography**: shaped runs, selected line breaks, glyph placement, canonical coordinates, and reusable glyph coverage;
- **style program**: fill/material/effect definitions plus their deterministic parameters;
- **frame context**: frame index, timestamp, duration, normalized progress, and an explicit seed;
- **presentation cache**: any stage proven not to depend on frame context is computed once and reused.

For a static style, the whole canonical title remains one cached layer exactly as today. For a moving shine, only material shading needs reevaluation. For a pulse or wobble, glyph presentation changes per frame, but font discovery and line breaking do not. Scene analysis remains untouched in every case.

An animated effect must also declare or conservatively estimate its maximum visual extent. Fitting must validate that envelope against the writable plaque mask rather than checking only frame zero.

## CLI growth rule

Do not add a dozen top-level flags for every new artistic effect. The intended split is:

- common, simple controls may have direct CLI flags;
- complex or composable effects belong in style files;
- later, named style presets can provide concise CLI shortcuts without flattening every parameter into `RenderArgs`.

This keeps ordinary rendering readable while still permitting deep control.

## Capability targets

The effect system should eventually be able to express the following families without changing scene analysis:

- glyph motion: wave, wobble, pulse, arc, jitter;
- coverage/layer effects: outlines, glow, shadow, trails, chromatic offsets;
- materials: gradients, image/procedural textures, gold, chrome, holographic foil, liquid/fire/ice-like fills, moving shine;
- temporal reveals: flicker, typewriter/scramble, dissolve/assemble;
- surface interaction: letterpress, laser/wood engraving, emboss, protrusion and scene-consistent cast/inner shadows.

This list is a capability target, not an assertion that every effect is implemented today.

## Dependency policy

Effects do not get to choose the architecture by importing a framework into the core renderer. A new dependency should be accepted only when it supplies a primitive that is hard to implement correctly or efficiently in the existing rendering layer.

GPU or shader backends may be added later behind a renderer boundary if they materially improve effects or performance. Backend-specific concepts must not leak into analysis caches, refinements, CLI domain types, or scene geometry.
