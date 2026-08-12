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
  - automatically runs ML segmentation when Rust detects a useful foreground crossing;
  - materializes any human-prompted ML refinement layers that are still missing;
  - retains only compact, bounded failure diagnostics under /tmp when review is needed;
  - builds review.html + review.txt automatically for failed quality gates.

Options:
  --force          Rebuild selected scene-analysis caches. Valid human-prompted ML artifacts are reused.
  --force-ml       Regenerate all ML layer artifacts instead of reusing valid results.
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

  args=(analyze --input "$input" --source-is-text-free --progress always)
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

    # Turn compact retained diagnostics into a human triage report immediately.
    retained="$(ls -1dt "/tmp/plaque-forge/failures/$name/"* 2>/dev/null | head -n 1 || true)"
    if [[ -n "$retained" && -d "$retained" ]]; then
      review_args=(review --analysis "$retained")
      refinement="assets/refinements/$name/refinement.toml"
      [[ -f "$refinement" ]] && review_args+=(--refinement "$refinement")
      printf '[review] building triage report from %s\n' "$retained" >&2
      target/release/plaque-forge "${review_args[@]}" || \
        printf '[review] could not build review report; compact diagnostics remain in %s\n' "$retained" >&2
    fi
  fi
done

if (( failures > 0 )); then
  printf '\n%d asset(s) still require review. Compact diagnostics are under /tmp/plaque-forge/failures/ (newest three per asset, seven-day limit).\n' "$failures" >&2
  exit 1
fi
