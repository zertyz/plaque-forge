#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

# Audit the capability inventory first so CI retains coverage evidence even when a later
# expensive render fails. Incomplete coverage is deliberate until a human accepts each
# representative output.
cargo run --release -- homologation-coverage \
  --matrix assets/homologation/capabilities.toml \
  --report output/homologation-coverage.json

# Keep the expensive CI sentinel set intentionally small and high-value. These cases are
# the visual equivalents of integration tests: they exercise a real source, real analysis,
# real typography, real compositing, and a delivery encode.
cases=(
  "16_9_dungeon_spider_iron_plaque:WITH THE BIGGER POTENTIAL OF SEEING FURTHER"
  "16_9_swamp_wooden_plaque:Nós que aqui estamos, por vós esperamos!"
  "9_16_dungeon_spider_iron_plaque:WITH THE BIGGER POTENTIAL OF SEEING FURTHER"
)

for item in "${cases[@]}"; do
  case_name="${item%%:*}"
  text="${item#*:}"
  contract="assets/homologation/$case_name/contract.toml"
  report="output/$case_name.homologation.json"
  rendered="output/$case_name.hevc.mkv"
  diagnostics="output/regressions"

  ./scripts/render_assets.sh \
    --text "$text" \
    --font-family "Noto Serif" \
    --style gold-shine \
    --fit artistic \
    "$case_name"

  cargo run --release -- homologate \
    --contract "$contract" \
    --rendered "$rendered" \
    --report "$report" \
    --diagnostics "$diagnostics"

  printf 'homologated: %s\n' "$rendered"
  printf 'acceptance:  %s\n' "$report"
done

printf 'coverage:    %s\n' "output/homologation-coverage.json"
