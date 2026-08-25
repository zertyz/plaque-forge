#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

# These are the six concrete non-ML regression witnesses captured in August 2026:
# four prompted layers must be skippable/reusable in --no-ml mode, while two
# reviewed static artifacts must remain valid without generated-worker identity.
#
# The witnesses run `analyze --force --no-ml` in place. In CI the workspace is
# ephemeral, so overwriting generated analysis is free. In a persistent checkout
# the same run would strip machine-generated ML artifacts (automatic foreground,
# prompted-layer caches) that took hours to produce, so it is refused unless the
# caller explicitly accepts the rewrite.

if [[ "${GITHUB_ACTIONS:-}" != "true" && "${PLAQUE_FORGE_ALLOW_NO_ML_REWRITE:-}" != "1" ]]; then
  clobbered=()
  for name in \
    16_9_swamp_wooden_plaque \
    9_16_dungeon_spider_iron_temporary_plaque \
    9_16_swamp_wooden_plaque \
    16_9_scrapyard_iron_plaque_foreground_chains \
    16_9_swamp_iron_plaque \
    16_9_swamp_wooden_plaque_foreground_vines_and_lizard; do
    if grep -qs '^automatic_ml_foreground = true' "assets/analysis/$name/manifest.toml" \
      || [[ -d "assets/analysis/$name/ml-foreground" ]]; then
      clobbered+=("$name")
    fi
  done
  if (( ${#clobbered[@]} )); then
    printf 'refusing to overwrite machine-generated ML analysis for:\n' >&2
    printf '  %s\n' "${clobbered[@]}" >&2
    printf 'This gate rewrites analysis in place with --force --no-ml; CI does that on an\n' >&2
    printf 'ephemeral checkout. Locally, re-run with PLAQUE_FORGE_ALLOW_NO_ML_REWRITE=1\n' >&2
    printf 'after committing or backing up the generated analysis you care about.\n' >&2
    exit 3
  fi
fi

./scripts/analyze_assets.sh --force --no-ml \
  16_9_swamp_wooden_plaque \
  9_16_dungeon_spider_iron_temporary_plaque \
  9_16_swamp_wooden_plaque \
  16_9_scrapyard_iron_plaque_foreground_chains \
  16_9_swamp_iron_plaque \
  16_9_swamp_wooden_plaque_foreground_vines_and_lizard
