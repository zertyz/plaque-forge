# Text effects

Plaque Forge treats title rendering as a pipeline because different effects operate on different data. A single `apply_effect(RGBA)` abstraction would be convenient and wrong.

```text
text + font
   |
   v
shape and line layout
   |        future glyph transforms: wave, wobble, arc, per-glyph motion
   v
glyph coverage
   |        stroke, glow, shadow, extrusion
   v
fill / material
   |        flat color, gradient, procedural gold
   v
surface detail
   |        bevel
   v
prepared canonical title
   |        frame presentation: pulse, moving shine
   v
plaque warp + foreground restoration
   |
   v
video frame
```

Scene analysis is independent from typography style. Changing text effects does not invalidate tracking/extraction caches.

## Implemented in 0.5

Static effects/materials:

- flat fill;
- linear gradient fill;
- procedural gold/bronze material;
- stroke;
- glow;
- drop shadow;
- extrusion/depth underlay;
- bevel highlight/shadow.

Animated presentation:

- pulse (opacity modulation);
- moving shine constrained to the actual glyph fill.

The renderer prepares font discovery, shaping, line breaks, and the static title once. Animated shine/pulse are evaluated per video frame without re-running typography layout.

Still future because they require different renderer stages:

- wave/wobble/arc and other per-glyph geometry transforms;
- texture-image materials;
- true plaque-surface engraving/laser burn;
- true protrusion/displacement with scene-consistent lighting;
- particle/dissolve/typewriter families.

## Defaults

`--fit artistic` is now the default. It searches bounded word-boundary arrangements and scores visual balance before choosing the maximum safe size.

Direct CLI rendering also uses a deliberately visible cyan glow by default. A style file replaces the direct paint flags.

## Style files

### Strong classic glow

```bash
./scripts/render_assets.sh \
  --text 'A title' \
  --font-family 'Noto Serif' \
  --style classic-glow
```

### Bronze / raised metal

This is intended for dark iron plaques such as the dungeon example:

```bash
./scripts/render_assets.sh \
  --text 'Vendo o que ninguém mais vê' \
  --font-family 'Noto Serif' \
  --style bronze-relief \
  16_9_dungeon_spider_iron_plaque
```

`bronze-relief.toml` combines procedural metallic fill, shadow, extrusion, outline, bevel, and moving shine.

### Gold shine

```toml
version = 2

[material]
type = "gold"

[[effects]]
type = "stroke"
width = 0.028
color = "#5A310EFF"

[[effects]]
type = "bevel"
width = 0.025

[[animations]]
type = "shine"
period_seconds = 2.8
width = 0.12
angle_degrees = 18
color = "#FFF9DEC8"
```

### Pulse

```toml
version = 1
fill = "#F6FFFFFF"

[[effects]]
type = "glow"
radius = 16
color = "#55EFFFF0"

[[animations]]
type = "pulse"
period_seconds = 2.2
minimum_opacity = 0.72
maximum_opacity = 1.0
```

## Material schema

Flat color:

```toml
fill = "#F4FFFFFF"
```

Linear gradient:

```toml
[material]
type = "linear-gradient"
top = "#FFF4D0FF"
bottom = "#8A4B18FF"
```

Procedural gold:

```toml
[material]
type = "gold"
dark = "#5B3210FF"
mid = "#C98B3CFF"
light = "#F3D38AFF"
highlight = "#FFF1C4FF"
```

## Static effect schema

```toml
[[effects]]
type = "shadow"
offset_x = 0.025       # fraction of fitted font size
offset_y = 0.035
blur_radius = 5        # final-output pixels
color = "#00000088"

[[effects]]
type = "stroke"
width = 0.030          # fraction of fitted font size
color = "#03181EE8"

[[effects]]
type = "glow"
radius = 12            # final-output pixels
color = "#69F2FA98"

[[effects]]
type = "extrude"
depth = 0.045          # fraction of fitted font size
angle_degrees = 62
color = "#3A1E0DDD"

[[effects]]
type = "bevel"
width = 0.030          # fraction of fitted font size
highlight = "#FFF0C0B8"
shadow = "#321707B8"
```

## Animation schema

Moving shine:

```toml
[[animations]]
type = "shine"
period_seconds = 3.2
width = 0.10           # fraction of projected title span
angle_degrees = 14
color = "#FFF4D0A8"
```

Pulse:

```toml
[[animations]]
type = "pulse"
period_seconds = 2.4
minimum_opacity = 0.82
maximum_opacity = 1.0
phase = 0.0            # cycles
```

## CLI growth rule

Complex effects belong in style files rather than flattening every artistic parameter into `RenderArgs`. Direct CLI flags remain for common fill/stroke/glow/shadow controls; named style files carry richer material/effect/animation stacks.

## Next renderer boundary

Wave/wobble cannot be implemented correctly by post-processing the complete title bitmap. They need reusable **prepared glyph geometry** so glyph transforms can vary per frame while line breaking remains static.

Likewise, convincing laser engraving belongs after plaque extraction because it must modulate the actual plaque pixels, not merely paint an RGBA title on top. That is the planned scene-surface stage rather than a fake alias for shadow/bevel.
