#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "$0")/common.sh"
ROOT="$PF_ROOT"

usage() {
  cat <<'USAGE'
Usage:
  ./scripts/place_plaque.sh <asset-stem> <plaque-png> [options]

Examples:
  ./scripts/place_plaque.sh 16_9_plaqueless_swamp plaque.png
  ./scripts/place_plaque.sh 16_9_plaqueless_swamp plaque.png --bounds 180,70,900,220
  ./scripts/place_plaque.sh 16_9_plaqueless_swamp plaque.png --space screen-canvas

The command copies/normalizes the plaque PNG into the asset's scene directory,
proposes a quiet placement when --bounds is omitted, and writes placement-preview.png.
After reviewing the preview, run ./scripts/analyze_assets.sh <asset-stem>.
USAGE
}

if (( $# < 2 )); then
  usage >&2
  exit 2
fi

case "$1" in
  -h|--help) usage; exit 0 ;;
esac

name="$1"
image="$2"
shift 2

input="$ROOT/assets/$name.mp4"
[[ -f "$input" ]] || { printf 'input video not found: %s\n' "$input" >&2; exit 1; }
[[ -f "$image" ]] || { printf 'plaque PNG not found: %s\n' "$image" >&2; exit 1; }

cd "$ROOT"
pf_build_release
exec target/release/plaque-forge place-plaque \
  --input "$input" \
  --image "$image" \
  "$@"
