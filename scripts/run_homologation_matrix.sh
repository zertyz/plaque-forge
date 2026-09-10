#!/usr/bin/env bash
# Execute scheduled/nightly full-matrix homologation across all 19 contracted video assets.
# Asserts dual-witness contracts (source preservation + title visibility) for each asset,
# compiles an overall summary report, and optionally prunes ephemeral MKV renders to bound disk usage.
set -euo pipefail

source "$(dirname "$0")/common.sh"
repo="$PF_ROOT"
cd "$repo"

output_dir="output"
diagnostics_dir="output/regressions"
summary_file="output/homologation-matrix-summary.json"
prune_renders=false
print_plan=false
dry_run=false
fail_fast=false
skip_coverage=false
assets=()

usage() {
  cat <<'USAGE'
Usage: ./scripts/run_homologation_matrix.sh [options] [asset ...]

Run full-matrix homologation against authoritative dual-witness contracts.
If no assets are specified, runs all 19 contracted assets.

Options:
  --assets "A B ..."   Explicit list of asset stems to verify.
  --output DIR         Output directory for renders and reports (default: output).
  --diagnostics DIR    Diagnostics directory for failure diffs (default: output/regressions).
  --summary FILE       Summary report JSON destination (default: output/homologation-matrix-summary.json).
  --prune-renders      Prune intermediate MKV renders after each asset or on completion.
  --print-plan         Print the list of contracted assets and render parameters and exit.
  --dry-run            Show commands without executing render or homologation.
  --fail-fast          Abort immediately on first asset contract failure.
  --no-coverage        Skip capability coverage audit step.
  -h, --help           Show this help message.
USAGE
}

while (( $# )); do
  case "$1" in
    --assets)
      read -r -a assets <<< "$2"
      shift 2
      ;;
    --output) output_dir="$2"; shift 2 ;;
    --diagnostics) diagnostics_dir="$2"; shift 2 ;;
    --summary) summary_file="$2"; shift 2 ;;
    --prune-renders) prune_renders=true; shift ;;
    --print-plan) print_plan=true; shift ;;
    --dry-run) dry_run=true; shift ;;
    --fail-fast) fail_fast=true; shift ;;
    --no-coverage) skip_coverage=true; shift ;;
    -h|--help) usage; exit 0 ;;
    -*) printf 'unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
    *) assets+=("$1"); shift ;;
  esac
done

# If no specific assets requested, discover all contracted assets from capabilities.toml
if (( ${#assets[@]} == 0 )); then
  mapfile -t assets < <(
    python3 -c '
import tomllib
with open("assets/homologation/capabilities.toml", "rb") as f:
    caps = tomllib.load(f)["capabilities"]
for c in caps:
    if "contract" in c:
        print(c["representative_asset"])
'
  )
fi

if [[ "$print_plan" == true ]]; then
  for asset in "${assets[@]}"; do
    contract="assets/homologation/$asset/contract.toml"
    if [[ ! -f "$contract" ]]; then
      printf 'error: contract missing for %s\n' "$asset" >&2
      exit 1
    fi
    python3 - "$contract" "$asset" <<'PY'
import sys, tomllib
from pathlib import Path
contract_path = Path(sys.argv[1])
asset = sys.argv[2]
with open(contract_path, "rb") as f:
    c = tomllib.load(f)
r = c["render"]
style = Path(r["style"]).name
fit = r.get("fit_mode", "artistic")
font = r.get("font_file", "NotoSerif-Regular.ttf")
print(f"{asset}\t{style}\t{fit}\t{font}")
PY
  done
  exit 0
fi

mkdir -p "$output_dir" "$diagnostics_dir"
if [[ "$summary_file" == */* ]]; then
  mkdir -p "$(dirname "$summary_file")"
fi

if [[ "$dry_run" != true ]]; then
  pf_build_release
fi

if [[ "$skip_coverage" != true ]]; then
  coverage_report="$output_dir/homologation-coverage.json"
  printf '[matrix] auditing homologation capability coverage -> %s\n' "$coverage_report" >&2
  if [[ "$dry_run" == true ]]; then
    printf '[dry-run] cargo run --release -- homologation-coverage --matrix assets/homologation/capabilities.toml --report %s\n' "$coverage_report"
  else
    target/release/plaque-forge homologation-coverage \
      --matrix assets/homologation/capabilities.toml \
      --report "$coverage_report"
  fi
fi

total_cases=${#assets[@]}
passed_cases=0
failed_cases=0
failed_assets=()

printf '[matrix] starting homologation matrix: %d asset(s)\n' "$total_cases" >&2

for asset in "${assets[@]}"; do
  contract="assets/homologation/$asset/contract.toml"
  if [[ ! -f "$contract" ]]; then
    printf '[matrix] ERROR: missing contract: %s\n' "$contract" >&2
    failed_cases=$((failed_cases + 1))
    failed_assets+=("$asset (missing contract)")
    [[ "$fail_fast" == true ]] && break
    continue
  fi

  # Extract render parameters from contract.toml
  eval "$(
    python3 - "$contract" <<'PY'
import sys, tomllib, shlex
from pathlib import Path
contract_path = Path(sys.argv[1])
with open(contract_path, "rb") as f:
    c = tomllib.load(f)
r = c["render"]
style_rel = r["style"]
style_resolved = (contract_path.parent / style_rel).resolve()
print(f"TEXT={shlex.quote(r['text'])}")
print(f"STYLE_FILE={shlex.quote(str(style_resolved))}")
print(f"FIT={shlex.quote(r.get('fit_mode', 'artistic'))}")
print(f"FONT_FILE={shlex.quote(r.get('font_file', 'NotoSerif-Regular.ttf'))}")
PY
  )"

  font_path="$repo/fonts/$FONT_FILE"
  if [[ ! -f "$font_path" ]]; then
    font_path="$repo/fonts/NotoSerif-Regular.ttf"
  fi

  rendered="$output_dir/$asset.hevc.mkv"
  report="$output_dir/$asset.homologation.json"

  printf '[matrix] [%d/%d] verifying %s (style: %s)...\n' \
    "$((passed_cases + failed_cases + 1))" "$total_cases" "$asset" "$(basename "$STYLE_FILE")" >&2

  if [[ "$dry_run" == true ]]; then
    printf '[dry-run] ./scripts/render_assets.sh --text "%s" --font "%s" --style-file "%s" --fit "%s" "%s"\n' \
      "$TEXT" "$font_path" "$STYLE_FILE" "$FIT" "$asset"
    printf '[dry-run] target/release/plaque-forge homologate --contract %s --rendered %s --report %s --diagnostics %s\n' \
      "$contract" "$rendered" "$report" "$diagnostics_dir"
    passed_cases=$((passed_cases + 1))
    continue
  fi

  # 1. Render
  if ! ./scripts/render_assets.sh \
      --text "$TEXT" \
      --font "$font_path" \
      --style-file "$STYLE_FILE" \
      --fit "$FIT" \
      "$asset" >&2; then
    printf '[matrix] FAIL: rendering failed for %s\n' "$asset" >&2
    failed_cases=$((failed_cases + 1))
    failed_assets+=("$asset (render failed)")
    [[ "$fail_fast" == true ]] && break
    continue
  fi

  # 2. Homologate
  if target/release/plaque-forge homologate \
      --contract "$contract" \
      --rendered "$rendered" \
      --report "$report" \
      --diagnostics "$diagnostics_dir" >&2; then
    printf '[matrix] PASS: %s\n' "$asset" >&2
    passed_cases=$((passed_cases + 1))
  else
    printf '[matrix] FAIL: contract violated on %s\n' "$asset" >&2
    failed_cases=$((failed_cases + 1))
    failed_assets+=("$asset (contract violation)")
    [[ "$fail_fast" == true ]] && break
  fi

  # 3. Optional render pruning
  if [[ "$prune_renders" == true || "${PLAQUE_FORGE_CI_CLEANUP:-0}" == "1" ]]; then
    rm -f -- "$rendered"
  fi
done

# Produce summary JSON
python3 - "$total_cases" "$passed_cases" "$failed_cases" "$output_dir" "$diagnostics_dir" "$summary_file" "${failed_assets[@]}" <<'PY'
import sys, json
from datetime import datetime, timezone

total_cases = int(sys.argv[1])
passed_cases = int(sys.argv[2])
failed_cases = int(sys.argv[3])
output_dir = sys.argv[4]
diagnostics_dir = sys.argv[5]
summary_file = sys.argv[6]
failed_assets = sys.argv[7:]

summary = {
    "total_cases": total_cases,
    "passed_cases": passed_cases,
    "failed_cases": failed_cases,
    "failed_assets": failed_assets,
    "timestamp": datetime.now(timezone.utc).isoformat(),
    "output_dir": output_dir,
    "diagnostics_dir": diagnostics_dir,
}
with open(summary_file, "w") as f:
    json.dump(summary, f, indent=2)
PY

printf '\n==================================================\n' >&2
printf 'HOMOLOGATION MATRIX SUMMARY:\n' >&2
printf '  Total:  %d\n' "$total_cases" >&2
printf '  Passed: %d\n' "$passed_cases" >&2
printf '  Failed: %d\n' "$failed_cases" >&2
printf '  Report: %s\n' "$summary_file" >&2
printf '==================================================\n' >&2

if (( failed_cases > 0 )); then
  printf '[matrix] FAILED assets:\n' >&2
  for fail in "${failed_assets[@]}"; do
    printf '  - %s\n' "$fail" >&2
  done
  exit 1
fi

printf '[matrix] ALL CONTRACTS PASSED!\n' >&2
exit 0
