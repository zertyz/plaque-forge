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

Prompts seed external segmentation workers:

```toml
[[plaques.prompts]]
frame = 51
box_bounds = [65.0, 6.0, 905.0, 487.0]
positive_points = [[400.0, 220.0]]
negative_points = [[40.0, 40.0]]
```

A positive point belongs to the target. A negative point excludes nearby content.
Boxes, polygons, and quads are also accepted.

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

## Layer artifacts

The sidecar can reserve portable references for dense foreground and material
artifacts:

```toml
[[layers]]
id = "foreground-moss"
role = "foreground"
plaque = "main"
in_front_of = "main"
artifact = "video.plaque-assets/foreground-moss.toml"
affects_layout = true
active_frames = [20, 180]

[[layers.prompts]]
frame = 51
object = "moss-left"
box_bounds = [65.0, 6.0, 200.0, 120.0]
```

`foreground` restores source pixels over the title and supersedes automatic
occlusion masks. `shadow` restores partial source alpha without changing layout.
`writing-surface` is an inclusion mask used only for layout. Other roles remain
reserved. Set `affects_layout = false` when text may run behind a foreground
layer. Prompts sharing an `object` name correct the same target over time;
different names produce one combined layer mask. `active_frames` bounds an
intermittent object's propagation and emits zero alpha outside the interval.

A plaque-attached mask uses one canonical PNG:

```toml
schema_version = 1
kind = "alpha-image"
coordinates = "plaque-canonical"
path = "foreground-moss.png"
affects_layout = true
```

Moving foreground uses source-sized frames:

```toml
schema_version = 1
kind = "alpha-sequence"
coordinates = "source-pixels"
pattern = "foreground/%06d.png"
first_frame = 0
last_frame = 239
```

```bash
plaque-forge segment --input video.mp4 --metadata video.plaque.toml \
  --layer foreground-branch --worker tools/segmentation_worker.py \
  --backend sam2-cutie-vitmatte --model facebook/sam2.1-hiera-large \
  --device auto \
  --output video.plaque-assets/foreground-branch
```

The worker supports `sam2-vitmatte`, `cutie-vitmatte`,
`sam2-cutie-vitmatte`, and the human-matting `matanyone2` backend. It writes
`artifact.toml`, `result.json`, and the alpha sequence. Model dependencies stay
outside the Rust build.

### Python worker setup

This example keeps Python, sources, weights, and caches under one removable root:

```bash
ROOT=/tmp/plaque-forge-python
mkdir -p "$ROOT"/{bin,cache,config,data,home,src,tmp}
export HOME="$ROOT/home" XDG_CACHE_HOME="$ROOT/cache/xdg"
export XDG_CONFIG_HOME="$ROOT/config" XDG_DATA_HOME="$ROOT/data"
export UV_CACHE_DIR="$ROOT/cache/uv" UV_PYTHON_INSTALL_DIR="$ROOT/python"
export UV_PYTHON_BIN_DIR="$ROOT/bin" PIP_CACHE_DIR="$ROOT/cache/pip"
export HF_HOME="$ROOT/cache/huggingface" TORCH_HOME="$ROOT/cache/torch"
export MPLCONFIGDIR="$ROOT/cache/matplotlib" TRITON_CACHE_DIR="$ROOT/cache/triton"
export TORCHINDUCTOR_CACHE_DIR="$ROOT/cache/torchinductor" TMPDIR="$ROOT/tmp"

uv python install 3.10
uv venv --python 3.10 --seed "$ROOT/venv"
PY="$ROOT/venv/bin/python"
"$PY" -m pip install torch==2.13.0+xpu torchvision==0.28.0+xpu \
  --index-url https://download.pytorch.org/whl/xpu
"$PY" -m pip install -r tools/segmentation-requirements.txt

git clone https://github.com/facebookresearch/sam2.git "$ROOT/src/sam2"
git -C "$ROOT/src/sam2" checkout 2b90b9f5ceec907a1c18123530e92e794ad901a4
SAM2_BUILD_CUDA=0 "$PY" -m pip install --no-deps --no-build-isolation -e "$ROOT/src/sam2"
git clone https://github.com/hkchengrex/Cutie.git "$ROOT/src/Cutie"
git -C "$ROOT/src/Cutie" checkout ec5cdd4cf16f75c73ad785a2f96fb97dbad4125a
"$PY" -m pip install --no-deps -e "$ROOT/src/Cutie"
git clone https://github.com/pq-yang/MatAnyone2.git "$ROOT/src/MatAnyone2"
git -C "$ROOT/src/MatAnyone2" checkout 0079197acd6d16a741f71558809c06c586c579e0
"$PY" -m pip install --no-deps -e "$ROOT/src/MatAnyone2"

"$PY" "$ROOT/src/Cutie/cutie/utils/download_models.py"
"$PY" -c 'from huggingface_hub import snapshot_download as d; [d(repo_id=x) for x in ("facebook/sam2.1-hiera-large", "hustvl/vitmatte-small-composition-1k", "PeiqingYang/MatAnyone2")]'
export PATH="$ROOT/venv/bin:$PATH"
```

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

Legacy `--track-csv` overrides TOML tracks. Normalized sidecar, track, and layer
content hashes, explicit plaque bounds, and legacy CSV contents are stored in
title-pack format 6. Semantic changes cause `replace` to
reanalyze; comment-only TOML edits do not invalidate motion. Other analysis
controls still require `replace --reanalyze` when changing an existing cache.
