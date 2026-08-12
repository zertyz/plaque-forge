#!/usr/bin/env bash
set -euo pipefail

# Replaces only the selected output/*.verification.json reports.
source "$(dirname "$0")/render_common.sh"
pf_configure_render "$@"

stage="$(mktemp -d /tmp/plaque-forge-validate.XXXXXX)"
cleanup() {
  # This path is created by mktemp above and contains only this run's lossless renders.
  if [[ "$stage" == /tmp/plaque-forge-validate.* ]]; then
    rm -rf -- "$stage"
  fi
}
trap cleanup EXIT

cd "$PF_ROOT"
cargo build --release --quiet
mkdir -p output

for name in "${PF_CASES[@]}"; do
  input="assets/$name.mp4"
  analysis="assets/analysis/$name"
  lossless="$stage/$name.mkv"
  report="output/$name.verification.json"

  if [[ ! -f "$input" ]]; then
    printf 'input video not found: %s\n' "$input" >&2
    exit 1
  fi
  target/release/plaque-forge render \
    --input "$input" --output "$lossless" \
    "${PF_RENDER_OPTIONS[@]}" --progress always
  target/release/plaque-forge verify \
    --analysis "$analysis" --rendered "$lossless" --original "$input" \
    --report "$report" --progress always
  printf '%s\n' "$report"
done
