#!/usr/bin/env bash
set -euo pipefail

: "${REFERENCE_VIDEO:?set REFERENCE_VIDEO to a supported text-free plaque video}"
: "${REFERENCE_FONT:?set REFERENCE_FONT to the exact font file to validate}"

reference_rect="${REFERENCE_RECT:-130,160,458,268}"
reference_text="${REFERENCE_TEXT:-CASE CLOSED}"
track_args=()
if [[ -n "${REFERENCE_TRACK:-}" ]]; then
  track_args=(--track-csv "$REFERENCE_TRACK")
fi
if [[ -n "${REFERENCE_OUTPUT:-}" ]]; then
  output_root="$REFERENCE_OUTPUT"
  mkdir -p "$output_root"
else
  output_root="$(mktemp -d /tmp/plaque-forge-reference.XXXXXX)"
fi

cargo build --release
./target/release/plaque-forge replace \
  --input "$REFERENCE_VIDEO" \
  --output "$output_root/render.mkv" \
  --analysis "$output_root/analysis.titlepack" \
  --text "$reference_text" \
  --font "$REFERENCE_FONT" \
  --plaque-hint "$reference_rect" \
  "${track_args[@]}" \
  --diagnostics "$output_root/diagnostics" \
  --progress always

source_frames="$(ffprobe -v error -count_packets -select_streams v:0 \
  -show_entries stream=nb_read_packets -of default=nw=1:nk=1 "$REFERENCE_VIDEO")"
render_frames="$(ffprobe -v error -count_packets -select_streams v:0 \
  -show_entries stream=nb_read_packets -of default=nw=1:nk=1 "$output_root/render.mkv")"
if [[ "$source_frames" != "$render_frames" ]]; then
  printf 'frame-count mismatch: source=%s render=%s\n' "$source_frames" "$render_frames" >&2
  exit 1
fi

printf 'reference validation artifacts: %s\n' "$output_root"
printf 'review: %s\n' "$output_root/diagnostics/render-contact-sheet.png"
