# Text effects roadmap

Status: active implementation plan. The stable pipeline contract is documented in `docs/TEXT_EFFECTS.md`.

## Implemented through 0.5.0

- shaping / fitting remains separate from paint and animation;
- static fill, linear gradient, and procedural gold/bronze material;
- stroke, visible glow, drop shadow, extrusion/depth underlay, and bevel;
- deterministic pulse and moving shine without rerunning shaping/layout each frame;
- TOML style programs plus concise named presets through the high-level render script;
- hard-effect fit envelopes so glow/blur tails do not needlessly shrink titles;
- style/title/font provenance in render manifests.

## Next: glyph/layout transforms

Preserve per-glyph placement information after shaping so effects can transform glyphs before rasterization.

Targets:

- wave / wobble;
- pulse/scale that changes geometry rather than only opacity;
- arc / curved baseline;
- controlled jitter / glitch.

The important constraint is to preserve shaping clusters and script correctness. Effects must transform shaped glyphs rather than split arbitrary Unicode text into Rust `char`s.

## Next: richer materials

Extend the material interface while keeping canonical coordinates and deterministic time input.

Targets:

- image texture clipped to text;
- radial/multistop gradients;
- chrome / holographic / iridescent fills;
- configurable procedural roughness and scratches.

A GPU backend may be added later behind the same renderer boundary if profiling justifies it; backend-specific types must not leak into scene analysis or scenes.

## Later: plaque-surface interaction

Engraving and convincing protrusion are not ordinary RGBA overlays. Add a scene/material stage that can sample the canonical plaque image and modify its shading while respecting the writable mask.

Targets:

- letterpress / deboss;
- laser-burn / carved wood;
- true emboss / protrusion;
- surface-aware inner/cast shadows;
- optional normal/height-map approximation derived from text coverage.

These effects stay downstream of scene analysis, so changing typography never rebuilds tracking caches.

## Later: temporal/reveal families

With reusable prepared typography and frame context in place, add effects whose visible geometry or coverage changes over time:

- flicker;
- trails;
- dissolve / assemble;
- typewriter / scramble where appropriate.

Every animated stage must declare or conservatively estimate its maximum hard visual extent so fitting remains safe.
