# Text effects roadmap

Status: implementation plan. Phase 1 foundations are implemented in 0.4.0. The architecture contract is documented in `docs/TEXT_EFFECTS.md`.

## Phase 1: mask effects and style stacks

- Move current stroke/glow out of shaping code.
- Add drop shadow.
- Add TOML style files so complex styles do not become giant CLI commands.
- Preserve direct paint flags.
- Record the resolved text style in render provenance.

## Phase 2: material/fill stage

Introduce a fill/material interface that receives canonical coordinates, glyph coverage, deterministic seed, and time.

Initial targets:

- linear/radial gradient;
- image texture clipped to text;
- procedural gold/metal;
- moving shine sweep;
- holographic/iridescent fill.

These should remain CPU-capable initially so preview/export paths stay deterministic. A GPU backend can be introduced later behind the same interface if profiling justifies it.

## Phase 3: glyph/layout transforms

Preserve per-glyph placement information after shaping so effects can transform glyphs before rasterization.

Targets:

- wave;
- wobble;
- pulse/scale;
- arc;
- controlled jitter/glitch.

The important constraint is to preserve shaping clusters and script correctness. Effects must transform shaped glyphs rather than split arbitrary Unicode text into Rust `char`s.

## Phase 4: temporal effects

Split the current static `TextRender` result into reusable **prepared typography** and a **presentation/style program**. Add deterministic `FrameContext` containing frame index, timestamp, duration, normalized progress, and seed.

Each style/effect must declare whether it depends on time and its maximum visual extent. Static stages remain cached; an animated stage recomputes only what actually changes. Line breaking, font discovery, plaque analysis, and unrelated static effects must not run once per video frame.

Targets:

- pulse;
- flicker;
- moving shine;
- trails;
- reveal/scramble where appropriate.

## Phase 5: plaque-surface interaction

Engraving and convincing protrusion are not ordinary overlays. Add a scene/material stage that can sample the canonical plaque image and modify its shading while still respecting the analyzed writing mask.

Targets:

- letterpress/deboss;
- laser-burn / carved wood;
- emboss/protrude;
- surface-aware inner/cast shadows;
- optional normal/height-map approximation derived from the text mask.

These effects must remain downstream of analysis so changing title style never rebuilds tracking caches.
