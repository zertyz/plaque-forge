# Text effects

Plaque Forge keeps text effects downstream of scene analysis. Changing a font, material,
animation, or plaque-surface treatment does **not** invalidate tracking/extraction caches.

```text
text + font
   ↓
shaping + artistic line layout
   ↓
layout transform     arc / orbital baseline
   ↓
glyph coverage
   ↓
underlays            shadow · stroke · glow · extrusion · chromatic split · trails
   ↓
material             flat · gradient · gold · chrome · holographic · fire · ice
                     nebula · liquid · halftone · blueprint · paper · image texture
   ↓
surface detail       bevel · letterpress
   ↓
frame presentation   pulse · shine · flicker · wave · typewriter · dissolve · glitch
                     scramble · split-flap · confetti convergence · orbit
   ↓
plaque interaction   laser burn / engraving · scene-sampled emboss/protrusion
   ↓
plaque warp + foreground restoration
```

## Capability coverage

The Rust text-art POCs are treated as a capability catalogue, not as an architectural
source. The production renderer now covers every effect family that was previously
listed as missing, while preserving the existing compositor and cache boundaries.

| Capability | Status | Production implementation |
|---|---|---|
| drop shadow | implemented | mask underlay |
| outline | implemented | stroke |
| neon glow / neon glass | implemented | stroke + glow |
| gradient fill | implemented | procedural material |
| gold / gilded metal | implemented | gold material + bevel/shine |
| chrome sheen | implemented | chrome material + shine |
| holographic foil / vaporwave | implemented | holographic material |
| bevel / raised lettering | implemented | bevel + extrusion |
| retro extrusion | implemented | repeated depth underlay |
| letterpress | implemented | reversed directional edge lighting |
| chromatic glitch / RGB split | implemented | RGB offset plus animated ripple/slice distortion |
| velocity trails | implemented | repeated directional underlays |
| halftone dots | implemented | halftone material |
| frosted ice | implemented | ice material |
| living fire | implemented | fire material |
| cosmic nebula | implemented | nebula material |
| liquid fill | implemented | liquid material |
| blueprint | implemented | drafting-grid procedural material |
| paper collage | implemented | fibrous paper material + registration offsets |
| pulse / moving shine / flicker | implemented | deterministic frame presentation |
| kinetic wave / wobble | implemented | cached-shaping raster deformation |
| typewriter reveal | implemented | reveal animation |
| particle dissolve | implemented | deterministic pixel dissolve |
| arc text | implemented | supersampled polar layout deformation after shaping |
| orbital typography | implemented | arc layout + deterministic orbit rotation |
| external image texture mapping | implemented | PNG texture material, clipped to glyph coverage and content-hashed |
| character scramble | implemented | discrete character states rendered at the selected fixed typography size and cached |
| split-flap | implemented | deterministic flap-character states with cached rendering |
| confetti convergence / particles | implemented | deterministic particle trajectories converging onto glyph samples |
| laser engraving / wood burn | implemented | per-frame plaque sampling + charred glyph/rim shading |
| protrusion / emboss lighting | implemented | plaque-sampled height-field shading + cast shadow, with automatic light-direction estimate |

The plaque-interaction effects are image-space height/shading models rather than a 3D
mesh simulation. Unlike the older overlay illusions, however, they sample and modify the
actual current plaque appearance before foreground restoration, so texture and frame
lighting remain visible in the effect.

## Bundled styles

Use a style by stem:

```bash
./scripts/render_assets.sh \
  --text 'Seeing what others cannot see!' \
  --font-family 'Noto Serif' \
  --style art-deco-arc
```

Notable presets:

- `art-deco-arc`
- `orbital-text`
- `texture-mapped`
- `scramble-reveal`
- `split-flap`
- `confetti-converge`
- `laser-burn-wood`
- `scene-emboss`
- `blueprint`
- `paper-collage`
- `chromatic-glitch`
- `classic-glow`
- `bronze-relief`
- `bronze-relief-banded`
- `gold-shine`
- `chrome-shine`
- `holographic-foil`
- `neon-pulse`
- `neon-flicker`
- `liquid-wave`
- `velocity-trails`
- `letterpress-wood`
- `frosted-ice`
- `living-fire`
- `cosmic-nebula`
- `halftone-pop`
- `typewriter`
- `particle-dissolve`

## Style schema version 4

Versions 1 through 3 remain accepted. Version 4 adds layout transforms, external
textures, character-state animations, particle convergence, animated distortion, and
plaque-surface effects.

### Arc / orbital layout

```toml
version = 4

[[layouts]]
type = "arc"
sweep_degrees = -55.0
radius_scale = 1.1

[[animations]]
type = "orbit"
period_seconds = 8.0
degrees_per_cycle = 360.0
```

Arc deformation happens on supersampled, already-shaped coverage. It therefore keeps
font shaping and line breaking upstream instead of splitting Unicode text into arbitrary
characters.

### External texture material

```toml
version = 4

[material]
type = "image-texture"
path = "../assets/textures/gilded-marble.png"
tile = true
scale = 0.42
offset_x = 0.0
offset_y = 0.0
```

The texture path is relative to the style file. The texture SHA-256 is included in the
resolved style description written to render provenance. The current project build
enables PNG decoding for image assets.

### Character animations

```toml
[[animations]]
type = "scramble"
period_seconds = 3.8
hold_fraction = 0.32
steps_per_second = 15.0
seed = 1396920910
```

```toml
[[animations]]
type = "split-flap"
period_seconds = 4.4
hold_fraction = 0.30
steps_per_second = 16.0
```

These are real character-state animations. Intermediate states are shaped/rasterized at
the already-selected title size and cached by state string. Scene analysis and line-fit
search are never rerun.

### Confetti convergence

```toml
[[animations]]
type = "confetti-converge"
period_seconds = 4.5
hold_fraction = 0.35
pieces = 900
spread = 0.52
seed = 1129270854
```

### Distortion / glitch

```toml
[[animations]]
type = "glitch"
period_seconds = 2.6
ripple = 0.018
slice = 0.085
burst_fraction = 0.20
seed = 1196185940
```

### Plaque-surface interaction

Laser burn:

```toml
[[surface_effects]]
type = "laser-burn"
depth = 0.82
warmth = 0.72
edge_width = 2
seed = 1112887886
```

Emboss/protrusion:

```toml
[[surface_effects]]
type = "emboss"
depth = 0.82
highlight_strength = 0.78
shadow_strength = 0.74
cast_shadow = 3
```

When `light_angle_degrees` is omitted, the emboss effect estimates a dominant image-space
light direction from the current canonical plaque image. It may also be specified
explicitly.

## CLI rule

Simple typography remains directly configurable from the CLI. Rich effects belong in
`styles/*.toml`; Plaque Forge deliberately does not flatten every artistic parameter into
dozens of top-level switches.
