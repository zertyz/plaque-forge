#!/usr/bin/env bash
set -euo pipefail

# Replaces only the selected output/*.hevc.mkv files.
source "$(dirname "$0")/render_common.sh"
pf_configure "$@"

cd "$PF_ROOT"
cargo build --release --quiet
mkdir -p output

stage="$(mktemp -d "$PF_ROOT/output/.plaque-forge-render.XXXXXX")"
cleanup() {
  # This path is created by mktemp above and contains only this run's intermediates.
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
  final="output/$name.hevc.mkv"

  if [[ ! -f "$input" ]]; then
    printf 'input video not found: %s\n' "$input" >&2
    exit 1
  fi
  target/release/plaque-forge render \
    --input "$input" --output "$staged" \
    "${PF_RENDER_OPTIONS[@]}" "${encoder_args[@]}" --progress always
  mv -f -- "$staged" "$final"
  printf '%s\n' "$final"
done
