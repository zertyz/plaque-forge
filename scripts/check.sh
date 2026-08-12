#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
bash -n scripts/*.sh tools/segmentation-worker
./scripts/check_segmentation.sh

set +e
./scripts/render_assets.sh --plaque-forge-invalid-option >/dev/null 2>&1
parser_status=$?
set -e
if (( parser_status != 2 )); then
  printf 'render option parser returned %d; expected 2\n' "$parser_status" >&2
  exit 1
fi

git diff --check
