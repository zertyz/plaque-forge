#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
force=false
force_ml=false
use_ml=true
backend="sam2-cutie-vitmatte"
model="facebook/sam2.1-hiera-large"
device="auto"
cases=()

usage() {
  cat <<'USAGE'
Usage: ./scripts/analyze_assets.sh [options] [asset-stem ...]

Do the expensive scene work once and cache everything reusable. The script:
  - builds Plaque Forge;
  - reuses a cache only when Rust verifies it is current;
  - runs automatic writing-surface detection and tracking;
  - builds canonical/writable masks and foreground occlusion;
  - materializes any prompted ML refinement layers that are still missing;
  - retains partial diagnostics when a scene still needs human refinement.

Options:
  --force          Rebuild selected Rust scene-analysis caches. Cached ML layers are reused.
  --force-ml       Regenerate prompted ML layer artifacts too (implies Python when prompts exist).
  --no-ml          Do not invoke optional Python ML segmentation layers.
  --backend NAME   Segmentation backend (default: sam2-cutie-vitmatte).
  --model NAME     Segmentation model (default: facebook/sam2.1-hiera-large).
  --device NAME    auto, cpu, cuda, or xpu (default: auto).

For the full workflow, run ./scripts/setup_segmentation.sh once first. Its Python,
model, source, and cache files live under /tmp/plaque-forge-python, not $HOME.
USAGE
}

while (( $# )); do
  case "$1" in
    --force) force=true; shift ;;
    --force-ml) force_ml=true; force=true; shift ;;
    --no-ml) use_ml=false; shift ;;
    --backend) (( $# >= 2 )) || { usage >&2; exit 2; }; backend="$2"; shift 2 ;;
    --model) (( $# >= 2 )) || { usage >&2; exit 2; }; model="$2"; shift 2 ;;
    --device) (( $# >= 2 )) || { usage >&2; exit 2; }; device="$2"; shift 2 ;;
    --help|-h) usage; exit 0 ;;
    -*) printf 'unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
    *) cases+=("$1"); shift ;;
  esac
done

if [[ "$force_ml" == true && "$use_ml" == false ]]; then
  printf 'error: --force-ml and --no-ml are mutually exclusive\n' >&2
  exit 2
fi

cd "$root"

if [[ "$use_ml" == true ]]; then
  printf '[ml] enabled: runtime=/tmp/plaque-forge-python, backend=%s, model=%s, device=%s, force_ml=%s\n' "$backend" "$model" "$device" "$force_ml"
  printf '[ml] inspect workers/history: ./scripts/ml_status.sh (log: /tmp/plaque-forge-python/worker-runs.jsonl)\n'
  [[ -x tools/segmentation-worker ]] || {
    printf 'segmentation worker is not executable: tools/segmentation-worker\n' >&2
    exit 1
  }
  [[ -x /tmp/plaque-forge-python/venv/bin/python && -f /tmp/plaque-forge-python/.complete ]] || {
    printf '%s\n' \
      'optional ML runtime is not installed.' \
      'Run ./scripts/setup_segmentation.sh once, or use --no-ml for the pure-Rust analysis path.' >&2
    exit 1
  }
else
  printf '[ml] disabled by --no-ml; Python will not run\n'
fi

cargo build --release --quiet

if (( ${#cases[@]} == 0 )); then
  shopt -s nullglob
  for input in assets/*.mp4; do cases+=("$(basename "$input" .mp4)"); done
  shopt -u nullglob
fi
(( ${#cases[@]} > 0 )) || { printf 'no assets/*.mp4 files found\n' >&2; exit 1; }

failures=0
for name in "${cases[@]}"; do
  input="assets/$name.mp4"
  [[ -f "$input" ]] || { printf 'input video not found: %s\n' "$input" >&2; exit 1; }

  args=(analyze --input "$input" --progress always)
  if [[ "$force" == true ]]; then
    args+=(--force)
  else
    args+=(--if-needed)
  fi
  if [[ "$use_ml" == true ]]; then
    args+=(
      --segmentation-worker tools/segmentation-worker
      --segmentation-backend "$backend"
      --segmentation-model "$model"
      --segmentation-device "$device"
    )
    [[ "$force_ml" == true ]] && args+=(--force-ml)
  fi

  printf '\n=== Analyze %s ===\n' "$name"
  if ! target/release/plaque-forge "${args[@]}"; then
    printf '## ANALYSIS NEEDS ATTENTION: %s\n' "$name" >&2
    failures=$((failures + 1))
  fi
done

if (( failures > 0 )); then
  printf '\n%d asset(s) still require review. Partial diagnostics were retained under assets/analysis/*.partial-*\n' "$failures" >&2
  exit 1
fi
