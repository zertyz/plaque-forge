# Refinements

A refinement is **human intent that automatic analysis must honor**. It is used only when automation cannot identify or track the intended writing surface reliably enough.

The human-facing file is `assets/refinements/<asset>/refinement.toml`. Dense per-frame motion and alpha masks may be generated artifacts; a person should not have to type them by hand.

## Writing surface: placement vs writable mask

Plaque Forge separates two ideas:

1. **placement / pose** — the enclosing planar region that is tracked through the video;
2. **writable mask** — the pixels inside that region where typography is allowed.

The tracker can therefore keep using a planar enclosing rectangle/quad while the writable region itself is rectangular, rounded, elliptical, polygonal, or arbitrary.

Simple human declarations are compiled into the general mask internally.

### Rectangle

Legacy `bounds` remains the shortest rectangular form:

```toml
schema_version = 1
source = "../../video.mp4"
default_plaque = "main"

[[plaques]]
id = "main"
reference_frame = 10
bounds = [190.0, 100.0, 920.0, 180.0] # x, y, width, height
```

### Rounded rectangle

```toml
[[plaques]]
id = "main"
reference_frame = 10

[plaques.writable_region]
shape = "rounded-rect"
bounds = [190.0, 100.0, 920.0, 180.0]
radius = 28.0
```

### Circle / oval

This is the convenient human form for circular or elliptical title areas. `radii` are X/Y radii in source pixels; rotation is optional.

```toml
[[plaques]]
id = "main"
reference_frame = 10

[plaques.writable_region]
shape = "ellipse"
center = [480.0, 330.0]
radii = [390.0, 270.0]
rotation_degrees = 0.0
```

A circle is simply an ellipse with equal radii.

### Polygon

```toml
[[plaques]]
id = "main"
reference_frame = 10

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

Use this only when a simple shape does not express the intended writing region. The PNG is local to the enclosing `bounds`; it is resized to canonical analysis dimensions if necessary. White means writable, black means forbidden, gray feathers the boundary.

```toml
[[plaques]]
id = "main"
reference_frame = 10

[plaques.writable_region]
shape = "mask"
bounds = [180.0, 40.0, 930.0, 210.0]
path = "cloud-mask.png"
```

When `bounds` and `writable_region` are both present, `bounds` is the outer planar tracking/placement enclosure and `writable_region` is the possibly smaller/non-rectangular area that may receive text. When only `writable_region` is present, its enclosing bounds are also used for tracking.

## Injected plaque for plaque-less video

An injected plaque is a different **surface source**, not a fake detection result. Because placement and appearance are supplied explicitly, automatic plaque selection and recovery of a source-plaque appearance are skipped. Plaque Forge still reconstructs/analyzes the underlying video region and foreground crossings because those are needed to composite the virtual plaque convincingly.

Prefer the high-level helper:

```bash
./scripts/place_plaque.sh 16_9_plaqueless_swamp my-plaque.png
```

It copies the image into `assets/refinements/<asset>/injected-plaque.png`, proposes a low-activity placement from several frames, and writes `placement-preview.png`. Override the proposal when desired:

```bash
./scripts/place_plaque.sh 16_9_plaqueless_swamp my-plaque.png \
  --bounds 180,70,900,220 --motion screen --inset 0.08,0.12,0.08,0.12
```

The generated human intent is small:

```toml
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

`motion` may be:

- `auto`: try scene anchoring and fall back to screen-fixed placement if it is unreliable;
- `screen`: explicitly fixed in screen coordinates;
- `scene`: require scene anchoring and fail rather than silently fall back.

The image content hash and the refinement semantics participate in analysis-cache identity. Replacing the PNG or changing placement/inset therefore invalidates the dependent cache, while changing title text or typography style does not. An explicit `writable_region` may replace the simple inset when the PNG has an irregular writing area.

Automatic temporal analysis can recover moving objects that cross the injected surface, but a plaque-less source contains no physical depth cue saying whether every static/ambiguous object should be in front of or behind the new plaque. For those cases, declare a foreground layer/segmentation prompt. The placement proposer deliberately favors quieter regions to reduce this ambiguity.

## Automatic first, refinement second

Run:

```bash
./scripts/analyze_assets.sh my-video
```

The analyzer first tries to discover the writing surface itself. Candidate detection now considers broad planar enclosures rather than assuming that the writable pixels form a rectangle; canonical extraction derives its own content mask. Rounded/oval/large surfaces are therefore allowed to emerge automatically.

If confidence is still insufficient, inspect the retained diagnostics and add the smallest correction that resolves the ambiguity. A manually declared `writable_region` is authoritative for where text may be drawn, while tracking/extraction/occlusion quality is still validated independently.

## Motion refinement

A motion track constrains how the enclosing planar region moves. Export the automatic track:

```bash
plaque-forge export-motion --analysis assets/analysis/video
```

The generated form is:

```toml
schema_version = 1
plaque = "main"
coordinates = "source-pixels"
source_sha256 = "..."

[[keyframes]]
frame = 51
quad = [[65.0, 6.0], [970.0, 6.0], [970.0, 493.0], [65.0, 493.0]]
locked = true
visibility = 1.0
```

Corners are top-left, top-right, bottom-right, bottom-left. Prefer a few locked corrections over locking every frame. Dense tracks are generated machine state, not the desired human interface.

## Foreground / segmentation layers

A layer may declare segmentation prompts. After the one-time temporary Python environment is installed with `./scripts/setup_segmentation.sh`, the **high-level `analyze_assets.sh` command automatically generates any prompted layer whose artifact is missing**. You do not normally need to invoke `segment` yourself.

Example layer intent:

```toml
[[layers]]
id = "foreground"
role = "foreground"
plaque = "main"
in_front_of = "main"
affects_layout = false

[[layers.prompts]]
frame = 72
object = "lizard"
box_bounds = [490.0, 145.0, 235.0, 105.0]
positive_points = [[615.0, 190.0]]
negative_points = [[760.0, 190.0]]
```

If `artifact` is omitted, Plaque Forge uses the conventional generated location `<refinement-dir>/<layer>/artifact.toml`. An explicit artifact path is still supported.

The isolated worker environment, Hugging Face/Torch caches, cloned model repositories, and synthetic `$HOME` all live under `/tmp/plaque-forge-python`.

## Generated layer artifacts

Canonical image:

```toml
schema_version = 1
kind = "alpha-image"
coordinates = "plaque-canonical"
path = "moss.png"
affects_layout = false
```

Source-pixel sequence:

```toml
schema_version = 1
kind = "alpha-sequence"
coordinates = "source-pixels"
pattern = "masks/%06d.png"
first_frame = 0
last_frame = 239
affects_layout = false
```

Soft alpha is preserved. Foreground layers restore source pixels over the title; shadow layers restore their alpha-weighted source contribution.
