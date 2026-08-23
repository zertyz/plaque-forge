#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
source "$(dirname "$0")/change_scope_lib.sh"

pf_classify_changes "${1:-}" "${2:-HEAD}"

printf 'needs_render=%s\n' "$NEEDS_RENDER"
