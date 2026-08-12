#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
root="${TMPDIR:-/tmp}/plaque-forge-python-check-$$"
cleanup() {
  [[ "$root" == "${TMPDIR:-/tmp}"/plaque-forge-python-check-* ]] && rm -rf -- "$root"
}
trap cleanup EXIT
mkdir -p "$root"
PYTHONPYCACHEPREFIX="$root" python3 -m py_compile tools/segmentation_worker.py
printf 'segmentation worker syntax: OK\n'
