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
  16_9_dungeon_spider_iron_plaque   current geometry vs crossing-web-aware scene
  16_9_swamp_wooden_plaque          current vs recovered v0.8 geometry
  9_16_background_ogre_dear         current vs recovered v0.8 geometry
  9_16_dungeon_spider_iron_plaque   current vs recovered v0.8 geometry
  9_16_swamp_wooden_plaque          current vs recovered v0.8 geometry

The 16:9 dungeon experiment keeps the retained current geometry fixed and compares
foreground-web semantics under classic-glow, bronze-relief, and bronze-relief-banded.

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
    16_9_dungeon_spider_iron_plaque
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


WEB_SCENE=""
make_crossing_web_scene() {
  local name="$1"
  [[ "$name" == "16_9_dungeon_spider_iron_plaque" ]] || die "crossing-web challenger is only authored for 16_9_dungeon_spider_iron_plaque"
  local original="assets/scenes/$name/scene.toml"
  [[ -f "$original" ]] || die "scene not found: $original"

  local dir="assets/.quality-bakeoff/${name}-crossing-web"
  local candidate="$dir/scene.toml"
  rm -rf -- "$dir"
  mkdir -p "$dir"
  TEMP_SCENES+=("$dir")
  cp "$original" "$candidate"
  cat >> "$candidate" <<'EOF'

# Quality-recovery challenger: the translucent web crosses in front of the title
# plane. Optical alpha preserves the strands while affects_tracking excludes those
# pixels from plaque motion evidence during the crossing.
[[layers]]
id = "crossing-web"
role = "foreground"
surface = "main"
in_front_of = "main"
active_frames = [94, 136]
affects_layout = false
affects_tracking = true
matte = { mode = "optical", support_threshold = 0.03, solid_threshold = 0.20 }

[[layers.prompts]]
frame = 95
coordinates = "source-pixels"
object = "crossing-web"
box_bounds = [0.0, 0.0, 520.0, 720.0]
positive_points = [[70.0, 55.0], [160.0, 25.0], [250.0, 115.0], [105.0, 240.0], [210.0, 360.0], [280.0, 520.0]]
negative_points = [[470.0, 250.0], [470.0, 600.0]]

[[layers.prompts]]
frame = 105
coordinates = "source-pixels"
object = "crossing-web"
box_bounds = [0.0, 0.0, 700.0, 720.0]
positive_points = [[90.0, 60.0], [230.0, 90.0], [360.0, 150.0], [130.0, 280.0], [260.0, 380.0], [390.0, 520.0]]
negative_points = [[650.0, 250.0], [650.0, 650.0]]

[[layers.prompts]]
frame = 115
coordinates = "source-pixels"
object = "crossing-web"
box_bounds = [0.0, 0.0, 1050.0, 720.0]
positive_points = [[120.0, 40.0], [300.0, 80.0], [520.0, 120.0], [760.0, 90.0], [250.0, 300.0], [520.0, 390.0], [760.0, 520.0]]
negative_points = [[1030.0, 300.0], [1000.0, 650.0]]

[[layers.prompts]]
frame = 125
coordinates = "source-pixels"
object = "crossing-web"
box_bounds = [450.0, 0.0, 830.0, 720.0]
positive_points = [[620.0, 40.0], [780.0, 90.0], [960.0, 120.0], [1130.0, 180.0], [720.0, 320.0], [930.0, 420.0], [1120.0, 560.0]]
negative_points = [[480.0, 300.0], [500.0, 650.0]]

[[layers.prompts]]
frame = 135
coordinates = "source-pixels"
object = "crossing-web"
box_bounds = [900.0, 0.0, 380.0, 720.0]
positive_points = [[1030.0, 40.0], [1170.0, 100.0], [1230.0, 230.0], [1130.0, 380.0], [1210.0, 540.0]]
negative_points = [[900.0, 250.0], [900.0, 650.0]]
EOF
  WEB_SCENE="$candidate"
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

  if [[ "$name" == "16_9_dungeon_spider_iron_plaque" ]]; then
    make_crossing_web_scene "$name"
    web_scene="$WEB_SCENE"
    cp "$current_scene" "$out/current-geometry.scene.toml"
    cp "$web_scene" "$out/current-geometry-web-aware.scene.toml"
    {
      sha256sum "$current_scene" "$web_scene" "$neutral_style_file" "$banded_bronze"
      sha256sum styles/bronze-relief.toml
    } > "$out/input-sha256.txt"

    current_analysis="$out/current-geometry/analysis"
    web_analysis="$out/current-geometry-web-aware/analysis"
    if [[ "$force_analysis" == true || ! -f "$current_analysis/manifest.toml" ]]; then
      analyze_variant "$name" current-geometry "$current_scene" "$out/current-geometry"
    else
      printf '\n=== reuse analysis %s / current-geometry ===\n' "$name"
    fi
    # Seed the challenger with the already-reviewed spider layer so the analyzer can
    # deterministically reuse that expensive prompted artifact while generating only
    # the new crossing-web layer. The stale partial bundle is transactionally replaced.
    if [[ ! -f "$web_analysis/manifest.toml" \
          && -d "$current_analysis/layers/spider" \
          && ! -d "$web_analysis/layers/spider" ]]; then
      mkdir -p "$web_analysis/layers"
      cp -a "$current_analysis/layers/spider" "$web_analysis/layers/spider"
    fi
    analyze_variant "$name" current-geometry-web-aware "$web_scene" "$out/current-geometry-web-aware"

    render_variant_if_missing "$name" current-geometry "$current_scene" "$current_analysis" "$neutral_style_file" "$out"
    render_variant "$name" current-geometry-web-aware "$web_scene" "$web_analysis" "$neutral_style_file" "$out"
    side_by_side \
      "$out/current-geometry.lossless.mkv" \
      "$out/current-geometry-web-aware.lossless.mkv" \
      "$out/current-vs-web-aware.mp4"

    current_bronze="styles/bronze-relief.toml"
    render_variant_if_missing "$name" current-geometry__current-bronze "$current_scene" "$current_analysis" "$current_bronze" "$out"
    render_variant_if_missing "$name" current-geometry__banded-bronze "$current_scene" "$current_analysis" "$banded_bronze" "$out"
    render_variant "$name" web-aware__current-bronze "$web_scene" "$web_analysis" "$current_bronze" "$out"
    render_variant "$name" web-aware__banded-bronze "$web_scene" "$web_analysis" "$banded_bronze" "$out"
    side_by_side \
      "$out/current-geometry__current-bronze.lossless.mkv" \
      "$out/web-aware__current-bronze.lossless.mkv" \
      "$out/current-bronze__baseline-vs-web-aware.mp4"
    side_by_side \
      "$out/current-geometry__banded-bronze.lossless.mkv" \
      "$out/web-aware__banded-bronze.lossless.mkv" \
      "$out/banded-bronze__baseline-vs-web-aware.mp4"

    cat > "$out/README.txt" <<'EOF'
16:9 dungeon crossing-web recovery experiment

The current authored plaque geometry is the retained baseline. The rejected v0.8 geometry is
not rerun for this asset. The only challenger adds an optical crossing-web foreground layer.

Review:
  current-vs-web-aware.mp4
  current-bronze__baseline-vs-web-aware.mp4
  banded-bronze__baseline-vs-web-aware.mp4

LEFT  = current geometry and current foreground model
RIGHT = same geometry plus explicit crossing-web optical foreground/tracking exclusion

Promotion requires the web to remain source-visible over the title while plaque motion stays
stable through the crossing.
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

  if [[ "$name" == 16_9_dungeon_spider_iron_plaque || "$name" == 9_16_dungeon_spider_iron_plaque ]]; then
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

  if [[ "$name" == 16_9_dungeon_spider_iron_plaque || "$name" == 9_16_dungeon_spider_iron_plaque ]]; then
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
