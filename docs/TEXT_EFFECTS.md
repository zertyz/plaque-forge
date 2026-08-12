# Text effects

Plaque Forge separates text layout, coverage effects, materials, frame presentation, and scene compositing. This keeps effects out of scene-analysis caches and lets static typography be reused across frames.

```text
text + font
   ↓
shaping + artistic line layout
   ↓
glyph coverage
   ↓
underlays       shadow · stroke · glow · extrusion · chromatic split · trails
   ↓
material        flat · gradient · gold · chrome · holographic · fire · ice · nebula · liquid · halftone
   ↓
surface detail  bevel · letterpress/recessed edge
   ↓
frame effects   pulse · shine · flicker · raster wave/wobble · typewriter · dissolve
   ↓
plaque warp + foreground restoration
```

Changing a text style does not invalidate writing-surface tracking/extraction caches.

## Capability coverage

The experimental Rust text-art POCs were treated as a **capability catalogue, not an architecture**. 0.8 covers most of their reusable visual primitives without importing their UI/rendering frameworks.

| Capability | 0.8 status | Production implementation |
|---|---|---|
| drop shadow | implemented | mask underlay |
| outline | implemented | stroke |
| neon glow / neon glass | implemented | stroke + strong glow |
| gradient fill | implemented | procedural material |
| gold / gilded metal | implemented | banded gold material + optional bevel/shine |
| chrome sheen | implemented | chrome material + shine |
| holographic foil / vaporwave color | implemented | procedural holographic material |
| bevel / raised lettering | implemented | bevel + extrusion |
| retro extrusion / 3D depth illusion | implemented | repeated depth underlay |
| letterpress / recessed lettering | implemented | reversed directional edge lighting |
| chromatic glitch / RGB split | implemented | chromatic offset underlay |
| velocity trails | implemented | repeated directional underlays |
| halftone dots | implemented | halftone material |
| frosted ice | implemented | ice material + bevel/glow preset |
| living fire | implemented | fire material + glow preset |
| cosmic nebula | implemented | procedural nebula material |
| liquid fill | implemented | procedural liquid material |
| pulse | implemented | opacity animation |
| moving shine | implemented | glyph-constrained animated highlight |
| neon flicker | implemented | deterministic opacity flicker |
| kinetic wave / liquid wobble | implemented as raster deformation | cached shaping; final prepared title is warped per frame |
| typewriter reveal | implemented as raster reveal | horizontal animated reveal |
| particle dissolve | implemented as deterministic pixel dissolve | no particle trajectories |
| texture-color experimentation | covered by procedural materials | not arbitrary external image mapping |
| true arc text / orbital typography | **not implemented** | requires retained per-glyph geometry |
| arbitrary image texture mapping | **not implemented** | needs texture asset/provenance stage |
| character scramble / split-flap | **not implemented** | requires per-character temporal state |
| confetti convergence / real particles | **not implemented** | requires particle simulation/state |
| physically correct plaque engraving / laser burn | **not implemented** | requires plaque-surface/material interaction |
| physically correct 3D protrusion + scene lighting | **not implemented** | requires scene-aware surface/lighting model |

The last group is intentionally not faked behind misleading names. `letterpress-wood` gives a useful recessed/engraved **illusion**, and extrusion+bevel gives useful raised depth, but neither claims physical plaque deformation.

## Bundled styles

Use a style by stem:

```bash
./scripts/render_assets.sh \
  --text 'Seeing what others cannot see!' \
  --font-family 'Noto Serif' \
  --style holographic-foil
```

Bundled presets include:

- `classic-glow`
- `bronze-relief`
- `gold-shine`
- `chrome-shine`
- `holographic-foil`
- `neon-pulse`
- `neon-flicker`
- `liquid-wave`
- `chromatic-glitch`
- `velocity-trails`
- `letterpress-wood`
- `frosted-ice`
- `living-fire`
- `cosmic-nebula`
- `halftone-pop`
- `typewriter`
- `particle-dissolve`

## Style schema

Styles are TOML. Versions 1 and 2 remain compatible; the new families use version 3.

### Materials

```toml
version = 3

[material]
type = "chrome"       # also holographic, fire, ice, nebula
```

Liquid:

```toml
[material]
type = "liquid"
first = "#29F4D5FF"
second = "#3958F8FF"
frequency = 3.4
```

Halftone:

```toml
[material]
type = "halftone"
foreground = "#FFF1B8FF"
background = "#F23C70FF"
cell = 7
```

Existing `linear-gradient` and `gold` materials remain supported.

### Coverage / depth effects

```toml
[[effects]]
type = "chromatic-split"
offset = 0.045
red = "#FF2450C0"
cyan = "#25F4FFC0"

[[effects]]
type = "trail"
distance = 0.28
copies = 9
angle_degrees = 180
color = "#46D8FF68"

[[effects]]
type = "letterpress"
width = 0.055
highlight = "#FFE1A870"
shadow = "#090504E8"
```

Stroke, glow, shadow, extrusion and bevel remain unchanged.

### Animation / reveal effects

```toml
[[animations]]
type = "flicker"
period_seconds = 1.7
minimum_opacity = 0.68
strength = 0.42

[[animations]]
type = "wave"
period_seconds = 2.8
amplitude = 0.030
wavelength = 0.46

[[animations]]
type = "typewriter"
period_seconds = 4.2
hold_fraction = 0.32

[[animations]]
type = "dissolve"
period_seconds = 4.0
hold_fraction = 0.35
seed = 1347174737
```

Pulse and moving shine remain available.

## CLI rule

Simple typography remains directly configurable from the CLI. Rich artistic stacks belong in `styles/*.toml`; Plaque Forge deliberately does not flatten every material and animation parameter into dozens of top-level flags.
