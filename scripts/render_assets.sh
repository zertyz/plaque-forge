#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "$0")/render_common.sh"

usage() {
  cat <<'USAGE'
Usage:
  ./scripts/render_assets.sh --text 'Title' [options] [asset-stem ...]
  TITLE_TEXT='Title' ./scripts/render_assets.sh [asset-stem ...]

Common options:
  --font PATH                 Font file.
  --font-family PATTERN       Resolve a font with fontconfig, e.g. 'Noto Serif'.
  --output-dir DIR            Delivery directory for the rendered videos (default: output).
  --max-lines N               Maximum automatic line count.
  --padding RATIO             Inset from the writable region.
  --stroke-width RATIO        Outline width relative to font size.
  --glow-radius PIXELS        Glow blur radius.
  --shadow-offset-x RATIO     Shadow X offset relative to font size.
  --shadow-offset-y RATIO     Shadow Y offset relative to font size.
  --shadow-blur-radius PX     Shadow blur radius.
  --shadow-color RGBA         Shadow color, e.g. #000000A0.
  --style NAME                Built-in preset from styles/NAME.toml.
  --style-file PATH           Custom TOML paint/effect stack; replaces paint flags.
  --fit MODE                  maximize, balanced, artistic, or fixed.

All typography options accepted by this script are also available through the
matching environment variables documented in scripts/render_common.sh.
USAGE
}

# Extract --output-dir before the shared parser so the delivery directory can
# vary (for example the sample-video producer) without touching render options.
output_dir="output"
forward=()
while (( $# )); do
  case "$1" in
    --output-dir)
      (( $# >= 2 )) || { printf 'error: --output-dir requires a value\n' >&2; exit 2; }
      output_dir="$2"; shift 2 ;;
    *) forward+=("$1"); shift ;;
  esac
done
if (( ${#forward[@]} )); then
  set -- "${forward[@]}"
else
  set --
fi

set +e
pf_configure_render "$@"
status=$?
set -e
if (( status != 0 )); then
  if (( status == 64 )); then usage; exit 0; fi
  usage >&2
  exit "$status"
fi

cd "$PF_ROOT"
pf_build_release
mkdir -p "$output_dir"

encoder_args=(
  --encoder-arg=-c:v --encoder-arg=libx265
  --encoder-arg=-preset --encoder-arg=medium
  --encoder-arg=-crf --encoder-arg=20
  --encoder-arg=-pix_fmt --encoder-arg=yuv420p
  --encoder-arg=-c:a --encoder-arg=copy
  --encoder-arg=-shortest
)

failures=0
rendered=0
for name in "${PF_CASES[@]}"; do
  input="assets/$name.mp4"
  final="$output_dir/$name.hevc.mkv"

  [[ -f "$input" ]] || { printf 'input video not found: %s\n' "$input" >&2; exit 1; }

  if [[ ! -f "assets/analysis/$name/manifest.toml" ]]; then
    retained="$(ls -1dt "/tmp/plaque-forge/failures/$name/"* 2>/dev/null | head -n 1 || true)"
    printf '## SKIPPING %s: no complete analysis cache. Run ./scripts/analyze_assets.sh %s\n' "$name" "$name" >&2
    if [[ -n "$retained" && -f "$retained/diagnostics/review.html" ]]; then
      printf '   review: %s/diagnostics/review.html\n' "$retained" >&2
    fi
    failures=$((failures + 1))
    continue
  fi

  target/release/plaque-forge render \
    --input "$input" --output "$final" \
    "${PF_RENDER_OPTIONS[@]}" "${encoder_args[@]}" --progress always || {
      printf "## PROCESSING FAILED FOR '%s'. Continuing...\n" "$input" >&2
      failures=$((failures + 1))
      continue
    }

  # A replaced render invalidates every acceptance report bound to the previous bytes.
  # Never leave a green-looking report beside an artifact with a different SHA-256.
  rm -f -- "output/$name.verification.json" "output/$name.homologation.json"

  rendered=$((rendered + 1))
  printf '%s\n' "$final"
done

if (( failures > 0 || rendered == 0 )); then
  printf 'render batch incomplete: %d rendered, %d failed/skipped\n' "$rendered" "$failures" >&2
  exit 1
fi
