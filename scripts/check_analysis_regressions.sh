#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "$0")/common.sh"
cd "$PF_ROOT"

# These are the six concrete non-ML regression witnesses captured in August 2026:
# four prompted layers must be skippable/reusable in --no-ml mode, while two
# reviewed static artifacts must remain valid without generated-worker identity.
./scripts/analyze_assets.sh --force --no-ml \
  16_9_swamp_wooden_plaque \
  9_16_dungeon_spider_iron_temporary_plaque \
  9_16_swamp_wooden_plaque \
  16_9_scrapyard_iron_plaque_foreground_chains \
  16_9_swamp_iron_plaque \
  16_9_swamp_wooden_plaque_foreground_vines_and_lizard
