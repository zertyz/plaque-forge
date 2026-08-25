#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "$0")/common.sh"
cd "$PF_ROOT"
root="${TMPDIR:-/tmp}/plaque-forge-python-check-$$"
cleanup() {
  [[ "$root" == "${TMPDIR:-/tmp}"/plaque-forge-python-check-* ]] && rm -rf -- "$root"
}
trap cleanup EXIT
mkdir -p "$root"

PYTHONPYCACHEPREFIX="$root" python3 -m py_compile \
  tools/segmentation_runtime.py \
  tools/segmentation_worker.py \
  tools/segmentation_service.py \
  tools/sam31_worker.py \
  tools/compare_segmentation_outputs.py \
  tools/summarize_segmentation_bakeoff.py \
  tools/check_segmentation_capabilities.py \
  tools/test_segmentation_runtime.py \
  tools/test_compare_segmentation_outputs.py \
  tools/test_sam31_worker.py \
  tools/test_segmentation_service.py \
  tools/test_segmentation_worker_cache.py \
  tools/test_segmentation_worker_quality.py \
  tools/test_segmentation_worker_parallelism.py
PYTHONPATH=tools PYTHONPYCACHEPREFIX="$root" python3 -m unittest \
  tools/test_segmentation_runtime.py \
  tools/test_compare_segmentation_outputs.py \
  tools/test_sam31_worker.py \
  tools/test_segmentation_service.py \
  tools/test_segmentation_worker_cache.py \
  tools/test_segmentation_worker_quality.py \
  tools/test_segmentation_worker_parallelism.py
bash -n \
  scripts/setup_segmentation.sh \
  scripts/setup_sam31.sh \
  scripts/analyze_assets.sh \
  scripts/compare_segmentation_devices.sh \
  scripts/bakeoff_segmentation_backends.sh \
  scripts/bakeoff_segmentation_matrix.sh \
  scripts/check_segmentation_capabilities.sh \
  tools/segmentation-worker
python3 tools/check_segmentation_capabilities.py >/dev/null
printf 'segmentation worker/setup contracts: OK\n'
