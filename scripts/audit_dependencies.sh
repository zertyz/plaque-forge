#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

if ! command -v cargo-audit >/dev/null 2>&1; then
  printf '%s\n' \
    'cargo-audit is required for this check.' \
    'Install the pinned CI version with: cargo install cargo-audit --version 0.22.2 --locked' >&2
  exit 127
fi

# RUSTSEC-2026-0192 is an unmaintained notice, not a vulnerability. It has no
# patched ttf-parser release and reaches us only through cosmic-text -> fontdb.
# Keep the exception explicit so --deny warnings still rejects every new notice.
cargo audit \
  --deny warnings \
  --ignore RUSTSEC-2026-0192
