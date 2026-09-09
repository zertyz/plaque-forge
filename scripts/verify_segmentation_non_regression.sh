#!/usr/bin/env bash
# Automated pre-promotion gate asserting that candidate segmentation
# models and parameters do not regress against accepted golden references.
set -euo pipefail

source "$(dirname "$0")/common.sh"
root="$PF_ROOT"
cd "$root"

device="auto"
profile="canonical"
precision="auto"
min_iou="0.95"
max_mean_absolute="0.05"
candidate_dir=""
specific_assets=()

usage() {
  cat <<'USAGE'
Usage: ./scripts/verify_segmentation_non_regression.sh [options] [asset ...]

Assert that segmentation outputs match accepted golden references within
strict tolerance thresholds, preventing cross-video quality regressions.

Options:
  --candidate-dir DIR         path to pre-generated candidate masks tree
  --device D                  execution device (default: auto)
  --profile P                 preview|balanced|canonical (default: canonical)
  --precision P               auto|fp32|bf16 (default: auto)
  --min-iou FLOAT             minimum acceptable IoU (default: 0.95)
  --max-mean-absolute FLOAT   maximum mean absolute drift (default: 0.05)
  -h, --help                  show this help message
USAGE
}

while (( $# )); do
  case "$1" in
    --candidate-dir) candidate_dir="$2"; shift 2 ;;
    --device) device="$2"; shift 2 ;;
    --profile) profile="$2"; shift 2 ;;
    --precision) precision="$2"; shift 2 ;;
    --min-iou) min_iou="$2"; shift 2 ;;
    --max-mean-absolute) max_mean_absolute="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) specific_assets+=("$1"); shift ;;
  esac
done

if [[ -z "$candidate_dir" ]]; then
  py_worker="${PLAQUE_FORGE_PYTHON_BIN:-/tmp/plaque-forge-python/venv/bin/python}"
  if [[ ! -x "$py_worker" ]] && ! python3 -c "import torch" 2>/dev/null; then
    cat >&2 <<'ERR'
[ERROR] ML segmentation runtime not found at /tmp/plaque-forge-python/venv/bin/python.
To run live segmentation generation, set up the runtime with:
  ./scripts/setup_segmentation.sh
Alternatively, verify an existing pre-generated mask tree with:
  ./scripts/verify_segmentation_non_regression.sh --candidate-dir <DIR>
ERR
    exit 2
  fi
  pf_build_release
fi

export SPECIFIC_ASSETS="${specific_assets[*]:-}"
mapfile -t cases < <(python3 - <<'PY'
import os
import tomllib
from pathlib import Path

specific = set(os.environ.get("SPECIFIC_ASSETS", "").split())
seen = set()

for scene_path in sorted(Path('assets/scenes').glob('*/scene.toml')):
    asset = scene_path.parent.name
    if specific and asset not in specific:
        continue
    doc = tomllib.loads(scene_path.read_text())
    for layer in doc.get('layers', []):
        layer_id = layer.get('id')
        if layer.get('prompts') and (asset, layer_id) not in seen:
            golden_ref = Path(f'assets/analysis/{asset}/layers/{layer_id}')
            if golden_ref.is_dir():
                seen.add((asset, layer_id))
                print(asset, layer_id)
PY
)

(( ${#cases[@]} > 0 )) || {
  printf 'no prompted layers with golden references found\n' >&2
  exit 1
}

run_dir="${TMPDIR:-/tmp}/plaque-forge/segmentation-non-regression"
rm -rf -- "$run_dir"
mkdir -p "$run_dir"

printf '=== Segmentation Non-Regression Gate ===\n'
printf 'Targets: %d prompted layers | min_iou=%s | max_mae=%s\n\n' "${#cases[@]}" "$min_iou" "$max_mean_absolute"

failures=()
passes=()

for case in "${cases[@]}"; do
  read -r asset layer <<< "$case"
  input="$root/assets/$asset.mp4"
  scene="$root/assets/scenes/$asset/scene.toml"
  golden="$root/assets/analysis/$asset/layers/$layer"

  if [[ -n "$candidate_dir" ]]; then
    if [[ -d "$candidate_dir/$asset/layers/$layer" ]]; then
      candidate="$candidate_dir/$asset/layers/$layer"
    elif [[ -d "$candidate_dir/$asset/$layer" ]]; then
      candidate="$candidate_dir/$asset/$layer"
    elif [[ -d "$candidate_dir/$layer" ]]; then
      candidate="$candidate_dir/$layer"
    else
      printf '[ERROR] candidate masks for %s / %s not found under %s\n' "$asset" "$layer" "$candidate_dir" >&2
      failures+=("$asset/$layer: missing candidate masks under $candidate_dir")
      continue
    fi
  else
    candidate="$run_dir/$asset/$layer"
    if ! target/release/plaque-forge segment \
      --input "$input" --scene "$scene" --layer "$layer" \
      --worker tools/segmentation-worker \
      --backend auto --model auto --device "$device" \
      --profile "$profile" --precision "$precision" \
      --output "$candidate" --force >/dev/null 2>&1; then
      printf '[ERROR] segmentation generation failed for %s / %s\n' "$asset" "$layer" >&2
      failures+=("$asset/$layer: segmentation generation failed")
      continue
    fi
  fi

  printf '[CHECKING] %s / %s ... ' "$asset" "$layer"

  report_json="$run_dir/${asset}_${layer}.json"
  if PYTHONPATH=tools python3 tools/compare_segmentation_outputs.py \
      "$golden" "$candidate" \
      --min-iou "$min_iou" \
      --max-mean-absolute "$max_mean_absolute" \
      --report-failures \
      --json "$report_json" >/dev/null 2>&1; then
    iou=$(python3 -c "import json; print(f'{json.load(open(\"$report_json\"))[\"binary_at_0_5\"][\"iou\"]:.4f}')")
    mae=$(python3 -c "import json; print(f'{json.load(open(\"$report_json\"))[\"alpha\"][\"mean_absolute\"]:.4f}')")
    printf 'PASS (mIoU: %s, MAE: %s)\n' "$iou" "$mae"
    passes+=("$asset/$layer: mIoU=$iou, MAE=$mae")
  else
    iou=$(python3 -c "import json; print(f'{json.load(open(\"$report_json\"))[\"binary_at_0_5\"][\"iou\"]:.4f}')" 2>/dev/null || echo "N/A")
    mae=$(python3 -c "import json; print(f'{json.load(open(\"$report_json\"))[\"alpha\"][\"mean_absolute\"]:.4f}')" 2>/dev/null || echo "N/A")
    printf 'FAIL (mIoU: %s, MAE: %s)\n' "$iou" "$mae"
    failures+=("$asset/$layer: mIoU=$iou, MAE=$mae")
  fi
done

printf '\n=== Summary ===\n'
printf 'Passed: %d / %d\n' "${#passes[@]}" "${#cases[@]}"
if (( ${#failures[@]} > 0 )); then
  printf 'Failed regressions (%d):\n' "${#failures[@]}" >&2
  for fail in "${failures[@]}"; do
    printf '  - %s\n' "$fail" >&2
  done
  printf '\n[CRITICAL ERROR] Segmentation quality degraded against accepted golden references!\n' >&2
  exit 1
fi

printf '\n[SUCCESS] All segmentation layers verified non-regressive against accepted goldens.\n'
