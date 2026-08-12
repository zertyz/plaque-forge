#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
python3 -m py_compile tools/segmentation_worker.py
printf 'segmentation worker syntax: OK\n'
