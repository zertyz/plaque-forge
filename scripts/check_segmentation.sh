#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
root="${TMPDIR:-/tmp}/plaque-forge-python-check-$$"
cleanup() {
  [[ "$root" == "${TMPDIR:-/tmp}"/plaque-forge-python-check-* ]] && rm -rf -- "$root"
}
trap cleanup EXIT
mkdir -p "$root"

PYTHONPYCACHEPREFIX="$root" python3 -m py_compile \
  tools/segmentation_runtime.py \
  tools/segmentation_worker.py \
  tools/test_segmentation_runtime.py
PYTHONPATH=tools PYTHONPYCACHEPREFIX="$root" python3 tools/test_segmentation_runtime.py
bash -n scripts/setup_segmentation.sh scripts/analyze_assets.sh tools/segmentation-worker
printf 'segmentation worker/setup contracts: OK\n'
