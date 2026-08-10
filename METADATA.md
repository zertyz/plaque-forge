# Plaque Forge metadata

Plaque Forge separates human-owned instructions from generated analysis.

| Artifact | Owner | May Plaque Forge overwrite it? |
|---|---|---|
| `video.plaque.toml` | Human | No |
| `video.<plaque>.track.toml` | Human after export | No |
| Dense layer assets referenced by the sidecar | Human or external tool | No |
| `.titlepack/` | Plaque Forge | Yes, during explicit reanalysis |

`init` and `export-track` create TOML with explanatory comments. Other commands
only read these files. `--force` is required to replace a human-owned file.

## Source sidecar

The default sidecar for `video.mp4` is `video.plaque.toml`:

```toml
schema_version = 1
source = "video.mp4"
default_plaque = "main"

[[plaques]]
id = "main"
reference_frame = 51
bounds = [65.0, 6.0, 905.0, 487.0]
motion_track = "video.main.track.toml"
```

`init` probes the source and writes its highest-ranked automatic candidate as
active `reference_frame` and `bounds` values. Up to three distinct candidates
from that reference frame are included as commented alternatives. Use
`--diagnostics <directory>` to retain the ranking and annotated frame.

Paths must be relative to the file containing them. Multiple `[[plaques]]`
entries are allowed. Use `default_plaque` or `--plaque <id>` to select one.

`bounds` identifies the plaque on `reference_frame`; it is not a fixed track.
Every source frame is still decoded and measured. Without bounds, detection
remains automatic.

The source sidecar is discovered automatically. `--metadata` selects another
file explicitly. Command-line bounds and motion-track paths override sidecar
values for that invocation.

## Segmentation prompts

Prompts are reserved for segmentation workers such as SAM 2:

```toml
[[plaques.prompts]]
frame = 51
box_bounds = [65.0, 6.0, 905.0, 487.0]
positive_points = [[400.0, 220.0]]
negative_points = [[40.0, 40.0]]
```

A positive point belongs to the target. A negative point marks nearby content
that must be excluded. Boxes, polygons, and four-corner quads are also accepted.
Prompts are validated in schema version 1 but are not yet sent to a segmentation
backend.

## Human motion tracks

A motion track stores source-pixel plaque corners:

```toml
schema_version = 2
plaque = "main"
coordinates = "source-pixels"
source_sha256 = "..."

[[keyframes]]
frame = 51
quad = [
  [65.0, 6.0],
  [970.0, 6.0],
  [970.0, 493.0],
  [65.0, 493.0],
]
locked = false
visibility = 1.0
```

Corner order is top-left, top-right, bottom-right, bottom-left.

- Sparse locked keyframes constrain an automatic measurement of every frame.
- A locked keyframe for every source frame is a fully authoritative track and
  bypasses automatic feature tracking.
- Unlocked keyframes supply starting estimates that plaque structural refinement
  may adjust.
- Schema version 2 permits generated guides and reviewed locked corrections in
  the same track. Locked corrections are reapplied after smoothing.
- Schema version 1 remains readable and retains its all-guided or all-locked rule.
- Authored visibility values are applied after automatic occlusion analysis,
  exactly match their keyframes, and interpolate as corrections between them.
  Keyframes without visibility anchor the automatic estimate unchanged.
- `--loop-closure on|off|auto` retains its meaning for guided, mixed, dense
  locked, and legacy CSV tracks.

Export a title-pack trajectory for review:

```bash
./target/release/plaque-forge export-track \
  --analysis video.titlepack \
  --output video.main.track.toml
```

The export contains one unlocked proposal per frame. Edit any incorrect quads
and set those reviewed entries to `locked = true`. Use `--locked` only when the
complete exported trajectory has already been reviewed and should be authoritative.
The plaque id defaults to the id recorded in the title-pack, then to `main` for a
pack without plaque metadata. `--plaque <id>` explicitly overrides that choice.

## Layer declarations

The sidecar can reserve portable references for dense foreground and material
artifacts:

```toml
[[layers]]
id = "foreground-branch"
role = "foreground"
plaque = "main"
in_front_of = "main"
artifact = "video.plaque-assets/foreground-branch.toml"
```

Roles are `foreground`, `background`, `reflection`, `shadow`, or `modulation`.
Schema version 1 validates these declarations. Dense RGBA/alpha artifact loading
and compositing are the next roadmap milestone and are not active yet.

## Precedence and cache identity

For plaque bounds:

1. `--plaque-hint` and `--plaque-frame`
2. selected sidecar plaque
3. first human motion keyframe
4. automatic detection

For TOML motion tracks:

1. `--motion-track`
2. selected sidecar plaque
3. automatic tracking

Legacy `--track-csv` overrides TOML tracks for compatibility. Normalized sidecar
and TOML-track hashes, explicit command-line plaque bounds, and the raw legacy
CSV hash are stored in title-pack format 4. Semantic changes cause `replace` to
reanalyze; comment-only TOML edits do not invalidate motion. Other analysis
controls still require `replace --reanalyze` when changing an existing cache.
