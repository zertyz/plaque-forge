#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

# These are the six concrete non-ML regression witnesses captured in August 2026:
# four prompted layers must be skippable/reusable in --no-ml mode, while two
# reviewed static artifacts must remain valid without generated-worker identity.
./scripts/analyze_assets.sh --force --no-ml \
  16_9_swamp_wooden_plaque \
  9_16_dungeon_spider_iron_temporary_plaque \
  9_16_swamp_wooden_plaque \
  rusty-plaque-with-object-in-front-parallax-and-plaque-moves \
  swamp-rusty-plaque \
  swamp-wooden-plaque-with-foreground-objects
