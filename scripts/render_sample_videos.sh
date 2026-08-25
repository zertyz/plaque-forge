#!/usr/bin/env bash
# Render the published sample-video set: every bundled asset with its policy style,
# verified per asset as acceptance evidence before publication.
#
# Style policy: real plaque surfaces carry the golden shine; plaque-less and
# background videos carry the classic glow. The plan is printed by --print-plan
# and asserted by tests/cli_workflows.rs.
set -euo pipefail

source "$(dirname "$0")/render_common.sh"

SAMPLE_TEXT="${SAMPLE_TEXT:-plaque-forge: High Quality & Advanced Text Placing in Videos}"
# Pinned to the repository reference font for reproducible published samples.
SAMPLE_FONT="${SAMPLE_FONT:-$PF_ROOT/fonts/NotoSerif-Regular.ttf}"
SAMPLE_FIT="${SAMPLE_FIT:-artistic}"

PF_SAMPLE_DIR="output/sample_videos"

# Showcase assets drive the MP4/WebP previews (visual variety across styles,
# surfaces, and scene types).
preview_cases=(
  16_9_background_digifall
  16_9_mountain_top_day_hummingbird_cloudy_plaque
  16_9_scrapyard_iron_plaque_foreground_chains
  16_9_swamp_wooden_plaque
)

# Acceptance sentinels are plaque scenes only: verify's untouched-scene
# threshold presumes a mostly-static preserved region, which fully animated
# background/plaque-less sources fail on HEVC encoder noise alone (their
# compositing behavior is identical, so per-scene-class coverage is what the
# default verification set represents). --verify-all opts into everything.
verify_cases=(
  16_9_swamp_wooden_plaque
  16_9_scrapyard_iron_plaque_foreground_chains
  9_16_swamp_wooden_plaque
  9_16_scrappy_datacenter_holographic_plaque
)

declare -A is_preview_case=()
for stem in "${preview_cases[@]}"; do is_preview_case["$stem"]=1; done

declare -A is_verify_case=()
for stem in "${verify_cases[@]}"; do is_verify_case["$stem"]=1; done

usage() {
  cat <<'USAGE'
Usage:
  ./scripts/render_sample_videos.sh [options]

Renders every bundled asset with the sample title text and its policy style
(gold-shine for real plaques, classic-glow otherwise) and stages everything
under output/sample_videos/<style>/.

Options:
  --print-plan     Print the "<style>\\t<asset-stem>" plan and exit.
  --showcase-only  Limit the plan to the showcase assets above.
  --previews       Also encode small H.264 MP4 previews for showcase assets.
  --webp           Also encode animated WebP loops of the showcase assets
                   into docs/media/ for the README (a manual, committed refresh).
  --from-existing  Skip rendering/verification and only encode --previews/--webp
                   from previously rendered videos.
  --no-verify      Skip the per-asset verification pass entirely.
  --verify-all     Verify every video instead of only the sentinel set.
  --stage-release BATCH DEST
                   Flatten a rendered batch directory into unique release asset
                   names under DEST: <style>/<stem>.hevc.mkv becomes
                   <stem>.<style>.hevc.mkv, batch-root previews pass through,
                   render sidecars are left behind. No rendering happens.
  --help           Show this help.

By default the four representative plaque-scene sentinels pass through the
lossless validation render (validate_assets.sh) before the batch is considered
accepted; verify's untouched-scene thresholds presume lossless frames.

Environment overrides: SAMPLE_TEXT, SAMPLE_FONT, SAMPLE_FIT.
USAGE
}

sample_style_for_stem() {
  local stem="$1"
  if [[ "$stem" == *plaque* && "$stem" != *plaqueless* ]]; then
    printf '%s' gold-shine
  else
    printf '%s' classic-glow
  fi
}

print_plan() {
  local stem
  pf_asset_cases
  for stem in "${PF_CASES[@]}"; do
    if $showcase_only && [[ -z "${is_preview_case[$stem]:-}" ]]; then
      continue
    fi
    printf '%s\t%s\n' "$(sample_style_for_stem "$stem")" "$stem"
  done | LC_ALL=C sort
}

# Flatten a rendered batch (output/sample_videos layout) into the unique,
# style-prefixed asset names published on the release. Pure file staging so it
# stays unit-testable; the workflow only adds report copying and gh calls.
stage_release_batch() {
  local src="$1" dest="$2" staged=0 entry style stem base
  [[ -d "$src" ]] || pf_die "render batch directory not found: $src"
  mkdir -p "$dest"

  shopt -s nullglob
  for entry in "$src"/*/*.hevc.mkv; do
    [[ -f "$entry" ]] || continue
    style="$(basename "$(dirname "$entry")")"
    base="$(basename "$entry")"
    stem="${base%.hevc.mkv}"
    cp -- "$entry" "$dest/$stem.$style.hevc.mkv"
    staged=$((staged + 1))
  done
  # Previews are encoded at the batch root, outside the per-style directories.
  for entry in "$src"/*.preview.mp4; do
    [[ -f "$entry" ]] || continue
    cp -- "$entry" "$dest/$(basename "$entry")"
    staged=$((staged + 1))
  done
  shopt -u nullglob

  (( staged > 0 )) || pf_die "no *.hevc.mkv renders found under $src"
  printf 'staged %d release asset(s) under %s\n' "$staged" "$dest" >&2
}

print_previews=false
print_webp=false
from_existing=false
showcase_only=false
verify_mode=representatives
while (( $# )); do
  case "$1" in
    --print-plan) print_plan; exit 0 ;;
    --stage-release)
      (( $# >= 3 )) || pf_die "--stage-release requires <batch-dir> <dest-dir>"
      stage_release_batch "$2" "$3"
      exit $?
      ;;
    --previews) print_previews=true; shift ;;
    --webp) print_webp=true; shift ;;
    --from-existing) from_existing=true; shift ;;
    --showcase-only) showcase_only=true; shift ;;
    --no-verify) verify_mode=none; shift ;;
    --verify-all) verify_mode=all; shift ;;
    --help|-h) usage; exit 0 ;;
    *) pf_die "unknown option: $1 (see --help)" ;;
  esac
done

cd "$PF_ROOT"
if ! $from_existing; then
  pf_build_release
fi
mkdir -p "$PF_SAMPLE_DIR"


should_verify() {
  local stem="$1"
  case "$verify_mode" in
    all) return 0 ;;
    representatives) [[ -n "${is_verify_case[$stem]:-}" ]] ;;
    *) return 1 ;;
  esac
}

declare -A staged=()
failures=0
while IFS=$'\t' read -r style stem; do
  staged["$stem"]="$style"
  if $from_existing; then
    [[ -f "$PF_SAMPLE_DIR/$style/$stem.hevc.mkv" ]] || {
      printf 'missing previous render: %s\n' "$PF_SAMPLE_DIR/$style/$stem.hevc.mkv" >&2
      failures=$((failures + 1))
    }
    continue
  fi

  # Render straight into the per-style directory so the transactional bundle
  # (video, text mask, manifest, decision trace) stays internally consistent.
  style_dir="$PF_SAMPLE_DIR/$style"
  mkdir -p "$style_dir"
  final="$style_dir/$stem.hevc.mkv"
  printf '## rendering %s (%s)\n' "$stem" "$style" >&2
  # One unusable asset (for example a stale analysis cache awaiting canonical
  # regeneration) must not block the remaining samples; the batch still exits
  # nonzero so incomplete sets can never pass silently.
  if ! ./scripts/render_assets.sh \
    --text "$SAMPLE_TEXT" \
    --font "$SAMPLE_FONT" \
    --style-file "styles/$style.toml" \
    --fit "$SAMPLE_FIT" \
    --output-dir "$style_dir" \
    "$stem" </dev/null; then
    printf '## SAMPLE RENDER FAILED FOR %s; continuing\n' "$stem" >&2
    failures=$((failures + 1))
    continue
  fi

  if should_verify "$stem"; then
    # Acceptance reuses the project's lossless validation pass: verify's
    # untouched-scene thresholds are calibrated for lossless renders, so the
    # delivery HEVC itself is never the verification artifact.
    if ! ./scripts/validate_assets.sh \
      --text "$SAMPLE_TEXT" \
      --font "$SAMPLE_FONT" \
      --style-file "styles/$style.toml" \
      --fit "$SAMPLE_FIT" \
      "$stem" </dev/null; then
      printf '## SAMPLE VERIFICATION FAILED FOR %s; continuing\n' "$stem" >&2
      failures=$((failures + 1))
      continue
    fi
    printf 'verified:  output/validation/%s.lossless.verification.json\n' "$stem" >&2
  fi
done < <(print_plan)

encode_preview_mp4() {
  local hevc="$1" preview="$2"
  ffmpeg -hide_banner -loglevel error -y \
    -i "$hevc" \
    -an -c:v libx264 -preset medium -crf 23 -pix_fmt yuv420p \
    -movflags +faststart \
    "$preview"
}

# Animated WebP renders inline on GitHub README pages (unlike <video> or MP4
# links) with autoplay and looping, so the committed README loops come from the
# same representative set as the MP4 previews.
encode_webp_loop() {
  local hevc="$1" webp="$2"
  local width height scale
  read -r width height < <(ffprobe -v error -select_streams v:0 \
    -show_entries stream=width,height -of csv=p=0:s=x "$hevc" | tr 'x' ' ')
  if (( width >= height )); then
    scale="scale=480:-2"
  else
    scale="scale=-2:560"
  fi
  ffmpeg -hide_banner -loglevel error -y \
    -i "$hevc" \
    -vf "fps=8,$scale" \
    -an -c:v libwebp -lossless 0 -q:v 55 -loop 0 \
    "$webp"
}

if $print_previews || $print_webp; then
  for stem in "${preview_cases[@]}"; do
    style="${staged[$stem]:-}"
    [[ -n "$style" ]] || { printf 'preview case not part of the plan: %s\n' "$stem" >&2; exit 1; }
    hevc="$PF_SAMPLE_DIR/$style/$stem.hevc.mkv"
    if $print_previews; then
      preview="$PF_SAMPLE_DIR/$stem.$style.preview.mp4"
      encode_preview_mp4 "$hevc" "$preview"
      printf '%s\n' "$preview"
    fi
    if $print_webp; then
      mkdir -p docs/media
      webp="docs/media/$stem.$style.webp"
      encode_webp_loop "$hevc" "$webp"
      printf '%s\n' "$webp"
    fi
  done
fi

printf 'sample batch complete: %d videos under %s\n' "${#staged[@]}" "$PF_SAMPLE_DIR" >&2
if (( failures > 0 )); then
  pf_die "sample batch incomplete: $failures asset(s) failed rendering or verification"
fi
