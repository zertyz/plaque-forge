#!/usr/bin/env bash
# Shared option parsing for the high-level rendering and validation commands.

PF_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

pf_die() {
  printf 'error: %s\n' "$*" >&2
  exit 2
}

pf_font_match() {
  local family="$1"
  fc-match -f '%{file}\n' "$family" | head -n 1
}

pf_append_env_option() {
  local variable="$1" flag="$2" value
  value="${!variable:-}"
  if [[ -n "$value" ]]; then
    PF_RENDER_OPTIONS+=("$flag" "$value")
  fi
}

pf_all_cases() {
  local input
  PF_CASES=()
  shopt -s nullglob
  for input in "$PF_ROOT"/assets/*.mp4; do
    PF_CASES+=("$(basename "$input" .mp4)")
  done
  shopt -u nullglob
  (( ${#PF_CASES[@]} > 0 )) || pf_die "no input videos found in $PF_ROOT/assets"
}

pf_configure_render() {
  local text="${TITLE_TEXT:-}"
  local text_file=""
  local font="${FONT:-}"
  local font_family="${FONT_FAMILY:-}"
  local style="${STYLE:-}"
  local style_file="${STYLE_FILE:-}"
  local -a cases=()

  while (( $# )); do
    case "$1" in
      --text)
        (( $# >= 2 )) || pf_die "--text requires a value"
        text="$2"; shift 2 ;;
      --text-file)
        (( $# >= 2 )) || pf_die "--text-file requires a path"
        text_file="$2"; shift 2 ;;
      --font)
        (( $# >= 2 )) || pf_die "--font requires a path"
        font="$2"; shift 2 ;;
      --font-family)
        (( $# >= 2 )) || pf_die "--font-family requires a fontconfig family/pattern"
        font_family="$2"; shift 2 ;;
      --style) (( $# >= 2 )) || pf_die "$1 requires a preset name"; style="$2"; shift 2 ;;
      --style-file) (( $# >= 2 )) || pf_die "$1 requires a path"; style_file="$2"; shift 2 ;;
      --fit) (( $# >= 2 )) || pf_die "$1 requires a value"; FIT="$2"; shift 2 ;;
      --font-size) (( $# >= 2 )) || pf_die "$1 requires a value"; FONT_SIZE="$2"; shift 2 ;;
      --supersampling) (( $# >= 2 )) || pf_die "$1 requires a value"; SUPERSAMPLING="$2"; shift 2 ;;
      --target-fill) (( $# >= 2 )) || pf_die "$1 requires a value"; TARGET_FILL="$2"; shift 2 ;;
      --max-lines) (( $# >= 2 )) || pf_die "$1 requires a value"; MAX_LINES="$2"; shift 2 ;;
      --padding) (( $# >= 2 )) || pf_die "$1 requires a value"; PADDING="$2"; shift 2 ;;
      --line-height) (( $# >= 2 )) || pf_die "$1 requires a value"; LINE_HEIGHT="$2"; shift 2 ;;
      --stroke-width) (( $# >= 2 )) || pf_die "$1 requires a value"; STROKE_WIDTH="$2"; shift 2 ;;
      --text-color) (( $# >= 2 )) || pf_die "$1 requires a value"; TEXT_COLOR="$2"; shift 2 ;;
      --stroke-color) (( $# >= 2 )) || pf_die "$1 requires a value"; STROKE_COLOR="$2"; shift 2 ;;
      --glow-color) (( $# >= 2 )) || pf_die "$1 requires a value"; GLOW_COLOR="$2"; shift 2 ;;
      --glow-radius) (( $# >= 2 )) || pf_die "$1 requires a value"; GLOW_RADIUS="$2"; shift 2 ;;
      --shadow-offset-x) (( $# >= 2 )) || pf_die "$1 requires a value"; SHADOW_OFFSET_X="$2"; shift 2 ;;
      --shadow-offset-y) (( $# >= 2 )) || pf_die "$1 requires a value"; SHADOW_OFFSET_Y="$2"; shift 2 ;;
      --shadow-blur-radius) (( $# >= 2 )) || pf_die "$1 requires a value"; SHADOW_BLUR_RADIUS="$2"; shift 2 ;;
      --shadow-color) (( $# >= 2 )) || pf_die "$1 requires a value"; SHADOW_COLOR="$2"; shift 2 ;;
      --text-align) (( $# >= 2 )) || pf_die "$1 requires a value"; TEXT_ALIGN="$2"; shift 2 ;;
      --vertical-align) (( $# >= 2 )) || pf_die "$1 requires a value"; VERTICAL_ALIGN="$2"; shift 2 ;;
      --)
        shift
        cases+=("$@")
        break ;;
      --help|-h)
        return 64 ;;
      -* ) pf_die "unknown render option: $1" ;;
      * ) cases+=("$1"); shift ;;
    esac
  done

  if [[ -n "$text" && -n "$text_file" ]]; then
    pf_die "use either --text/TITLE_TEXT or --text-file, not both"
  fi
  if [[ -z "$text" && -z "$text_file" ]]; then
    pf_die "title text is required; use --text '...' or TITLE_TEXT='...'"
  fi
  if [[ -n "$style" && -n "$style_file" ]]; then
    pf_die "use either --style/STYLE or --style-file/STYLE_FILE, not both"
  fi
  if [[ -n "$style" ]]; then
    [[ "$style" != */* && "$style" != *..* ]] || pf_die "--style expects a preset name, not a path"
    style_file="$PF_ROOT/styles/$style.toml"
    [[ -f "$style_file" ]] || pf_die "style preset not found: $style (expected $style_file)"
  fi
  if [[ -n "$font" && -n "$font_family" ]]; then
    pf_die "use either --font/FONT or --font-family/FONT_FAMILY, not both"
  fi
  if [[ -z "$font" ]]; then
    # Default to the repository-pinned reference font so plain renders do not
    # depend on whichever fonts fontconfig happens to prefer on this machine.
    local bundled_font="$PF_ROOT/fonts/NotoSerif-Regular.ttf"
    if [[ -n "$font_family" ]]; then
      font="$(pf_font_match "$font_family")"
    elif [[ -f "$bundled_font" ]]; then
      font="$bundled_font"
    else
      font="$(pf_font_match "Noto Serif")"
    fi
  fi
  [[ -n "$font" && -f "$font" ]] || pf_die "font file not found: ${font:-<none>}"

  PF_RENDER_OPTIONS=(--font "$font")
  if [[ -n "$text_file" ]]; then
    PF_RENDER_OPTIONS+=(--text-file "$text_file")
  else
    PF_RENDER_OPTIONS+=(--text "$text")
  fi

  if [[ -n "$style_file" ]]; then
    PF_RENDER_OPTIONS+=(--style-file "$style_file")
  fi
  pf_append_env_option FIT --fit
  pf_append_env_option FONT_SIZE --font-size
  pf_append_env_option SUPERSAMPLING --supersampling
  pf_append_env_option TARGET_FILL --target-fill
  pf_append_env_option MAX_LINES --max-lines
  pf_append_env_option PADDING --padding
  pf_append_env_option LINE_HEIGHT --line-height
  pf_append_env_option STROKE_WIDTH --stroke-width
  pf_append_env_option TEXT_COLOR --text-color
  pf_append_env_option STROKE_COLOR --stroke-color
  pf_append_env_option GLOW_COLOR --glow-color
  pf_append_env_option GLOW_RADIUS --glow-radius
  pf_append_env_option SHADOW_OFFSET_X --shadow-offset-x
  pf_append_env_option SHADOW_OFFSET_Y --shadow-offset-y
  pf_append_env_option SHADOW_BLUR_RADIUS --shadow-blur-radius
  pf_append_env_option SHADOW_COLOR --shadow-color
  pf_append_env_option TEXT_ALIGN --text-align
  pf_append_env_option VERTICAL_ALIGN --vertical-align

  if (( ${#cases[@]} )); then
    PF_CASES=("${cases[@]}")
  else
    pf_all_cases
  fi
}
