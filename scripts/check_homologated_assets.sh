#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

# Audit the capability inventory first so CI retains coverage evidence even when a later
# expensive render fails. Incomplete coverage is deliberate until a human accepts each
# representative output.
cargo run --release -- homologation-coverage \
  --matrix assets/homologation/capabilities.toml \
  --report output/homologation-coverage.json

# Keep the expensive CI sentinel set intentionally small and behaviorally diverse.
# Each case uses the exact text/style sentinel from its accepted homologation contract.
run_case() {
  local case_name="$1"
  local text="$2"
  local style="$3"
  local contract="assets/homologation/$case_name/contract.toml"
  local report="output/$case_name.homologation.json"
  local rendered="output/$case_name.hevc.mkv"
  local diagnostics="output/regressions"

  [[ -f "$contract" ]] || {
    printf 'missing CI homologation contract: %s\n' "$contract" >&2
    exit 2
  }

  ./scripts/render_assets.sh \
    --text "$text" \
    --font "$PWD/fonts/NotoSerif-Regular.ttf" \
    --style "$style" \
    --fit artistic \
    "$case_name"

  cargo run --release -- homologate \
    --contract "$contract" \
    --rendered "$rendered" \
    --report "$report" \
    --diagnostics "$diagnostics"

  printf 'homologated: %s\n' "$rendered"
  printf 'acceptance:  %s\n' "$report"
}

run_case \
  "16_9_swamp_wooden_plaque" \
  "Nós que aqui estamos, por vós esperamos!" \
  "gold-shine"

run_case \
  "moving-holographic-plaque" \
  $'SEEING FURTHER\nTHAN BEFORE' \
  "classic-glow"

run_case \
  "16_9_scrapyard_iron_plaque_foreground_chains" \
  $'SEEING\nFURTHER' \
  "classic-glow"

run_case \
  "9_16_dungeon_spider_iron_temporary_plaque" \
  $'Seeing what\nothers cannot\nsee!' \
  "classic-glow"

run_case \
  "9_16_dungeon_spider_iron_plaque" \
  "Seeing what others cannot see!" \
  "gold-shine"

run_case \
  "9_16_swamp_wooden_plaque" \
  "Seeing what others cannot see!" \
  "gold-shine"

printf 'coverage:    %s\n' "output/homologation-coverage.json"
