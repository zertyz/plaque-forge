#!/usr/bin/env bash
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
analysis_root="$repo/assets/analysis"
temporary_root="/tmp/plaque-forge"
apply=false

usage() {
  cat <<'USAGE'
Usage: ./scripts/cleanup_work.sh [--yes]

Without --yes, reports obsolete generated work. With --yes, removes legacy
assets/analysis/*.partial-* directories, pre-0.8 publication siblings, and
obsolete ML request files. Current transactional work and bounded failure
diagnostics already live under /tmp.
USAGE
}

while (( $# )); do
  case "$1" in
    --yes) apply=true ;;
    --help|-h) usage; exit 0 ;;
    *) printf 'unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

[[ "$(realpath -m "$analysis_root")" == "$repo/assets/analysis" ]] || {
  printf 'refusing unexpected analysis root: %s\n' "$analysis_root" >&2
  exit 1
}
[[ "$temporary_root" == /tmp/plaque-forge ]] || exit 1

shopt -s nullglob
partials=("$analysis_root"/*.partial-*)
requests=("$analysis_root"/*/ml-foreground/request.json)
shopt -u nullglob
mapfile -d '' legacy_siblings < <(
  find "$analysis_root" -xdev -mindepth 1 \
    \( -name '.*.incoming-[0-9]*' -o -name '.*.replaced-[0-9]*' \) \
    -print0
)

printf 'legacy analysis work directories: %d\n' "${#partials[@]}"
printf 'obsolete persisted ML requests: %d\n' "${#requests[@]}"
printf 'legacy publication siblings: %d\n' "${#legacy_siblings[@]}"
if [[ "$apply" != true ]]; then
  printf 'dry run; add --yes to remove these generated files\n'
  exit 0
fi

for path in "${partials[@]}"; do
  [[ "$path" == "$analysis_root"/*.partial-* && -d "$path" ]] || {
    printf 'refusing unexpected work directory: %s\n' "$path" >&2
    exit 1
  }
  rm -rf -- "$path"
done
for path in "${requests[@]}"; do
  [[ "$path" == "$analysis_root"/*/ml-foreground/request.json ]] || {
    printf 'refusing unexpected request file: %s\n' "$path" >&2
    exit 1
  }
  [[ -f "$path" ]] && rm -f -- "$path"
done
for path in "${legacy_siblings[@]}"; do
  [[ "$path" == "$analysis_root"/* ]] || {
    printf 'refusing unexpected publication sibling: %s\n' "$path" >&2
    exit 1
  }
  rm -rf -- "$path"
done

printf 'removed %d legacy work directories, %d obsolete request files, and %d publication siblings\n' \
  "${#partials[@]}" "${#requests[@]}" "${#legacy_siblings[@]}"
