#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

usage() {
  cat <<'USAGE'
Usage:
  ./scripts/review_assets.sh [asset-stem ...]

Builds human-oriented review.html + review.txt from complete analysis diagnostics or,
when analysis failed, the newest retained partial diagnostics. Includes verification/render
provenance when available.
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
  if [[ ! -d "$analysis" ]]; then
    analysis="$(ls -1dt "assets/analysis/$name.partial-"* 2>/dev/null | head -n 1 || true)"
  fi
  [[ -n "$analysis" && -d "$analysis" ]] || {
    printf 'warning: no complete or partial analysis found for %s\n' "$name" >&2
    continue
  }

  verification="output/$name.verification.json"
  render_manifest="output/$name.hevc.render-manifest.json"
  refinement="assets/refinements/$name/refinement.toml"
  args=(--analysis "$analysis")
  [[ -f "$refinement" ]] && args+=(--refinement "$refinement")
  [[ -f "$verification" ]] && args+=(--verification "$verification")
  [[ -f "$render_manifest" ]] && args+=(--render-manifest "$render_manifest")
  target/release/plaque-forge review "${args[@]}"
done
