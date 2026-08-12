#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

usage() {
  cat <<'USAGE'
Usage:
  ./scripts/review_assets.sh [asset-stem ...]

Builds human-oriented HTML review pages from existing analysis diagnostics and,
when available, the matching output/<asset>.verification.json report.
USAGE
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  usage
  exit 0
fi

cargo build --release --quiet

if (( $# )); then
  cases=("$@")
else
  cases=()
  shopt -s nullglob
  for input in assets/*.mp4; do
    cases+=("$(basename "$input" .mp4)")
  done
  shopt -u nullglob
fi

(( ${#cases[@]} > 0 )) || { printf 'error: no assets selected\n' >&2; exit 2; }

for name in "${cases[@]}"; do
  analysis="assets/analysis/$name"
  verification="output/$name.verification.json"
  render_manifest="output/$name.hevc.render-manifest.json"
  args=(--analysis "$analysis")
  [[ -f "$verification" ]] && args+=(--verification "$verification")
  [[ -f "$render_manifest" ]] && args+=(--render-manifest "$render_manifest")
  target/release/plaque-forge review "${args[@]}"
done
