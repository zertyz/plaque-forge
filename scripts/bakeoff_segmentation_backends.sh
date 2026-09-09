#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "$0")/common.sh"
root="$PF_ROOT"
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
  --min-iou VALUE     minimum acceptable IoU against canonical reference (e.g. 0.95)
  --max-mean-absolute VALUE maximum mean absolute drift against canonical reference (e.g. 0.05)
  --gate              abort with exit code 1 if any candidate backend violates acceptance criteria

Examples:
  --backends "sam2 sam2-cutie sam2-cutie-vitmatte"
  --backends "sam2 sam3.1" --device cuda --precision bf16
  --backends "sam2 sam2-cutie" --min-iou 0.95 --gate
USAGE
}

min_iou=""
max_mean_absolute=""
gate=0

while (( $# > 2 )); do
  case "$1" in
    --device) device="$2"; shift 2 ;;
    --profile) profile="$2"; shift 2 ;;
    --precision) precision="$2"; shift 2 ;;
    --backends) backends="$2"; shift 2 ;;
    --min-iou) min_iou="$2"; shift 2 ;;
    --max-mean-absolute) max_mean_absolute="$2"; shift 2 ;;
    --gate) gate=1; shift ;;
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
pf_build_release
run_root="/tmp/plaque-forge/segmentation-bakeoff/$asset/$layer"
golden_ref="$root/assets/analysis/$asset/layers/$layer"
rm -rf -- "$run_root"
mkdir -p "$run_root"

baseline=""
regression_failed=0
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
    PYTHONPATH=tools python3 tools/compare_segmentation_outputs.py \
      "$run_root/$baseline" "$output" \
      --json "$run_root/${baseline}-vs-${backend}.json" >/dev/null
  fi

  if [[ -d "$golden_ref" ]]; then
    compare_cmd=(
      env PYTHONPATH=tools python3 tools/compare_segmentation_outputs.py
      "$golden_ref" "$output"
      --json "$run_root/canonical-vs-${backend}.json"
    )
    if [[ -n "$min_iou" ]]; then
      compare_cmd+=(--min-iou "$min_iou")
    fi
    if [[ -n "$max_mean_absolute" ]]; then
      compare_cmd+=(--max-mean-absolute "$max_mean_absolute")
    fi
    if (( gate )); then
      compare_cmd+=(--report-failures)
    fi
    if ! "${compare_cmd[@]}"; then
      printf '[REGRESSION] backend %s failed non-regression criteria against canonical reference!\n' "$backend" >&2
      regression_failed=1
    fi
  fi
done

printf '\n[bakeoff] outputs: %s\n' "$run_root" >&2
summarize_cmd=(
  env PYTHONPATH=tools python3 tools/summarize_segmentation_bakeoff.py "$run_root"
  --json "$run_root/summary.json" --markdown "$run_root/summary.md"
)
if [[ -n "$min_iou" ]]; then
  summarize_cmd+=(--min-iou "$min_iou")
fi
if [[ -n "$max_mean_absolute" ]]; then
  summarize_cmd+=(--max-mean-absolute "$max_mean_absolute")
fi
"${summarize_cmd[@]}"

if (( gate && regression_failed )); then
  printf '\n[ERROR] Bake-off gate failed due to segmentation regressions on %s / %s\n' "$asset" "$layer" >&2
  exit 1
fi
