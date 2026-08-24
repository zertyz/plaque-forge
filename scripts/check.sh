#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
bash -n scripts/*.sh tools/segmentation-worker
./scripts/check_segmentation.sh

set +e
./scripts/render_assets.sh --plaque-forge-invalid-option >/dev/null 2>&1
parser_status=$?
set -e
if (( parser_status != 2 )); then
  printf 'render option parser returned %d; expected 2\n' "$parser_status" >&2
  exit 1
fi

# The helper-script default font must be the repository-pinned reference font,
# independently of whichever fonts fontconfig happens to prefer on this machine.
default_font="$(
  env -u FONT -u FONT_FAMILY bash -c '
    source scripts/render_common.sh
    pf_configure_render --text gate >/dev/null
    printf "%s" "${PF_RENDER_OPTIONS[1]}"
  '
)"
if [[ "$default_font" != "$PWD/fonts/NotoSerif-Regular.ttf" ]]; then
  printf 'default render font is "%s"; expected the bundled fonts/NotoSerif-Regular.ttf\n' \
    "$default_font" >&2
  exit 1
fi

if git -c safe.directory="*" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  git -c safe.directory="*" diff --check
fi
