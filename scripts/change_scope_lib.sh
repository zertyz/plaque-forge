#!/usr/bin/env bash
# Shared change-scope classification for Plaque Forge CI gating.
#
# This file is sourced by the change-scope entry points; it performs no work on
# its own. It centralizes the conservative path-classification rules so that the
# analysis and render detectors cannot drift apart.
#
# The detector is an optimization only. It must prefer a false positive over
# missing a relevant gate, so every class intentionally over-matches.

# Emit the newline-separated list of paths changed between BASE and HEAD.
# When BASE is empty, all-zeros, or not a resolvable commit, fall back to every
# tracked file so an unresolvable history still triggers the conservative path.
pf_changed_paths() {
  local base="${1:-}" head="${2:-HEAD}"
  if [[ -z "$base" || "$base" =~ ^0+$ ]] || ! git cat-file -e "$base^{commit}" 2>/dev/null; then
    git ls-files
  else
    local changed
    if ! changed="$(git diff --name-only "$base...$head" 2>/dev/null)"; then
      changed="$(git diff --name-only "$base" "$head")"
    fi
    printf '%s\n' "$changed"
  fi
}

# Classify the changed paths into the conservative CI scope flags.
# Results are exposed through the global variables NEEDS_ANALYSIS,
# NEEDS_ML_ANALYSIS, NEEDS_ML_RUNTIME, and NEEDS_RENDER.
pf_classify_changes() {
  local base="$1" head="$2"
  NEEDS_ANALYSIS=false
  NEEDS_ML_ANALYSIS=false
  NEEDS_ML_RUNTIME=false
  NEEDS_RENDER=false

  while IFS= read -r path; do
    [[ -n "$path" ]] || continue

    # Analysis inputs. A change here may also change rendered output, so it sets
    # NEEDS_RENDER as well as NEEDS_ANALYSIS.
    case "$path" in
      Cargo.toml|Cargo.lock|\
      src/analyze/*|src/analyze/**/*|src/analysis.rs|src/layers.rs|src/model.rs|\
      src/scene.rs|src/segmentation.rs|src/segmentation_strategy.rs|\
      src/video.rs|src/writable_region.rs|\
      assets/segmentation/*|scripts/analyze_assets.sh|scripts/analysis_change_scope.sh|\
      assets/*.mp4|assets/scenes/*|assets/scenes/**/*)
        NEEDS_ANALYSIS=true; NEEDS_RENDER=true ;;
    esac

    case "$path" in
      assets/*.mp4|assets/scenes/*|assets/scenes/**/*|assets/segmentation/*|\
      src/segmentation.rs|src/segmentation_strategy.rs|\
      tools/segmentation-worker|tools/segmentation_worker.py|tools/segmentation_service.py|tools/segmentation_runtime.py|tools/sam31_worker.py|\
      tools/segmentation-requirements.txt|scripts/setup_segmentation.sh|scripts/setup_sam31.sh|\
      scripts/compare_segmentation_devices.sh|scripts/bakeoff_segmentation_backends.sh)
        NEEDS_ML_ANALYSIS=true ;;
    esac

    case "$path" in
      tools/segmentation-worker|tools/segmentation_worker.py|tools/segmentation_service.py|tools/segmentation_runtime.py|tools/sam31_worker.py|\
      tools/compare_segmentation_outputs.py|tools/test_segmentation_runtime.py|tools/test_compare_segmentation_outputs.py|tools/test_sam31_worker.py|\
      tools/segmentation-requirements.txt|scripts/setup_segmentation.sh|scripts/setup_sam31.sh|\
      scripts/check_segmentation.sh|scripts/compare_segmentation_devices.sh|scripts/bakeoff_segmentation_backends.sh)
        NEEDS_ML_RUNTIME=true ;;
    esac

    # Render-output-affecting paths beyond the analysis inputs above.
    case "$path" in
      src/render/*|src/render/**/*|src/render.rs|src/application.rs|src/cli.rs|\
      styles/*|styles/**/*|\
      assets/plaques/*|assets/plaques/**/*|assets/textures/*|assets/textures/**/*|\
      fonts/*|fonts/**/*|\
      scripts/render_assets.sh|scripts/render_common.sh|build.rs)
        NEEDS_RENDER=true ;;
    esac
  done < <(pf_changed_paths "$base" "$head")
}
