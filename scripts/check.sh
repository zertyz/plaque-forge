#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
python3 -m py_compile tools/segmentation_worker.py
bash -n scripts/render_common.sh scripts/render_assets.sh scripts/validate_assets.sh \
  scripts/setup_segmentation.sh tools/segmentation-worker
