#!/usr/bin/env bash
# Shared shell infrastructure for the high-level repository scripts:
# repository-root resolution, release builds, and bundled-asset enumeration.

PF_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

pf_die() {
  printf 'error: %s\n' "$*" >&2
  exit 2
}

pf_build_release() {
  (cd "$PF_ROOT" && cargo build --release --quiet)
}

# Fill PF_CASES (or the array named by the optional first argument) with the
# stems of every bundled source video.
pf_asset_cases() {
  local -n cases="${1:-PF_CASES}"
  local input
  cases=()
  shopt -s nullglob
  for input in "$PF_ROOT"/assets/*.mp4; do
    cases+=("$(basename "$input" .mp4)")
  done
  shopt -u nullglob
  (( ${#cases[@]} > 0 )) || pf_die "no input videos found in $PF_ROOT/assets"
}
