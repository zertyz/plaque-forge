#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/common.sh"
cd "$PF_ROOT"
backends="sam2 sam2-cutie"
device="auto"
profile="canonical"
precision="fp32"
while (( $# )); do
  case "$1" in
    --backends) backends="$2"; shift 2 ;;
    --device) device="$2"; shift 2 ;;
    --profile) profile="$2"; shift 2 ;;
    --precision) precision="$2"; shift 2 ;;
    -h|--help)
      printf 'usage: %s [--backends "LIST"] [--device D] [--profile P] [--precision P]\n' "$0"; exit 0 ;;
    *) printf 'unknown option: %s\n' "$1" >&2; exit 2 ;;
  esac
done
mapfile -t cases < <(python3 - <<'PY'
import tomllib
from pathlib import Path
matrix=tomllib.loads(Path('assets/homologation/segmentation-capabilities.toml').read_text())
seen=set()
for cap in matrix['capabilities']:
    asset=cap.get('representative_asset','')
    scene=Path('assets/scenes')/asset/'scene.toml'
    if not asset or not scene.is_file(): continue
    doc=tomllib.loads(scene.read_text())
    for layer in doc.get('layers',[]):
        if layer.get('prompts') and (asset,layer.get('id')) not in seen:
            seen.add((asset,layer['id']))
            print(asset, layer['id'])
PY
)
(( ${#cases[@]} > 0 )) || { printf 'no represented prompted segmentation capability found\n' >&2; exit 1; }
for case in "${cases[@]}"; do
  read -r asset layer <<< "$case"
  printf '\n=== Segmentation bake-off: %s / %s ===\n' "$asset" "$layer"
  ./scripts/bakeoff_segmentation_backends.sh \
    --backends "$backends" --device "$device" --profile "$profile" --precision "$precision" \
    "$asset" "$layer"
done
