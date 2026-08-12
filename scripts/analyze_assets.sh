#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
force=false
cases=()

usage() {
  cat <<'USAGE'
Usage: ./scripts/analyze_assets.sh [--force] [asset-stem ...]

Build missing reusable scene-analysis caches. Existing caches are skipped unless
--force is given. Refinements under assets/refinements/<asset>/ are discovered by
Plaque Forge automatically.
USAGE
}

while (( $# )); do
  case "$1" in
    --force) force=true; shift ;;
    --help|-h) usage; exit 0 ;;
    -*) printf 'unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
    *) cases+=("$1"); shift ;;
  esac
done

cd "$root"
cargo build --release --quiet

if (( ${#cases[@]} == 0 )); then
  shopt -s nullglob
  for input in assets/*.mp4; do cases+=("$(basename "$input" .mp4)"); done
  shopt -u nullglob
fi
(( ${#cases[@]} > 0 )) || { printf 'no assets/*.mp4 files found\n' >&2; exit 1; }

for name in "${cases[@]}"; do
  input="assets/$name.mp4"
  cache="assets/analysis/$name"
  [[ -f "$input" ]] || { printf 'input video not found: %s\n' "$input" >&2; exit 1; }
  if [[ -d "$cache" && "$force" == false ]]; then
    printf 'skip existing cache: %s\n' "$cache"
    continue
  fi
  args=(analyze --input "$input" --progress always)
  [[ "$force" == true ]] && args+=(--force)
  target/release/plaque-forge "${args[@]}" || echo "## PROCESSING FAILED FOR THE ABOVE VIDEO. Continuing..."
done
