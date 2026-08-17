#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

usage() {
  cat <<'USAGE'
Usage:
  ./scripts/bakeoff_historical_quality.sh [options] [asset-stem ...]

Runs isolated visual-quality challengers through the CURRENT Plaque Forge implementation.
It never replaces assets/scenes, assets/analysis, output/*.mkv, or homologation contracts.

Options:
  --text TEXT                 Benchmark title text.
  --font PATH                 Font file.
  --font-family PATTERN       Fontconfig family/pattern (default: Noto Serif).
  --style NAME                Neutral style for geometry comparisons (default: classic-glow).
  --ml auto|on|off            Use the current segmentation worker (default: auto).
  --profile NAME              ML profile when ML is used (default: canonical).
  --precision NAME            ML precision when ML is used (default: fp32).
  --force-analysis            Rebuild bakeoff analysis even when a compatible cache exists.
  -h, --help                  Show this help.

With no asset names, the current recovery experiments are tested:
  rusty-plaque-with-object-in-front-parallax-and-plaque-moves
                                      current tracker vs reviewed v0.8 dense trajectory
  16_9_swamp_wooden_plaque          current vs recovered v0.8 geometry
  9_16_background_ogre_dear         current vs recovered v0.8 geometry
  9_16_dungeon_spider_iron_plaque   current vs recovered v0.8 geometry
  9_16_swamp_wooden_plaque          current vs recovered v0.8 geometry

The accepted 16:9 dungeon and moving-holographic champions have left the bakeoff.
Their remaining work belongs to canonical homologation or later generic algorithm improvement.

Results are written under output/quality-bakeoff/<asset>/.
USAGE
}

die() {
  printf 'error: %s\n' "$*" >&2
  exit 2
}

text="${QUALITY_BAKEOFF_TEXT:-Seeing what others cannot see!}"
font="${FONT:-}"
font_family="${FONT_FAMILY:-Noto Serif}"
neutral_style="${QUALITY_BAKEOFF_STYLE:-classic-glow}"
ml_mode="auto"
ml_profile="canonical"
ml_precision="fp32"
force_analysis=false
cases=()

while (( $# )); do
  case "$1" in
    --text) (( $# >= 2 )) || die "--text requires a value"; text="$2"; shift 2 ;;
    --font) (( $# >= 2 )) || die "--font requires a path"; font="$2"; shift 2 ;;
    --font-family) (( $# >= 2 )) || die "--font-family requires a value"; font_family="$2"; shift 2 ;;
    --style) (( $# >= 2 )) || die "--style requires a preset name"; neutral_style="$2"; shift 2 ;;
    --ml) (( $# >= 2 )) || die "--ml requires auto, on, or off"; ml_mode="$2"; shift 2 ;;
    --profile) (( $# >= 2 )) || die "--profile requires a value"; ml_profile="$2"; shift 2 ;;
    --precision) (( $# >= 2 )) || die "--precision requires a value"; ml_precision="$2"; shift 2 ;;
    --force-analysis) force_analysis=true; shift ;;
    -h|--help) usage; exit 0 ;;
    --) shift; cases+=("$@"); break ;;
    -*) die "unknown option: $1" ;;
    *) cases+=("$1"); shift ;;
  esac
done

case "$ml_mode" in auto|on|off) ;; *) die "--ml must be auto, on, or off" ;; esac

if (( ${#cases[@]} == 0 )); then
  cases=(
    rusty-plaque-with-object-in-front-parallax-and-plaque-moves
    16_9_swamp_wooden_plaque
    9_16_background_ogre_dear
    9_16_dungeon_spider_iron_plaque
    9_16_swamp_wooden_plaque
  )
fi

if [[ -z "$font" ]]; then
  font="$(fc-match -f '%{file}\n' "$font_family" | head -n 1)"
fi
[[ -n "$font" && -f "$font" ]] || die "font file not found: ${font:-<none>}"

neutral_style_file="styles/$neutral_style.toml"
[[ -f "$neutral_style_file" ]] || die "style preset not found: $neutral_style_file"
banded_bronze="styles/bronze-relief-banded.toml"
[[ -f "$banded_bronze" ]] || die "banded bronze style missing: $banded_bronze"

worker="tools/segmentation-worker"
ml_available=false
if [[ -x "$worker" && -x /tmp/plaque-forge-python/venv/bin/python && -f /tmp/plaque-forge-python/.complete ]]; then
  ml_available=true
fi
if [[ "$ml_mode" == on && "$ml_available" != true ]]; then
  die "--ml on requested, but the managed segmentation runtime is not ready; run ./scripts/setup_segmentation.sh"
fi
use_ml=false
if [[ "$ml_mode" == on || ( "$ml_mode" == auto && "$ml_available" == true ) ]]; then
  use_ml=true
fi

printf 'Quality recovery bakeoff\n'
printf '  current binary: current working tree\n'
printf '  text:           %s\n' "$text"
printf '  font:           %s\n' "$font"
printf '  neutral style:  %s\n' "$neutral_style_file"
printf '  ML:             %s\n' "$use_ml"
if [[ "$use_ml" == true ]]; then
  printf '  ML policy:      profile=%s precision=%s\n' "$ml_profile" "$ml_precision"
else
  printf '  warning: geometry results are pure-Rust; rerun with --ml on before promoting foreground-heavy cases\n' >&2
fi

cargo build --release --quiet
mkdir -p output/quality-bakeoff assets/.quality-bakeoff

TEMP_SCENES=()
cleanup() {
  local path
  for path in "${TEMP_SCENES[@]:-}"; do
    [[ -n "$path" ]] && rm -rf -- "$path"
  done
  rmdir assets/.quality-bakeoff 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# Rewrite only the first/default surface geometry. Everything else in the CURRENT scene,
# including prompts, depth semantics, and foreground policy, is preserved verbatim.
rewrite_scene_geometry() {
  local input="$1" output="$2" surface_bounds="$3" kind="$4"
  local writable_a="$5" writable_b="${6:-}" writable_c="${7:-}"

  awk \
    -v surface_bounds="$surface_bounds" \
    -v kind="$kind" \
    -v writable_a="$writable_a" \
    -v writable_b="$writable_b" \
    -v writable_c="$writable_c" '
      BEGIN {
        in_surface = 0;
        in_writable = 0;
        surface_done = 0;
        writable_a_done = 0;
        writable_b_done = 0;
        writable_c_done = 0;
      }
      /^\[\[surfaces\]\]$/ && !surface_done {
        in_surface = 1;
        in_writable = 0;
        print;
        next;
      }
      /^\[surfaces\.writable_region\]$/ && surface_done && !writable_a_done {
        in_surface = 0;
        in_writable = 1;
        print;
        next;
      }
      /^\[/ && $0 !~ /^\[\[surfaces\]\]$/ && $0 !~ /^\[surfaces\.writable_region\]$/ {
        in_surface = 0;
        if (in_writable) in_writable = 0;
      }
      in_surface && !surface_done && /^bounds[[:space:]]*=/ {
        print "bounds = " surface_bounds;
        surface_done = 1;
        next;
      }
      in_writable && kind == "rounded-rect" && !writable_a_done && /^bounds[[:space:]]*=/ {
        print "bounds = " writable_a;
        writable_a_done = 1;
        next;
      }
      in_writable && kind == "rounded-rect" && !writable_b_done && /^radius[[:space:]]*=/ {
        print "radius = " writable_b;
        writable_b_done = 1;
        next;
      }
      in_writable && kind == "ellipse" && !writable_a_done && /^center[[:space:]]*=/ {
        print "center = " writable_a;
        writable_a_done = 1;
        next;
      }
      in_writable && kind == "ellipse" && !writable_b_done && /^radii[[:space:]]*=/ {
        print "radii = " writable_b;
        writable_b_done = 1;
        next;
      }
      in_writable && kind == "ellipse" && !writable_c_done && /^rotation_degrees[[:space:]]*=/ {
        print "rotation_degrees = " writable_c;
        writable_c_done = 1;
        next;
      }
      { print }
      END {
        ok = surface_done && writable_a_done && writable_b_done;
        if (kind == "ellipse") ok = ok && writable_c_done;
        if (!ok) exit 42;
      }
    ' "$input" > "$output" || die "could not apply historical geometry to $input; scene schema changed"
}

HISTORICAL_SCENE=""
make_v08_scene() {
  local name="$1"
  local original="assets/scenes/$name/scene.toml"
  [[ -f "$original" ]] || die "scene not found: $original"

  local dir="assets/.quality-bakeoff/${name}-v0.8"
  local candidate="$dir/scene.toml"
  rm -rf -- "$dir"
  mkdir -p "$dir"
  TEMP_SCENES+=("$dir")

  case "$name" in
    16_9_swamp_wooden_plaque)
      rewrite_scene_geometry "$original" "$candidate" \
        '[314.0, 24.0, 622.0, 183.0]' rounded-rect \
        '[346.0, 53.0, 558.0, 116.0]' '18.0'
      ;;
    9_16_background_ogre_dear)
      rewrite_scene_geometry "$original" "$candidate" \
        '[20.0, 104.0, 494.0, 423.0]' ellipse \
        '[267.0, 315.5]' '[238.0, 198.0]' '0.0'
      ;;
    9_16_dungeon_spider_iron_plaque)
      rewrite_scene_geometry "$original" "$candidate" \
        '[142.0, 172.0, 437.0, 251.0]' rounded-rect \
        '[160.0, 190.0, 401.0, 215.0]' '22.0'
      ;;
    9_16_swamp_wooden_plaque)
      rewrite_scene_geometry "$original" "$candidate" \
        '[14.0, 0.0, 693.0, 394.0]' rounded-rect \
        '[58.0, 70.0, 605.0, 238.0]' '30.0'
      ;;
    *) die "no recovered v0.8 geometry challenger is registered for $name" ;;
  esac

  HISTORICAL_SCENE="$candidate"
}


HISTORICAL_TRAJECTORY_SCENE=""
make_v08_locked_trajectory_scene() {
  local name="$1"
  [[ "$name" == "rusty-plaque-with-object-in-front-parallax-and-plaque-moves" ]] ||
    die "v0.8 trajectory challenger is only authored for rusty-plaque-with-object-in-front-parallax-and-plaque-moves"
  local original="assets/scenes/$name/scene.toml"
  local input="assets/$name.mp4"
  [[ -f "$original" ]] || die "scene not found: $original"
  [[ -f "$input" ]] || die "input video not found: $input"

  local historical_motion="scripts/.quality-recovery/rusty-plaque-with-object-in-front-parallax-and-plaque-moves-v0.8-motion.json.gz"
  local historical_motion_sha expected_historical_motion_sha
  [[ -f "$historical_motion" ]] || die "recovered v0.8 trajectory input missing: $historical_motion"
  historical_motion_sha="$(sha256sum "$historical_motion" | awk '{print $1}')"
  expected_historical_motion_sha="34600cea073d7718d30ee48ede58d04b677db91d2694bb43764ef2e4aaca297e"
  [[ "$historical_motion_sha" == "$expected_historical_motion_sha" ]] ||
    die "recovered v0.8 trajectory input hash mismatch; refusing to use corrupted recovery data"

  local source_sha expected_sha
  source_sha="$(sha256sum "$input" | awk '{print $1}')"
  expected_sha="7415a7b03289eac8cb92fa6c7e0b9d4d1f44095c1eb70923c04481c574f7ba1e"
  [[ "$source_sha" == "$expected_sha" ]] ||
    die "rusty moving-plaque source hash differs from the v0.8 reviewed source; refusing to replay its trajectory"

  local dir="assets/.quality-bakeoff/${name}-v0.8-locked-trajectory"
  local candidate="$dir/scene.toml"
  local trajectory="$dir/trajectory.toml"
  rm -rf -- "$dir"
  mkdir -p "$dir"
  TEMP_SCENES+=("$dir")

  python3 - "$historical_motion" "$trajectory" "$source_sha" <<'PY_TRAJECTORY'
import gzip
import hashlib
import json
import math
import sys
from pathlib import Path

motion_path = Path(sys.argv[1])
output_path = Path(sys.argv[2])
source_sha = sys.argv[3]
raw = gzip.decompress(motion_path.read_bytes())
expected_raw_sha = "8c6012b35621477b4ab2f23a81c7cbf699a7efb477ea3f2a5bc435551511ae82"
raw_sha = hashlib.sha256(raw).hexdigest()
if raw_sha != expected_raw_sha:
    raise SystemExit(f"recovered v0.8 motion payload hash mismatch: {raw_sha}")
entries = json.loads(raw.decode("utf-8"))
if len(entries) != 240:
    raise SystemExit(f"expected 240 historical motion samples, got {len(entries)}")

# v0.8 used the same reviewed reference-frame plaque rectangle as the current scene.
x, y, width, height = 322.0, 46.0, 634.0, 133.0
corners = ((x, y), (x + width, y), (x + width, y + height), (x, y + height))

def project(matrix, point):
    px, py = point
    denominator = matrix[2][0] * px + matrix[2][1] * py + matrix[2][2]
    if not math.isfinite(denominator) or abs(denominator) < 1.0e-12:
        raise ValueError("historical trajectory contains a singular homography")
    qx = (matrix[0][0] * px + matrix[0][1] * py + matrix[0][2]) / denominator
    qy = (matrix[1][0] * px + matrix[1][1] * py + matrix[1][2]) / denominator
    if not math.isfinite(qx) or not math.isfinite(qy):
        raise ValueError("historical trajectory contains a non-finite projected point")
    return qx, qy

lines = [
    '# Temporary recovery artifact generated from the human-reviewed v0.8 analysis.\n',
    'format = "plaque-forge.trajectory/1"\n',
    'surface = "main"\n',
    'coordinates = "source-pixels"\n',
    f'source_sha256 = "{source_sha}"\n',
]
for expected_frame, entry in enumerate(entries):
    frame = int(entry["frame"])
    if frame != expected_frame:
        raise ValueError(f"historical motion frames are not dense at {expected_frame}: got {frame}")
    matrix = entry["transform"]["values"]
    quad = [project(matrix, corner) for corner in corners]
    visibility = float(entry.get("plaque_visibility", 1.0))
    lines.append('\n[[keyframes]]\n')
    lines.append(f'frame = {frame}\n')
    lines.append('quad = [\n')
    for qx, qy in quad:
        lines.append(f'  [{qx:.9f}, {qy:.9f}],\n')
    lines.append(']\n')
    lines.append('locked = true\n')
    lines.append(f'visibility = {min(1.0, max(0.0, visibility)):.9f}\n')
output_path.write_text(''.join(lines), encoding='utf-8')
PY_TRAJECTORY

  python3 - "$original" "$candidate" <<'PY_SCENE'
import sys
from pathlib import Path
source = Path(sys.argv[1]).read_text(encoding="utf-8").splitlines(keepends=True)
output = []
in_surface = False
inserted = False
for line in source:
    if line.strip() == "[[surfaces]]" and not inserted:
        in_surface = True
    elif in_surface and line.startswith("[") and line.strip() != "[[surfaces]]":
        output.append('trajectory = "trajectory.toml"\n')
        inserted = True
        in_surface = False
    output.append(line)
if in_surface and not inserted:
    output.append('trajectory = "trajectory.toml"\n')
    inserted = True
if not inserted:
    raise SystemExit("failed to locate the first surface table")
Path(sys.argv[2]).write_text(''.join(output), encoding="utf-8")
PY_SCENE
  HISTORICAL_TRAJECTORY_SCENE="$candidate"
}

analyze_variant() {
  local name="$1" tag="$2" scene="$3" out="$4"
  local analysis="$out/analysis"
  mkdir -p "$out"

  local mode=(--if-needed)
  if [[ "$force_analysis" == true ]]; then
    mode=(--force)
  fi

  local args=(
    analyze
    --input "assets/$name.mp4"
    --scene "$scene"
    --output "$analysis"
    --source-is-text-free
    "${mode[@]}"
    --progress always
  )
  if [[ "$use_ml" == true ]]; then
    args+=(
      --segmentation-worker "$worker"
      --segmentation-backend auto
      --segmentation-model auto
      --segmentation-device auto
      --segmentation-profile "$ml_profile"
      --segmentation-precision "$ml_precision"
    )
  fi

  printf '\n=== analyze %s / %s ===\n' "$name" "$tag"
  target/release/plaque-forge "${args[@]}"
  [[ -f "$analysis/manifest.toml" ]] || die "analysis did not publish a manifest: $analysis"
}

render_variant() {
  local name="$1" tag="$2" scene="$3" analysis="$4" style_file="$5" out="$6"
  local video="$out/$tag.lossless.mkv"
  local diagnostics="$out/$tag.render-diagnostics"
  local report="$out/$tag.verification.json"
  local verification_diagnostics="$out/$tag.verification-diagnostics"

  printf '\n=== render %s / %s ===\n' "$name" "$tag"
  target/release/plaque-forge render \
    --input "assets/$name.mp4" \
    --analysis "$analysis" \
    --scene "$scene" \
    --text "$text" \
    --font "$font" \
    --style-file "$style_file" \
    --diagnostics "$diagnostics" \
    --output "$video" \
    --fit artistic \
    --progress always

  # Verification is evidence, not a reason to stop the bakeoff. A challenger that fails
  # a threshold must remain visible beside its report rather than aborting the experiment.
  if ! target/release/plaque-forge verify \
      --analysis "$analysis" \
      --rendered "$video" \
      --original "assets/$name.mp4" \
      --report "$report" \
      --diagnostics "$verification_diagnostics" \
      --progress always; then
    printf 'verification failed for %s / %s; retained report/diagnostics for comparison\n' "$name" "$tag" >&2
  fi
}


render_variant_if_missing() {
  local name="$1" tag="$2" scene="$3" analysis="$4" style_file="$5" out="$6"
  if [[ -f "$out/$tag.lossless.mkv" && "$force_analysis" != true ]]; then
    printf '\n=== reuse render %s / %s ===\n' "$name" "$tag"
    return 0
  fi
  render_variant "$name" "$tag" "$scene" "$analysis" "$style_file" "$out"
}

side_by_side() {
  local left="$1" right="$2" output="$3"
  ffmpeg -y -hide_banner -loglevel error \
    -i "$left" -i "$right" \
    -filter_complex '[0:v][1:v]hstack=inputs=2[v]' \
    -map '[v]' -an -c:v libx264 -preset slow -crf 16 -pix_fmt yuv420p "$output"
}

four_up() {
  local a="$1" b="$2" c="$3" d="$4" output="$5"
  ffmpeg -y -hide_banner -loglevel error \
    -i "$a" -i "$b" -i "$c" -i "$d" \
    -filter_complex \
      '[0:v]scale=iw/2:ih/2[a];[1:v]scale=iw/2:ih/2[b];[2:v]scale=iw/2:ih/2[c];[3:v]scale=iw/2:ih/2[d];[a][b]hstack[top];[c][d]hstack[bottom];[top][bottom]vstack[v]' \
    -map '[v]' -an -c:v libx264 -preset slow -crf 16 -pix_fmt yuv420p "$output"
}

for name in "${cases[@]}"; do
  [[ -f "assets/$name.mp4" ]] || die "input video not found: assets/$name.mp4"
  current_scene="assets/scenes/$name/scene.toml"
  out="output/quality-bakeoff/$name"
  mkdir -p "$out"

  if [[ "$name" == "rusty-plaque-with-object-in-front-parallax-and-plaque-moves" ]]; then
    [[ "$use_ml" == true ]] ||
      die "rusty moving-plaque comparison requires --ml on (or --ml auto with a ready runtime) because the current scene has generated chain foregrounds"
    make_v08_locked_trajectory_scene "$name"
    historical_scene="$HISTORICAL_TRAJECTORY_SCENE"
    cp "$current_scene" "$out/current-tracker.scene.toml"
    cp "$historical_scene" "$out/v0.8-locked-trajectory.scene.toml"
    cp "$(dirname "$historical_scene")/trajectory.toml" "$out/v0.8-locked-trajectory.toml"
    {
      sha256sum "$current_scene" "$historical_scene" "$(dirname "$historical_scene")/trajectory.toml" "$neutral_style_file"
    } > "$out/input-sha256.txt"

    current_analysis="$out/current-tracker/analysis"
    historical_analysis="$out/v0.8-locked-trajectory/analysis"
    analyze_variant "$name" current-tracker "$current_scene" "$out/current-tracker"
    # The layer prompts and source bytes are identical. Seed the historical-trajectory
    # analysis with the current chain artifact and let provenance validation decide
    # whether it can be reused rather than blindly paying for the same ML pass twice.
    if [[ ! -f "$historical_analysis/manifest.toml" \
          && -d "$current_analysis/layers/chains" \
          && ! -d "$historical_analysis/layers/chains" ]]; then
      mkdir -p "$historical_analysis/layers"
      cp -a "$current_analysis/layers/chains" "$historical_analysis/layers/chains"
    fi
    analyze_variant "$name" v0.8-locked-trajectory "$historical_scene" "$out/v0.8-locked-trajectory"
    render_variant "$name" current-tracker "$current_scene" "$current_analysis" "$neutral_style_file" "$out"
    render_variant "$name" v0.8-locked-trajectory "$historical_scene" "$historical_analysis" "$neutral_style_file" "$out"
    side_by_side \
      "$out/current-tracker.lossless.mkv" \
      "$out/v0.8-locked-trajectory.lossless.mkv" \
      "$out/current-vs-v0.8-locked-trajectory.mp4"

    cat > "$out/README.txt" <<'EOF'
Rusty moving plaque trajectory recovery

LEFT  = current automatic tracker
RIGHT = dense locked trajectory reconstructed from the human-reviewed v0.8 analysis

Both candidates keep the current scene's chain prompts, current segmentation policy, current
renderer, and current verifier. The intended variable is only plaque trajectory. If the right-hand
candidate fixes the rotated/unstable title plane while chains remain wrong, that cleanly separates
the tracking regression from the foreground-compositing regression.

Review:
  current-vs-v0.8-locked-trajectory.mp4
  current-tracker.lossless.mkv
  v0.8-locked-trajectory.lossless.mkv
EOF
    continue
  fi


  make_v08_scene "$name"
  historical_scene="$HISTORICAL_SCENE"
  # Retain byte-for-byte experiment inputs even though the runnable historical scene lives
  # temporarily under assets/ so its existing project-relative references stay valid.
  cp "$current_scene" "$out/current-geometry.scene.toml"
  cp "$historical_scene" "$out/v0.8-geometry.scene.toml"
  {
    sha256sum "$current_scene" "$historical_scene" "$neutral_style_file" "$banded_bronze"
    if [[ -f styles/bronze-relief.toml ]]; then
      sha256sum styles/bronze-relief.toml
    fi
  } > "$out/input-sha256.txt"

  current_analysis="$out/current-geometry/analysis"
  historical_analysis="$out/v0.8-geometry/analysis"

  analyze_variant "$name" current-geometry "$current_scene" "$out/current-geometry"
  analyze_variant "$name" v0.8-geometry "$historical_scene" "$out/v0.8-geometry"

  render_variant "$name" current-geometry "$current_scene" "$current_analysis" "$neutral_style_file" "$out"
  render_variant "$name" v0.8-geometry "$historical_scene" "$historical_analysis" "$neutral_style_file" "$out"
  side_by_side \
    "$out/current-geometry.lossless.mkv" \
    "$out/v0.8-geometry.lossless.mkv" \
    "$out/geometry-side-by-side.mp4"

  if [[ "$name" == 9_16_dungeon_spider_iron_plaque ]]; then
    current_bronze="styles/bronze-relief.toml"
    [[ -f "$current_bronze" ]] || die "current bronze style missing: $current_bronze"

    render_variant "$name" current-geometry__current-bronze "$current_scene" "$current_analysis" "$current_bronze" "$out"
    render_variant "$name" current-geometry__banded-bronze "$current_scene" "$current_analysis" "$banded_bronze" "$out"
    render_variant "$name" v0.8-geometry__current-bronze "$historical_scene" "$historical_analysis" "$current_bronze" "$out"
    render_variant "$name" v0.8-geometry__banded-bronze "$historical_scene" "$historical_analysis" "$banded_bronze" "$out"

    four_up \
      "$out/current-geometry__current-bronze.lossless.mkv" \
      "$out/current-geometry__banded-bronze.lossless.mkv" \
      "$out/v0.8-geometry__current-bronze.lossless.mkv" \
      "$out/v0.8-geometry__banded-bronze.lossless.mkv" \
      "$out/dungeon-geometry-style-2x2.mp4"
  fi

  cat > "$out/README.txt" <<EOF
Historical quality bakeoff for: $name

Geometry preview:
  LEFT  = current authored scene geometry
  RIGHT = recovered v0.8 authored geometry
  file  = geometry-side-by-side.mp4

Every *.lossless.mkv has its own render provenance and verification evidence beside it.
No candidate is a winner merely because it came from an older commit. Promotion requires
visual review plus non-regressing verification/homologation.
EOF

  if [[ "$name" == 9_16_dungeon_spider_iron_plaque ]]; then
    cat >> "$out/README.txt" <<'EOF'

Dungeon 2x2 preview (dungeon-geometry-style-2x2.mp4):
  top-left     current geometry + current bronze-relief
  top-right    current geometry + bronze-relief-banded
  bottom-left  v0.8 geometry   + current bronze-relief
  bottom-right v0.8 geometry   + bronze-relief-banded
EOF
  fi

done

printf '\nBakeoff complete. Review output/quality-bakeoff/*/README.txt and the generated comparison previews.\n'
printf 'Do not promote a historical challenger until the same result is captured in a homologation contract.\n'
