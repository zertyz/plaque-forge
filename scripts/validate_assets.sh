#!/usr/bin/env bash
set -euo pipefail

# Retain the exact lossless artifact that each verification report certifies.
source "$(dirname "$0")/render_common.sh"
pf_configure_render "$@"

cd "$PF_ROOT"
pf_build_release
mkdir -p output/validation

for name in "${PF_CASES[@]}"; do
  input="assets/$name.mp4"
  analysis="assets/analysis/$name"
  lossless="output/validation/$name.lossless.mkv"
  report="output/validation/$name.lossless.verification.json"

  if [[ ! -f "$input" ]]; then
    printf 'input video not found: %s\n' "$input" >&2
    exit 1
  fi
  target/release/plaque-forge render \
    --input "$input" --output "$lossless" \
    "${PF_RENDER_OPTIONS[@]}" --progress always
  # The render was replaced successfully. If verification aborts before publishing
  # a new report, no report is safer than a stale report for older bytes.
  rm -f -- "$report"
  target/release/plaque-forge verify \
    --analysis "$analysis" --rendered "$lossless" --original "$input" \
    --report "$report" --progress always
  printf '%s\n' "$report"
done
