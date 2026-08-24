#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "$0")/common.sh"
cd "$PF_ROOT"
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

if git -c safe.directory="*" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  git -c safe.directory="*" diff --check
fi
