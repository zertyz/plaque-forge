#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
base="${1:-}"
head="${2:-HEAD}"

if [[ -z "$base" || "$base" =~ ^0+$ ]] || ! git cat-file -e "$base^{commit}" 2>/dev/null; then
  changed="$(git ls-files)"
else
  if ! changed="$(git diff --name-only "$base...$head" 2>/dev/null)"; then
    changed="$(git diff --name-only "$base" "$head")"
  fi
fi

needs_analysis=false
needs_ml_analysis=false
needs_ml_runtime=false

while IFS= read -r path; do
  [[ -n "$path" ]] || continue
  case "$path" in
    Cargo.toml|Cargo.lock|\
    src/analyze/*|src/analyze/**/*|src/analysis.rs|src/layers.rs|src/model.rs|\
    src/scene.rs|src/segmentation.rs|src/video.rs|src/writable_region.rs|\
    scripts/analyze_assets.sh|scripts/analysis_change_scope.sh|\
    assets/*.mp4|assets/scenes/*|assets/scenes/**/*)
      needs_analysis=true
      ;;
  esac

  case "$path" in
    assets/*.mp4|assets/scenes/*|assets/scenes/**/*|\
    src/segmentation.rs|\
    tools/segmentation-worker|tools/segmentation_worker.py|tools/segmentation_runtime.py|\
    tools/segmentation-requirements.txt|scripts/setup_segmentation.sh)
      needs_ml_analysis=true
      ;;
  esac

  case "$path" in
    tools/segmentation-worker|tools/segmentation_worker.py|tools/segmentation_runtime.py|\
    tools/segmentation-requirements.txt|tools/test_segmentation_runtime.py|\
    scripts/setup_segmentation.sh|scripts/check_segmentation.sh)
      needs_ml_runtime=true
      ;;
  esac
done <<< "$changed"

printf 'needs_analysis=%s\n' "$needs_analysis"
printf 'needs_ml_analysis=%s\n' "$needs_ml_analysis"
printf 'needs_ml_runtime=%s\n' "$needs_ml_runtime"
