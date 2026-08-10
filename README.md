# Plaque Forge 0.3.0

Plaque Forge analyzes a selected moving planar title plaque, caches its motion and masks, renders custom typography, and verifies the result.

See [`PROJECT-OBJECTIVE.md`](PROJECT-OBJECTIVE.md) for the artistic objective,
supported source class, and decision criteria. Numerical verification does not
replace full-size visual review.

## Source contract

Version 0.3 deliberately adopts the production pipeline agreed during development:

> The input plaque cavity must already be free of title text.

Plaque Forge **does not clear, repaint, inpaint, blur, or reconstruct the plaque or background**. Rendering changes only the new text pixels. When an object crosses in front of the plaque, original source pixels are restored through the detected occluder mask.

This contract removes the most destructive and least general part of earlier versions.

## Commands

```text
init      detect plaque candidates and create an editable source metadata sidecar
analyze   inspect the video once and create a reusable .titlepack
export-track  export generated motion as a commented human-owned track
render    render a title using a cached title-pack
verify    score tracking, scene preservation, typography, occlusion and loop continuity
replace   analyze when needed, render, then verify
```

Analysis is automatic unless a sidecar or command-line input constrains it.
`--plaque-hint` identifies the subject; it is not a fixed track.

## Human metadata

Create the portable sidecar for a source video:

```bash
./target/release/plaque-forge init \
  --input video.mp4 \
  --diagnostics video-plaque-diagnostics/
```

This creates `video.plaque.toml` with the best automatic `reference_frame` and
`bounds` proposal as active values. Distinct alternatives and editing guidance
remain comments. The sidecar supports multiple named plaques, segmentation
prompts, human motion tracks, and future dense layer artifacts. Plaque Forge
refuses to replace it unless `--force` is supplied.

To turn generated all-frame motion into an editable track:

```bash
./target/release/plaque-forge analyze \
  --input video.mp4 \
  --output video.titlepack

./target/release/plaque-forge export-track \
  --analysis video.titlepack \
  --output video.main.track.toml
```

Exported samples are unlocked guidance by default. `export-track --locked`
marks every frame authoritative and should be used only for reviewed motion.
The plaque id is copied from the title-pack; `--plaque` is only needed to
override it or name a pack that was analyzed without metadata.
Fonts are required by `render` and `replace`, not by these metadata commands.

See [`METADATA.md`](METADATA.md) for the schema, ownership rules, precedence,
track export, and the boundary between implemented inputs and future layers.

## Build on CachyOS / Arch Linux

```bash
sudo pacman -S --needed rustup clang opencv ffmpeg pkgconf fontconfig
rustup default stable
rustup update stable

./scripts/check.sh
cargo build --release
./target/release/plaque-forge --version
```

The crate targets Rust 1.89+, OpenCV 5 through the `opencv` crate, FFmpeg/FFprobe executables, and `cosmic-text` typography.

`--version` includes a deterministic source fingerprint. Record it with generated
artifacts; a packaged binary with a different fingerprint was built from different
source even when the semantic version is the same.

## One-shot replacement

Use a text file to avoid shell quoting issues:

```bash
printf '%s\n' \
  'Text, custom here we are!' \
  > /tmp/title.txt

FONT="$(fc-match -f '%{file}\n' 'DejaVu Sans' | head -n1)"

./target/release/plaque-forge replace \
  --input /path/to/text-free-plaque.mp4 \
  --output /tmp/custom-title.mkv \
  --analysis /tmp/video.titlepack \
  --text-file /tmp/title.txt \
  --font "$FONT" \
  --plaque-hint 130,160,458,268 \
  --diagnostics /tmp/video-diagnostics
```

The default output is lossless FFV1 in Matroska. This is important because `verify` expects pixels outside the text to remain effectively unchanged.

## Cached workflow

Analysis is the expensive stage:

```bash
./target/release/plaque-forge analyze \
  --input text-free-plaque.mp4 \
  --output video.titlepack \
  --plaque-hint 130,160,458,268 \
  --diagnostics diagnostics/
```

Render any number of titles from that cache:

```bash
./target/release/plaque-forge render \
  --analysis video.titlepack \
  --text-file title-a.txt \
  --font "$FONT" \
  --output title-a.mkv

./target/release/plaque-forge render \
  --analysis video.titlepack \
  --text-file title-b.txt \
  --font "$FONT" \
  --output title-b.mkv
```

Force reanalysis when you want to discard a compatible cache:

```bash
./target/release/plaque-forge replace ... --reanalyze
```

A title-pack is reused only when its complete version-4 manifest and required
assets exist, its analyzer build matches the running binary, and its source
SHA-256 matches `replace --input`. `replace` automatically reanalyzes an
incompatible cache. Direct `render` refuses one with a missing, unknown, or
different analyzer build instead of silently using stale motion. Human metadata
and motion-track hashes, explicit plaque bounds, and legacy CSV contents are also
part of cache identity. Other analysis-setting changes require `--reanalyze`.

## Tracking model

Plaque Forge uses one composite tracker rather than exposing competing algorithms:

```text
fixed root reference, limiting cumulative drift
       +
adaptive feature reference, surviving appearance change
       +
plaque-border feature mask, following the plaque rather than the scene
       +
independent plaque-outline geometry, constraining feature drift
       +
plaque-local structural lock in canonical space
       +
all-frame zero-phase smoothing over all four corners
```

Every frame is decoded and measured with SIFT/RANSAC against plaque-border
features from both:

1. the original root frame;
2. the current adaptive reference frame.

A credible root estimate is preferred when it is not materially worse. The
adaptive estimate becomes the fallback when scale or appearance changes weaken
the root match. When the plaque outline is visible, an independent contour
estimate prevents feature matches from expanding or drifting off the plaque.
Scene/background motion is not used as a proxy for plaque motion.

After impulse rejection and plaque-local structural refinement, zero-phase filters
regularize every measured corner trajectory. Human TOML motion tracks can mix
automatic guides with reviewed locked constraints or replace every frame
authoritatively.

Useful analysis options:

```text
--anchor-interval <frames>       default 24
--tracking-inertia <0..0.98>     default 0.35
--local-refinement-radius <px>   default 12
--motion-model adaptive|similarity|affine|projective
--loop-closure auto|on|off
--metadata <path>                explicit source sidecar
--plaque <id>                    plaque selected from the sidecar
--motion-track <path>            human TOML quad track
--track-csv <path>               reviewed sparse quad keyframes
--minimum-analysis-confidence    default 0.70
--allow-low-confidence           diagnostics-only escape hatch
```

`--anchor-interval` controls only when the mutable feature reference is refreshed;
it never skips frame measurements. Lower values refresh more often during rapid
appearance changes. `--tracking-inertia 0.35` is the default; raise it toward
`0.50` for more smoothing, or lower it toward `0.20` only when the title visibly
lags real plaque motion.

`--motion-track` is the human-owned production format documented in
[`METADATA.md`](METADATA.md). The legacy `--track-csv` accepts
`frame,tl_x,tl_y,tr_x,tr_y,br_x,br_y,bl_x,bl_y`. It must cover the complete shot;
intermediate frames are interpolated. This is the supervised production path when
automatic tracking cannot meet the quality contract. Low-confidence automatic
analysis is not committed unless `--allow-low-confidence` is explicitly supplied.
Human-authored visibility is merged after automatic occlusion analysis, and
`--loop-closure on|off|auto` applies to automatic and human tracks alike.

## Typography

All typography controls are optional except `--font` and the title itself.

```text
--fit maximize|balanced|fixed    default maximize
--font-size <px>                 required only for fixed; upper bound otherwise
--max-lines <n>                  default 3
--padding <ratio>                default 0.05
--line-height <ratio>            default 1.16
--stroke-width <ratio>           default 0
--text-color <#RRGGBBAA>         default #EBFFFFFF
--stroke-color <#RRGGBBAA>       default #03181ED2
--glow-color <#RRGGBBAA>         default #69F2FA48
--glow-radius <px>               default 4
--text-align left|center|right
--vertical-align top|center|bottom
--supersampling <1..4>           default 4
```

`maximize` first finds the shaped rectangular limit, then searches the final
supersampled glyph, stroke, and glow layer against the actual irregular content
mask. Explicit newlines are preserved; additional word/glyph wrapping is allowed
up to `--max-lines`.

Plaque Forge never silently produces an empty title. It fails with a corrective message when:

- the text is empty;
- the font lacks a glyph or invokes fallback;
- fixed font size overflows;
- no layout fits even at the minimum size;
- glow or stroke crosses the plaque mask.

For Bash multiline strings, `$'line one\nline two'` is shell syntax, not Plaque Forge syntax. `--text-file` is usually clearer.

## Verification

```bash
./target/release/plaque-forge verify \
  --analysis video.titlepack \
  --rendered custom-title.mkv \
  --original text-free-plaque.mp4 \
  --report custom-title.verification.json \
  --diagnostics diagnostics/
```

The default release gate is aggressive:

| Score | Minimum |
|---|---:|
| Overall | 0.95 |
| Tracking lock | 0.95 |
| Scene integrity | 0.995 |
| Typography fit | 0.98 |
| Typography validity | 1.00 |
| Temporal stability | 0.95 |
| Occlusion restoration | 0.95 |
| Loop seam | 0.98 |

Verification does not only say “failed”. The JSON report contains:

- every failed subscore;
- the worst tracking, trajectory, and scene-integrity frames;
- a concrete remedy for each failure.

`tracking_lock` combines residual structural registration with plaque-edge
alignment, avoiding false drift reports from legitimate animated color or
specular changes. `temporal_stability` measures high-frequency four-corner and
visibility trajectory error. For loops, the seam score measures circular motion
and visibility continuity. Pixel-domain title-effect differences remain in the
report as diagnostics, but are not scored because a translucent title over an
animated plaque legitimately changes RGB values.

During verification, `structural lock 0..1` is the per-frame registration score;
it is not a percentage of frames tracked. `maximum_trajectory_residual_pixels`
is the largest second-order corner/visibility motion error and
`worst_trajectory_frame` locates it. `loop_seam` is based on circular trajectory
curvature; `loop_seam_mean_error` is a separate pixel-domain diagnostic, so the
two values are not expected to have the same numerical scale.

For a dense locked TOML track, geometric lock is an asserted, reviewed input and
the report records `tracking_lock_basis = authoritative-human-quad-track`. Legacy
supervised CSV tracks retain `authoritative-supervised-quad-track`. The verifier
continues to reject high-frequency trajectory errors, broken loop continuity,
bad masks, or scene damage, and still reports structural registration residuals.
This is a trust boundary: labeling an unreviewed automatic track as supervised
does not make it production quality.

For automatic tracking failures, the report gives the current setting, exact
worst frame and timestamp, measured correction, analyzed rectangle, and the path
to `verification-worst-tracking-frame.png`. `--plaque-hint x,y,width,height`
means a source-pixel rectangle around the full plaque; use it only when
`candidate.png` shows the yellow rectangle on the wrong object.

## Progress and diagnostics

Long-running stages report frame count, percentage, ETA and current tracking statistics to stderr:

```text
[3/7] Adaptive scene tracking 137/240 (57.1%), ETA 00:18, root, inliers 0.91, error 0.48px
[5/7] Plaque structural lock 137/240 (57.1%), ETA 00:05, residual 0.42px
[3/3] Composite and encode 181/240 (75.4%), ETA 00:03
```

Control it with:

```text
--progress auto|always|never
--progress-interval-ms 500
```

Analysis is transactional. It is created under a `.partial-<pid>` directory and renamed only when complete. Failed analysis retains the partial diagnostics and reports their exact path.

`candidate-ranking.json` records the leading automatic candidates.
`candidate.png` shows the selected rectangle and distinct alternatives,
`tracking-contact-sheet.jpg` samples the recovered track, and verifier
diagnostics include the exact worst frame.

## Title-pack contents

```text
manifest.toml
motion.json
content-mask.png
structural-mask.png
structural-template.png
analysis-summary.json
occluder/                 optional per-frame masks
diagnostics/
```

No blank-plaque PNG or reconstructed background is produced.

## Production correction path

Automatic analysis is the fast path, not the definition of artistic acceptance.
An artist or tracking package can approve TOML plaque quadrilaterals once, and
every title then reuses the same immutable track. The title-pack also stores explicit
per-frame plaque visibility and full-frame occluder masks, so rendering is already
separated from inference and those artifacts can be reviewed.

Lossless RGBA foreground mattes and a reusable material/style profile are the next
production interfaces. They are not implemented yet. Today the art direction
surface is a static text/stroke/glow style; it cannot reproduce an arbitrary
animated plaque material by itself. See
[`PROJECT-OBJECTIVE.md`](PROJECT-OBJECTIVE.md) for that boundary.

## Intended scope

The first target is intentionally precise:

> A video with one dominant planar text-free plaque, smooth camera/plaque motion, and zero or more foreground objects crossing the plaque.

This covers the generated title loops that motivated the project. It is not a general semantic video editor.

More precisely, the first validated class requires a plaque whose motion can be
represented by one quadrilateral per frame, enough stable visual structure for
tracking, and occlusion/disappearance that can be represented by an alpha
visibility mask. It also assumes one continuous, fixed-dimension,
constant-frame-rate shot. Unsupported cases must fail or request preprocessing,
supervised keyframes, and mattes rather than silently producing a low-quality render. See
[`PROJECT-OBJECTIVE.md`](PROJECT-OBJECTIVE.md) for the complete boundary and the
broader destination.

## Validation boundary

The local quality gate is:

```bash
./scripts/check.sh
```

For the complete source-to-render reference gate:

```bash
REFERENCE_VIDEO=/path/to/moving-holographic-plaque.mp4 \
REFERENCE_FONT=/path/to/font.ttf \
scripts/validate_reference.sh
```

Set `REFERENCE_METADATA=/path/to/video.plaque.toml` to exercise sidecar geometry
and `REFERENCE_MOTION_TRACK=/path/to/video.main.track.toml` for the TOML
production path. `REFERENCE_TRACK=/path/to/reviewed-plaque-track.csv` retains the
legacy CSV compatibility gate. Omitting all three deliberately tests the
automatic path.

This produces a fresh title-pack, lossless render, verification report, exact
packet-count check, and `render-contact-sheet.png`. The contact sheet remains a
required human review artifact under the quality contract in
[`PROJECT-OBJECTIVE.md`](PROJECT-OBJECTIVE.md).
