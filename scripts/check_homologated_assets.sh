#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

# Keep this list intentionally small and high-value. These cases are the visual
# equivalents of integration tests: they exercise a real source, real analysis,
# real typography, real compositing, and a delivery encode.
case_name="16_9_dungeon_spider_iron_plaque"
contract="assets/homologation/$case_name/contract.toml"
report="output/$case_name.homologation.json"
rendered="output/$case_name.hevc.mkv"

./scripts/render_assets.sh \
  --text "WITH THE BIGGER POTENTIAL OF SEEING FURTHER" \
  --font-family "Noto Serif" \
  --style gold-shine \
  --fit artistic \
  "$case_name"

target/release/plaque-forge homologate \
  --contract "$contract" \
  --rendered "$rendered" \
  --report "$report"

printf 'homologated: %s\n' "$rendered"
printf 'acceptance:  %s\n' "$report"
