#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "$0")/common.sh"
repo="$PF_ROOT"
analysis_root="${PLAQUE_FORGE_ANALYSIS_ROOT:-$repo/assets/analysis}"
output_root="${PLAQUE_FORGE_OUTPUT_ROOT:-$repo/output}"
temporary_root="/tmp/plaque-forge"
apply=false
prune_renders="${PLAQUE_FORGE_PRUNE_RENDERS:-false}"
cleanup_analysis=true

usage() {
  cat <<'USAGE'
Usage: ./scripts/cleanup_work.sh [options]

Without --yes, reports obsolete generated work and candidate render files.
With --yes, removes stale files according to the selected options.

Options:
  --yes            Perform the cleanup deletions.
  --prune-renders  Prune intermediate HEVC renders (*.hevc.mkv, *.mkv) and ephemeral
                   frame directories under output/, while preserving structured JSON
                   test reports, manifests, decision traces, and regression diffs.
  --all            Perform both analysis stale work cleanup and render artifact pruning.
  --output DIR     Override output directory (default: output).
  --analysis DIR   Override analysis directory (default: assets/analysis).
  -h, --help       Show this help message.
USAGE
}

while (( $# )); do
  case "$1" in
    --yes) apply=true ;;
    --prune-renders) prune_renders=true ;;
    --all) prune_renders=true; cleanup_analysis=true ;;
    --output)
      (( $# >= 2 )) || { usage >&2; exit 2; }
      output_root="$2"
      shift
      ;;
    --analysis)
      (( $# >= 2 )) || { usage >&2; exit 2; }
      analysis_root="$2"
      shift
      ;;
    --help|-h) usage; exit 0 ;;
    *) printf 'unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

partials=()
requests=()
stale_siblings=()

if [[ "$cleanup_analysis" == true && -d "$analysis_root" ]]; then
  shopt -s nullglob
  partials=("$analysis_root"/*.partial-*)
  requests=("$analysis_root"/*/ml-foreground/request.json)
  shopt -u nullglob
  mapfile -d '' stale_siblings < <(
    find "$analysis_root" -xdev -mindepth 1 \
      \( -name '.*.incoming-[0-9]*' -o -name '.*.replaced-[0-9]*' \) \
      -print0 2>/dev/null || true
  )
fi

render_mkvs=()
render_temp_dirs=()
render_bytes=0

if [[ "$prune_renders" == true && -d "$output_root" ]]; then
  shopt -s nullglob
  render_mkvs=("$output_root"/*.mkv)
  render_temp_dirs=("$output_root"/*.frames "$output_root"/.tmp-* "$output_root"/tmp-*)
  shopt -u nullglob
  for path in "${render_mkvs[@]}"; do
    if [[ -f "$path" ]]; then
      size=$(stat -c %s "$path" 2>/dev/null || wc -c < "$path" || echo 0)
      render_bytes=$((render_bytes + size))
    fi
  done
fi

if [[ "$cleanup_analysis" == true ]]; then
  printf 'stale analysis work directories: %d\n' "${#partials[@]}"
  printf 'obsolete persisted ML requests: %d\n' "${#requests[@]}"
  printf 'stale publication siblings: %d\n' "${#stale_siblings[@]}"
fi
if [[ "$prune_renders" == true ]]; then
  printf 'ephemeral video renders to prune: %d (%d bytes)\n' "${#render_mkvs[@]}" "$render_bytes"
  printf 'ephemeral frame directories to prune: %d\n' "${#render_temp_dirs[@]}"
fi

if [[ "$apply" != true ]]; then
  printf 'dry run; add --yes to remove these generated files\n'
  exit 0
fi

deleted_partials=0
for path in "${partials[@]}"; do
  [[ "$path" == "$analysis_root"/*.partial-* && -d "$path" ]] || {
    printf 'refusing unexpected work directory: %s\n' "$path" >&2
    exit 1
  }
  rm -rf -- "$path"
  deleted_partials=$((deleted_partials + 1))
done

deleted_requests=0
for path in "${requests[@]}"; do
  [[ "$path" == "$analysis_root"/*/ml-foreground/request.json ]] || {
    printf 'refusing unexpected request file: %s\n' "$path" >&2
    exit 1
  }
  if [[ -f "$path" ]]; then
    rm -f -- "$path"
    deleted_requests=$((deleted_requests + 1))
  fi
done

deleted_siblings=0
for path in "${stale_siblings[@]}"; do
  [[ "$path" == "$analysis_root"/* ]] || {
    printf 'refusing unexpected publication sibling: %s\n' "$path" >&2
    exit 1
  }
  rm -rf -- "$path"
  deleted_siblings=$((deleted_siblings + 1))
done

deleted_renders=0
deleted_render_bytes=0
if [[ "$prune_renders" == true ]]; then
  for path in "${render_mkvs[@]}"; do
    [[ "$path" == "$output_root"/*.mkv && -f "$path" ]] || {
      printf 'refusing unexpected render file: %s\n' "$path" >&2
      exit 1
    }
    size=$(stat -c %s "$path" 2>/dev/null || wc -c < "$path" || echo 0)
    rm -f -- "$path"
    deleted_renders=$((deleted_renders + 1))
    deleted_render_bytes=$((deleted_render_bytes + size))
  done

  for path in "${render_temp_dirs[@]}"; do
    [[ "$path" == "$output_root"/* && -d "$path" ]] || {
      printf 'refusing unexpected temp directory: %s\n' "$path" >&2
      exit 1
    }
    rm -rf -- "$path"
  done
fi

if [[ "$cleanup_analysis" == true ]]; then
  printf 'removed %d stale work directories, %d obsolete request files, and %d publication siblings\n' \
    "$deleted_partials" "$deleted_requests" "$deleted_siblings"
fi
if [[ "$prune_renders" == true ]]; then
  printf 'pruned %d ephemeral render(s) (%d bytes freed)\n' \
    "$deleted_renders" "$deleted_render_bytes"
fi
