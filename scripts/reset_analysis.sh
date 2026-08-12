#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
analysis_root="$root/assets/analysis"
confirm=false

usage() {
  cat <<'USAGE'
Usage: ./scripts/reset_analysis.sh --yes

Delete only Plaque Forge generated scene-analysis caches under assets/analysis/.
Human refinements, source videos, plaque assets, rendered outputs, and the optional
Python runtime/model cache under /tmp/plaque-forge-python are preserved.

Bounded failure diagnostics under /tmp/plaque-forge/failures are also preserved;
successful reanalysis purges the corresponding asset's retained failures.

After reset, rebuild everything with:
  ./scripts/analyze_assets.sh
USAGE
}

while (( $# )); do
  case "$1" in
    --yes) confirm=true; shift ;;
    --help|-h) usage; exit 0 ;;
    *) printf 'unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ "$confirm" == true ]] || {
  printf 'refusing to delete analysis caches without --yes\n\n' >&2
  usage >&2
  exit 2
}

canonical_repo="$(realpath -m "$root")"
canonical_root="$(realpath -m "$analysis_root")"
expected="$canonical_repo/assets/analysis"
[[ "$canonical_root" == "$expected" ]] || {
  printf 'refusing unexpected analysis root: %s\n' "$canonical_root" >&2
  exit 1
}

mkdir -p "$analysis_root"
shopt -s dotglob nullglob
entries=("$analysis_root"/*)
if (( ${#entries[@]} )); then
  printf 'deleting %d generated analysis cache entr%s from %s\n' \
    "${#entries[@]}" "$([[ ${#entries[@]} == 1 ]] && printf y || printf ies)" "$analysis_root"
  rm -rf -- "${entries[@]}"
else
  printf 'analysis cache is already empty: %s\n' "$analysis_root"
fi
shopt -u dotglob nullglob
mkdir -p "$analysis_root"
printf 'done; refinements and Python/model caches were not touched\n'
