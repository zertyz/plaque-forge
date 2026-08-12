#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
backend="sam2-cutie-vitmatte"
model="facebook/sam2.1-hiera-large"
device="auto"
force=false

usage() {
  cat <<'USAGE'
Usage: ./scripts/detect_objects.sh ASSET-STEM LAYER [options]

Generate the alpha-mask artifact for a refinement layer whose prompts are already
specified in assets/refinements/ASSET-STEM/refinement.toml.

Options:
  --backend NAME   Segmentation backend (default: sam2-cutie-vitmatte)
  --model NAME     Model identifier (default: facebook/sam2.1-hiera-large)
  --device NAME    auto, cpu, cuda, or xpu (default: auto)
  --force          Replace the layer artifact only after a successful run

Python is used only behind the segmentation-worker boundary. Run
./scripts/setup_segmentation.sh once before using this command.
USAGE
}

if [[ "${1:-}" == --help || "${1:-}" == -h ]]; then usage; exit 0; fi
(( $# >= 2 )) || { usage >&2; exit 2; }
asset="$1"; layer="$2"; shift 2
while (( $# )); do
  case "$1" in
    --backend) (( $# >= 2 )) || { usage >&2; exit 2; }; backend="$2"; shift 2 ;;
    --model) (( $# >= 2 )) || { usage >&2; exit 2; }; model="$2"; shift 2 ;;
    --device) (( $# >= 2 )) || { usage >&2; exit 2; }; device="$2"; shift 2 ;;
    --force) force=true; shift ;;
    --help|-h) usage; exit 0 ;;
    *) printf 'unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
done

cd "$root"
input="assets/$asset.mp4"
[[ -f "$input" ]] || { printf 'input video not found: %s\n' "$input" >&2; exit 1; }
[[ -x tools/segmentation-worker ]] || { printf 'segmentation worker is not executable\n' >&2; exit 1; }
[[ -x /tmp/plaque-forge-python/venv/bin/python ]] || {
  printf 'segmentation runtime is not installed; run ./scripts/setup_segmentation.sh\n' >&2
  exit 1
}

cargo build --release --quiet
args=(segment --input "$input" --layer "$layer" --worker tools/segmentation-worker \
  --backend "$backend" --model "$model" --device "$device")
[[ "$force" == true ]] && args+=(--force)
target/release/plaque-forge "${args[@]}"
