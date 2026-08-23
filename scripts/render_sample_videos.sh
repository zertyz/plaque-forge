#!/usr/bin/env bash
# Render the project's sample videos for publication.
#
# Plaque assets (whose stem contains "plaque" but not "plaqueless") use the golden
# "gold-shine" style, which reads well on metal/stone surfaces. The remaining
# plaque-less videos use the built-in glowing default style, which reads better on
# open/transparent backgrounds.
#
# Output lands in output/*.hevc.mkv, ready for the sample-videos release workflow.

set -euo pipefail

source "$(dirname "$0")/render_common.sh"

SAMPLE_TEXT="${SAMPLE_TEXT:-plaque-forge: High Quality & Advanced Text Placing in Videos}"
SAMPLE_FONT_FAMILY="${SAMPLE_FONT_FAMILY:-Noto Serif}"
SAMPLE_FIT="${SAMPLE_FIT:-artistic}"
SAMPLE_PLAQUE_STYLE_FILE="${SAMPLE_PLAQUE_STYLE_FILE:-styles/gold-shine.toml}"
SAMPLE_OTHER_STYLE_FILE="${SAMPLE_OTHER_STYLE_FILE:-}"

cd "$PF_ROOT"
cargo build --release --quiet
mkdir -p output

shopt -s nullglob
plaque_cases=()
other_cases=()
for input in assets/*.mp4; do
  name="$(basename "$input" .mp4)"
  if [[ "$name" == *plaque* && "$name" != *plaqueless* ]]; then
    plaque_cases+=("$name")
  else
    other_cases+=("$name")
  fi
done
shopt -u nullglob

(( ${#plaque_cases[@]} + ${#other_cases[@]} > 0 )) || pf_die "no input videos found in $PF_ROOT/assets"

render_batch() {
  local style_file="$1"; shift
  local -a cases=("$@")
  (( ${#cases[@]} )) || return 0

  local -a cmd=(./scripts/render_assets.sh --text "$SAMPLE_TEXT" --font-family "$SAMPLE_FONT_FAMILY" --fit "$SAMPLE_FIT")
  if [[ -n "$style_file" ]]; then
    cmd+=(--style-file "$style_file")
  fi
  cmd+=("${cases[@]}")
  "${cmd[@]}"
}

printf '## Rendering %d plaque sample(s) with style: %s\n' "${#plaque_cases[@]}" "${SAMPLE_PLAQUE_STYLE_FILE:-<default glow>}"
render_batch "$SAMPLE_PLAQUE_STYLE_FILE" "${plaque_cases[@]}"

printf '## Rendering %d plaque-less sample(s) with style: %s\n' "${#other_cases[@]}" "${SAMPLE_OTHER_STYLE_FILE:-<default glow>}"
render_batch "$SAMPLE_OTHER_STYLE_FILE" "${other_cases[@]}"

printf '## Sample videos written to output/*.hevc.mkv\n'
