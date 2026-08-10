#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
font="${FONT:-$(fc-match -f '%{file}\n' 'DejaVu Sans' | head -n 1)}"
text="${TITLE_TEXT:-Analises desta 3a. feira, 1 de Agosto}"
stage="$(mktemp -d /tmp/plaque-forge-render.XXXXXX)"
trap 'rm -rf "$stage"' EXIT

cd "$root"
cargo build --release
rm -rf assets/analysis output
mkdir -p assets/analysis output

if (( $# )); then
  cases=("$@")
else
  cases=()
  for input in assets/*.mp4; do
    cases+=("$(basename "$input" .mp4)")
  done
fi

for name in "${cases[@]}"; do
  input="assets/$name.mp4"
  lossless="$stage/$name.mkv"
  final="output/$name.hevc.mkv"

  target/release/plaque-forge render \
    --input "$input" --output "$lossless" \
    --text "$text" --font "$font" --reanalyze --progress always

  if [[ -c /dev/dri/renderD128 ]]; then
    ffmpeg -hide_banner -loglevel warning -vaapi_device /dev/dri/renderD128 \
      -i "$lossless" -map 0:v:0 -map '0:a:0?' -vf 'format=nv12,hwupload' \
      -c:v hevc_vaapi -preset veryslow -qp 26 -c:a copy -y "$final"
  else
    ffmpeg -hide_banner -loglevel warning -i "$lossless" \
      -map 0:v:0 -map '0:a:0?' -c:v libx265 -preset medium -crf 20 \
      -pix_fmt yuv420p -c:a copy -y "$final"
  fi

  cp "${lossless%.mkv}.verification.json" "output/$name.verification.json"
  printf '%s\n' "$final"
done
