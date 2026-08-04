#!/usr/bin/env bash
set -euo pipefail

python3 tools/validate_source.py

cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features

if [[ -n "${REFERENCE_VIDEO:-}" && -n "${REFERENCE_MOTION:-}" ]]; then
  reference_output="${REFERENCE_OUTPUT:-/tmp/plaque-forge-reference-m3}"
  reference_rect="${REFERENCE_RECT:-130,160,458,268}"
  python3 tools/reference_validate_m3.py \
    --video "$REFERENCE_VIDEO" \
    --truth "$REFERENCE_MOTION" \
    --output "$reference_output" \
    --rect "$reference_rect"
fi

if [[ -n "${REFERENCE_VIDEO:-}" && -n "${REFERENCE_FONT:-}" ]]; then
  scripts/validate_reference.sh
fi
