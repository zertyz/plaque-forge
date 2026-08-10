#!/usr/bin/env bash

PF_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

pf_render_options() {
  local font text
  font="${FONT:-$(fc-match -f '%{file}\n' 'DejaVu Sans' | head -n 1)}"
  text="${TITLE_TEXT:-Analises desta 3a. feira, 1 de Agosto}"
  PF_RENDER_OPTIONS=(--text "$text" --font "$font")

  local variable flag value
  while (( $# )); do
    variable="$1"
    flag="$2"
    value="${!variable:-}"
    if [[ -n "$value" ]]; then
      PF_RENDER_OPTIONS+=("$flag" "$value")
    fi
    shift 2
  done
}

pf_cases() {
  if (( $# )); then
    PF_CASES=("$@")
    return
  fi

  local input
  PF_CASES=()
  shopt -s nullglob
  for input in "$PF_ROOT"/assets/*.mp4; do
    PF_CASES+=("$(basename "$input" .mp4)")
  done
  shopt -u nullglob
  if (( ${#PF_CASES[@]} == 0 )); then
    printf 'no input videos found in %s/assets\n' "$PF_ROOT" >&2
    return 1
  fi
}

pf_configure() {
  pf_render_options \
    FIT --fit \
    FONT_SIZE --font-size \
    SUPERSAMPLING --supersampling \
    TARGET_FILL --target-fill \
    MAX_LINES --max-lines \
    PADDING --padding \
    LINE_HEIGHT --line-height \
    STROKE_WIDTH --stroke-width \
    TEXT_COLOR --text-color \
    STROKE_COLOR --stroke-color \
    GLOW_COLOR --glow-color \
    GLOW_RADIUS --glow-radius \
    TEXT_ALIGN --text-align \
    VERTICAL_ALIGN --vertical-align
  pf_cases "$@"
}
