#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
device="auto"
profile="canonical"
precision="fp32"
backends="sam2 sam2-cutie"

usage() {
  cat >&2 <<'USAGE'
usage: ./scripts/bakeoff_segmentation_backends.sh [options] ASSET_STEM LAYER_ID

Generate comparable segmentation candidates for one authored scene layer. The first
backend is the numerical comparison baseline only, not visual ground truth. Review the
render/homologation evidence before promoting any strategy.

Options:
  --device NAME       execution device (default: auto)
  --profile NAME      preview|balanced|canonical (default: canonical)
  --precision NAME    fp32|bf16 (default: fp32)
  --backends "LIST"   space-separated explicit backends (default: "sam2 sam2-cutie")

Examples:
  --backends "sam2 sam2-cutie sam2-cutie-vitmatte"
  --backends "sam2 sam3.1" --device cuda --precision bf16
USAGE
}

while (( $# > 2 )); do
  case "$1" in
    --device) device="$2"; shift 2 ;;
    --profile) profile="$2"; shift 2 ;;
    --precision) precision="$2"; shift 2 ;;
    --backends) backends="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) printf 'unknown option: %s\n' "$1" >&2; usage; exit 2 ;;
  esac
done
(( $# == 2 )) || { usage; exit 2; }
asset="$1"
layer="$2"
input="$root/assets/$asset.mp4"
scene="$root/assets/scenes/$asset/scene.toml"
[[ -f "$input" && -f "$scene" ]] || { printf 'asset or scene missing for %s\n' "$asset" >&2; exit 1; }

cd "$root"
cargo build --release --quiet
run_root="/tmp/plaque-forge/segmentation-bakeoff/$asset/$layer"
rm -rf -- "$run_root"
mkdir -p "$run_root"

baseline=""
for backend in $backends; do
  output="$run_root/$backend"
  printf '\n[bakeoff] backend=%s device=%s profile=%s precision=%s\n' \
    "$backend" "$device" "$profile" "$precision" >&2
  target/release/plaque-forge segment \
    --input "$input" --scene "$scene" --layer "$layer" \
    --worker tools/segmentation-worker \
    --backend "$backend" --model auto --device "$device" \
    --profile "$profile" --precision "$precision" \
    --output "$output" --force
  if [[ -z "$baseline" ]]; then
    baseline="$backend"
  else
    python3 tools/compare_segmentation_outputs.py \
      "$run_root/$baseline" "$output" \
      --json "$run_root/${baseline}-vs-${backend}.json" >/dev/null
  fi
done

printf '\n[bakeoff] outputs: %s\n' "$run_root" >&2
python3 tools/summarize_segmentation_bakeoff.py "$run_root" \
  --json "$run_root/summary.json" --markdown "$run_root/summary.md"
