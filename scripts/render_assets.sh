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
matching environment variables used by earlier versions.
USAGE
}

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
cargo build --release --quiet
mkdir -p output

stage="$(mktemp -d "$PF_ROOT/output/.plaque-forge-render.XXXXXX")"
cleanup() {
  if [[ "$stage" == "$PF_ROOT"/output/.plaque-forge-render.* ]]; then
    rm -rf -- "$stage"
  fi
}
trap cleanup EXIT

encoder_args=(
  --encoder-arg=-c:v --encoder-arg=libx265
  --encoder-arg=-preset --encoder-arg=medium
  --encoder-arg=-crf --encoder-arg=20
  --encoder-arg=-pix_fmt --encoder-arg=yuv420p
  --encoder-arg=-c:a --encoder-arg=copy
  --encoder-arg=-shortest
)

for name in "${PF_CASES[@]}"; do
  input="assets/$name.mp4"
  staged="$stage/$name.hevc.mkv"
  staged_manifest="$stage/$name.hevc.render-manifest.json"
  final="output/$name.hevc.mkv"
  final_manifest="output/$name.hevc.render-manifest.json"

  [[ -f "$input" ]] || { printf 'input video not found: %s\n' "$input" >&2; exit 1; }

  if [[ ! -f "assets/analysis/$name/manifest.toml" ]]; then
    partial="$(ls -1dt "assets/analysis/$name.partial-"* 2>/dev/null | head -n 1 || true)"
    printf '## SKIPPING %s: no complete analysis cache. Run ./scripts/analyze_assets.sh %s\n' "$name" "$name" >&2
    if [[ -n "$partial" && -f "$partial/diagnostics/review.html" ]]; then
      printf '   review: %s/diagnostics/review.html\n' "$partial" >&2
    fi
    continue
  fi

  target/release/plaque-forge render \
    --input "$input" --output "$staged" \
    "${PF_RENDER_OPTIONS[@]}" "${encoder_args[@]}" --progress always || {
      echo "## PROCESSING FAILED FOR '$input'. Continuing..."
      continue;
    }

  [[ -f "$staged_manifest" ]] || {
    printf 'render manifest not produced: %s\n' "$staged_manifest" >&2
    exit 1
  }
  mv -f -- "$staged_manifest" "$final_manifest"
  mv -f -- "$staged" "$final"
  printf '%s\n' "$final"
done
