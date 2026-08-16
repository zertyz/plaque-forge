#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
device_a="cpu"
device_b="xpu"
profile="canonical"
precision="fp32"
backend="auto"
model="auto"

usage() {
  cat >&2 <<'USAGE'
usage: ./scripts/compare_segmentation_devices.sh [options] ASSET_STEM LAYER_ID

Run the exact same Rust-sealed segmentation strategy on two execution devices and
compare their stored 16-bit masks. This measures numerical/backend drift; it does not
claim that either device is intrinsically higher quality.

Options:
  --device-a NAME   first device (default: cpu)
  --device-b NAME   second device (default: xpu)
  --profile NAME    preview|balanced|canonical (default: canonical)
  --precision NAME  fp32|bf16 (default: fp32)
  --backend NAME    auto or explicit backend (default: auto)
  --model NAME      auto or explicit model (default: auto)
USAGE
}

while (( $# > 2 )); do
  case "$1" in
    --device-a) device_a="$2"; shift 2 ;;
    --device-b) device_b="$2"; shift 2 ;;
    --profile) profile="$2"; shift 2 ;;
    --precision) precision="$2"; shift 2 ;;
    --backend) backend="$2"; shift 2 ;;
    --model) model="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) printf 'unknown option: %s\n' "$1" >&2; usage; exit 2 ;;
  esac
done
(( $# == 2 )) || { usage; exit 2; }
asset="$1"
layer="$2"
input="$root/assets/$asset.mp4"
scene="$root/assets/scenes/$asset/scene.toml"
[[ -f "$input" ]] || { printf 'missing input: %s\n' "$input" >&2; exit 1; }
[[ -f "$scene" ]] || { printf 'missing scene: %s\n' "$scene" >&2; exit 1; }
[[ -x "$root/tools/segmentation-worker" ]] || { printf 'segmentation runtime missing\n' >&2; exit 1; }

cd "$root"
cargo build --release --quiet
run_root="/tmp/plaque-forge/segmentation-drift/$asset/$layer"
rm -rf -- "$run_root"
mkdir -p "$run_root"

run_one() {
  local device="$1"
  local output="$2"
  target/release/plaque-forge segment \
    --input "$input" --scene "$scene" --layer "$layer" \
    --worker tools/segmentation-worker \
    --backend "$backend" --model "$model" --device "$device" \
    --profile "$profile" --precision "$precision" \
    --output "$output" --force
}

printf '[drift] first:  %s/%s\n' "$device_a" "$precision" >&2
run_one "$device_a" "$run_root/$device_a"
printf '[drift] second: %s/%s\n' "$device_b" "$precision" >&2
run_one "$device_b" "$run_root/$device_b"

python3 tools/compare_segmentation_outputs.py \
  "$run_root/$device_a" "$run_root/$device_b" \
  --json "$run_root/report.json"
printf '[drift] report: %s\n' "$run_root/report.json" >&2
