#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/common.sh"
cd "$PF_ROOT"
backends="sam2 sam2-cutie"
device="auto"
profile="canonical"
precision="fp32"
all_prompted=0
min_iou=""
max_mean_absolute=""
gate=0

while (( $# )); do
  case "$1" in
    --backends) backends="$2"; shift 2 ;;
    --device) device="$2"; shift 2 ;;
    --profile) profile="$2"; shift 2 ;;
    --precision) precision="$2"; shift 2 ;;
    --min-iou) min_iou="$2"; shift 2 ;;
    --max-mean-absolute) max_mean_absolute="$2"; shift 2 ;;
    --gate) gate=1; shift ;;
    --all-prompted) all_prompted=1; shift ;;
    -h|--help)
      cat <<'USAGE'
usage: ./scripts/bakeoff_segmentation_matrix.sh [options]

Options:
  --backends "LIST"           space-separated explicit backends (default: "sam2 sam2-cutie")
  --device D                  execution device (default: auto)
  --profile P                 preview|balanced|canonical (default: canonical)
  --precision P               fp32|bf16 (default: fp32)
  --min-iou FLOAT             minimum acceptable IoU against canonical reference (e.g. 0.95)
  --max-mean-absolute FLOAT   maximum mean absolute drift against canonical reference (e.g. 0.05)
  --gate                      abort with non-zero exit code if any layer regresses
  --all-prompted              evaluate all prompted scene layers across all assets
USAGE
      exit 0 ;;
    *) printf 'unknown option: %s\n' "$1" >&2; exit 2 ;;
  esac
done

export ALL_PROMPTED="$all_prompted"
mapfile -t cases < <(python3 - <<'PY'
import os
import tomllib
from pathlib import Path

all_prompted = os.environ.get("ALL_PROMPTED") == "1"
seen = set()

if all_prompted:
    for scene in sorted(Path('assets/scenes').glob('*/scene.toml')):
        asset = scene.parent.name
        doc = tomllib.loads(scene.read_text())
        for layer in doc.get('layers', []):
            if layer.get('prompts') and (asset, layer.get('id')) not in seen:
                seen.add((asset, layer['id']))
                print(asset, layer['id'])
else:
    matrix = tomllib.loads(Path('assets/homologation/segmentation-capabilities.toml').read_text())
    for cap in matrix['capabilities']:
        asset = cap.get('representative_asset', '')
        scene = Path('assets/scenes') / asset / 'scene.toml'
        if not asset or not scene.is_file(): continue
        doc = tomllib.loads(scene.read_text())
        for layer in doc.get('layers', []):
            if layer.get('prompts') and (asset, layer.get('id')) not in seen:
                seen.add((asset, layer['id']))
                print(asset, layer['id'])
PY
)
(( ${#cases[@]} > 0 )) || { printf 'no represented prompted segmentation capability found\n' >&2; exit 1; }

matrix_failed=0
for case in "${cases[@]}"; do
  read -r asset layer <<< "$case"
  printf '\n=== Segmentation bake-off: %s / %s ===\n' "$asset" "$layer"
  cmd=(
    ./scripts/bakeoff_segmentation_backends.sh
    --backends "$backends" --device "$device" --profile "$profile" --precision "$precision"
  )
  if [[ -n "$min_iou" ]]; then
    cmd+=(--min-iou "$min_iou")
  fi
  if [[ -n "$max_mean_absolute" ]]; then
    cmd+=(--max-mean-absolute "$max_mean_absolute")
  fi
  if (( gate )); then
    cmd+=(--gate)
  fi
  cmd+=("$asset" "$layer")

  if ! "${cmd[@]}"; then
    printf '[REGRESSION ERROR] Bake-off failure on %s / %s\n' "$asset" "$layer" >&2
    matrix_failed=1
  fi
done

if (( gate && matrix_failed )); then
  printf '\n[CRITICAL] Multi-asset segmentation regression gate failed!\n' >&2
  exit 1
fi
