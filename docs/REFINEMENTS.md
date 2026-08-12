# Refinements

A refinement is **small human intent that automatic analysis must honor**. It is the fallback when `./scripts/analyze_assets.sh` cannot reach a trustworthy result by itself.

The editable file is:

```text
assets/refinements/<asset>/refinement.toml
```

Do not copy generated per-frame state into it. Dense tracks, propagated masks, previews, and model outputs are artifacts/caches.

## Normal workflow

Run analysis first:

```bash
./scripts/analyze_assets.sh my-video
```

If a quality gate fails, the script now creates both:

```text
assets/analysis/<name>.partial-.../diagnostics/review.html
assets/analysis/<name>.partial-.../diagnostics/review.txt
```

Open the HTML first. It orders the likely problems and shows the relevant visual evidence. The coordinate helper lets you click an image and copy normalized points instead of calculating coordinates.

Only then add the smallest correction needed.

## Minimal rectangular surface

Schema 2 remains compatible with schema-1 files.

```toml
schema_version = 2
source = "../../video.mp4"
default_plaque = "main"

[[plaques]]
id = "main"
reference_frame = 10
bounds = [190.0, 100.0, 920.0, 180.0]
```

`bounds` is `[x, y, width, height]` in source pixels. It is the enclosing planar region used for tracking.

## Placement and writable region are different

A surface may be easy to track through an enclosing rectangle while only part of it may receive text. `writable_region` declares that inner shape.

### Rounded rectangle

```toml
[plaques.writable_region]
shape = "rounded-rect"
bounds = [190.0, 100.0, 920.0, 180.0]
radius = 28.0
```

### Circle / oval

```toml
[plaques.writable_region]
shape = "ellipse"
center = [480.0, 330.0]
radii = [390.0, 270.0]
rotation_degrees = 0.0
```

A circle is an ellipse with equal radii.

### Polygon

```toml
[plaques.writable_region]
shape = "polygon"
points = [
  [210.0, 70.0],
  [1060.0, 75.0],
  [1110.0, 230.0],
  [180.0, 225.0],
]
```

### Arbitrary mask

Use this only when a simple shape is insufficient, such as a cloud silhouette.

```toml
[plaques.writable_region]
shape = "mask"
bounds = [180.0, 40.0, 930.0, 210.0]
path = "cloud-mask.png"
```

White is writable, black is forbidden, and gray feathers the boundary.

When both `bounds` and `writable_region` exist, `bounds` controls the outer tracked plane and `writable_region` controls typography. When only `writable_region` exists, its enclosing bounds are also used for tracking.

## Sparse motion correction

Do **not** edit or maintain a quad for every frame. Add anchors only at frames where automatic tracking is visibly wrong.

Normalized coordinates keep the human correction independent of video resolution:

```toml
[[plaques.motion]]
frame = 120
coordinates = "normalized"
quad = [
  [0.206, 0.302], # top-left
  [0.802, 0.296], # top-right
  [0.811, 0.612], # bottom-right
  [0.198, 0.618], # bottom-left
]
locked = true
```

The tracker solves the frames between sparse anchors. `locked = false` provides a guide rather than an exact constraint.

Legacy external `motion_track = "..."` files remain supported. New `export-motion` output goes under `artifacts/motion.toml` because a dense exported track is generated review material, not ordinary human intent.

## Sparse foreground prompts

Use a prompt only when automatic foreground recovery misses an object that should cross in front of the text/plaque.

Prefer normalized coordinates:

```toml
[[layers]]
id = "foreground"
role = "foreground"
plaque = "main"
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

The static review page has a click helper that reports normalized and source-pixel coordinates. Plaque Forge converts normalized prompts to source pixels before invoking the external segmentation worker.

Existing schema-1 prompts with no `coordinates` field continue to mean source pixels.

## Injected plaque for plaque-less video

Use the high-level helper:

```bash
./scripts/place_plaque.sh 16_9_plaqueless_swamp my-plaque.png
```

It copies/normalizes the PNG, proposes a quiet placement, writes `placement-preview.png`, and creates a small refinement. Then use the ordinary analyzer and renderer:

```bash
./scripts/analyze_assets.sh 16_9_plaqueless_swamp
./scripts/render_assets.sh --text 'Title' --font-family 'Noto Serif' 16_9_plaqueless_swamp
```

The generated intent resembles:

```toml
schema_version = 2
source = "../../16_9_plaqueless_swamp.mp4"
default_plaque = "main"

[[plaques]]
id = "main"
reference_frame = 0
bounds = [180.0, 70.0, 900.0, 220.0]

[plaques.surface]
type = "injected"
image = "injected-plaque.png"
motion = "auto"
inset = [0.08, 0.12, 0.08, 0.12]
```

`motion` is `auto`, `screen`, or `scene`. The PNG hash and placement semantics participate in cache identity. Plaque detection/source-plaque extraction are skipped, but scene motion and foreground crossings are still analyzed.

## Generated artifacts

Generated data belongs outside the short human manifest. Current and legacy artifact paths remain readable for compatibility. The direction is:

```text
assets/refinements/<scene>/
  refinement.toml
  injected-plaque.png          # human-supplied source asset, when applicable
  artifacts/
    motion.toml                # dense export/review material
    layers/<name>/             # newly generated prompted ML layers
    ...
```

New implicit prompted-layer outputs also go under `artifacts/layers/`; explicit/legacy artifact paths remain supported. The analysis cache under `assets/analysis/` remains the canonical home for reusable machine analysis state.

## Guiding rule

When analysis fails, correct **intent**, in this order:

1. intended writing surface / writable shape;
2. only the motion frames that are wrong;
3. only foreground objects the automatic masks miss.

Do not repair generated machine state by hand unless you explicitly need an authoritative external track.
