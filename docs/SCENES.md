# Scene contract

`assets/scenes/<asset>/scene.toml` records authored artistic intent. It does not
contain analyzer conclusions, propagated masks, dense generated trajectories, or
quality scores.

```toml
format = "plaque-forge.scene/1"
source = "../../video.mp4"
default_surface = "main"

[[surfaces]]
id = "main"
space = "scene-plane"
depth = "automatic"
reference_frame = 10
bounds = [190.0, 100.0, 920.0, 180.0]

[surfaces.writable_region]
shape = "rounded-rect"
bounds = [220.0, 118.0, 860.0, 144.0]
radius = 28.0
```

All paths are relative to the TOML file. Absolute paths, platform-specific paths,
and any format other than `plaque-forge.scene/1` are rejected.

## Coordinate spaces

- `scene-plane` is a physical planar surface. Plaque Forge measures a four-corner
  projective pose on every frame, including perspective, rotation, scale, and
  partial off-screen motion. Missing observations are solved offline from evidence
  on both sides; they never turn the title into a screen-fixed overlay.
- `scene-mesh` reserves the contract for a genuinely non-planar title surface. The
  current analyzer rejects it until a mesh solver exists.
- `screen-canvas` is an intentional graphic canvas fixed to image coordinates. It
  must use `depth = "flat"`. It is not a fallback for difficult tracking.

`bounds` is the enclosing plane used for tracking. `writable_region` is the exact
shape allowed to receive typography. Keeping them separate lets the tracker use
stable material outside a circular or irregular writing area without allowing text
there.

## Depth

- `automatic` discovers foreground crossings and can refine them with the Python
  segmentation worker.
- `declared-only` uses only the layers declared in the scene.
- `flat` is valid only for `screen-canvas`; everything is deliberately on one plane.

Foreground layers restore their source pixels above the title. Background layers
are negative depth evidence and never erase the title. A complete source-pixel
writing-surface sequence is a per-pixel membership and depth constraint: trackers
may select reference points only on its visible material. The matte's breathing,
rounded, irregular, clipped, or foreground-cut outline is deliberately never
mistaken for rigid four-corner geometry. Persistent material points estimate the
projective plane; the noncausal solver bridges unsupported intervals. Its
`affects_layout` flag independently decides whether the matte also constrains
typography. Canonical writing-surface images affect layout only. Shadow, reflection,
and modulation layers preserve soft material relationships without becoming opaque
cutouts.

## Sparse reviewed anchors

Use anchors only when the automatic trajectory is visibly wrong. Four ordered
corners (`TL, TR, BR, BL`) capture a full planar homography:

```toml
[[surfaces.anchors]]
frame = 120
coordinates = "normalized"
quad = [[0.20, 0.30], [0.80, 0.29], [0.81, 0.61], [0.19, 0.62]]
locked = true
```

The offline solver uses the entire clip, so physically plausible acceleration can
begin before an observed turn. It does not wait for a jump and then imitate inertia.
A dense reviewed trajectory can be exported separately with `export-trajectory`,
but is exceptional generated review material, not the normal authoring format.

## Prompted foreground

```toml
[[layers]]
id = "lizard"
role = "foreground"
surface = "main"
in_front_of = "main"
affects_layout = false

[[layers.prompts]]
frame = 72
coordinates = "normalized"
object = "lizard"
box_bounds = [0.38, 0.27, 0.19, 0.20]
positive_points = [[0.48, 0.36]]
negative_points = [[0.59, 0.36]]
```

Prompted output is generated transactionally under `/tmp`, validated, and then
packaged once under `assets/analysis/<asset>/layers/`. Scene files never point at a
workstation cache. Masks are lossless 16-bit PNG when the ML worker produces soft
probabilities.

Foreground masks also declare their **matte semantics**. The default is `optical`:
the mask is literal measured transparency and soft values remain soft. Opaque objects
whose ML output represents semantic confidence rather than physical transparency use
`opaque`; confidence is calibrated into solid source restoration with a narrow feather:

```toml
[[layers]]
id = "spider"
role = "foreground"
surface = "main"
in_front_of = "main"
affects_layout = false
affects_tracking = false
matte = { mode = "opaque", support_threshold = 0.03, solid_threshold = 0.20 }
```

`affects_layout` and `affects_tracking` are independent on purpose. A foreground object
may need to appear above typography without shrinking the writable area or changing a
previously accepted plaque trajectory. This separation makes adding better compositing
evidence incapable of silently changing unrelated geometry unless the scene explicitly
opts into that coupling.

## Injected plaques

An image surface references an aspect-appropriate transparent PNG:

```toml
[[surfaces]]
id = "main"
space = "screen-canvas"
depth = "flat"
bounds = [180.0, 28.0, 920.0, 258.0]

[surfaces.appearance]
kind = "image"
image = "../../plaques/aetherglass-aurora-16_9.png"
inset = [0.08, 0.12, 0.08, 0.12]
```

The PNG alpha controls compositing; the inset or writable region controls title
placement. These are deliberately separate.

## Lifecycle

```text
assets/scenes/                 authored intent and small reviewed static masks
assets/analysis/               complete generated cache only
/tmp/plaque-forge/work/        private in-progress transactions
/tmp/plaque-forge/failures/    bounded compact failure evidence
```

`reset_analysis.sh --yes` deletes only the generated cache. A failed analysis never
publishes prompted masks or partial analysis data into the project.
