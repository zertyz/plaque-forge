
# Status update (2026-08-02)

The failure recorded below was reproduced with the pre-existing release binary.
That executable was stale relative to the checked-in source. Current builds carry
a deterministic source fingerprint, and the typography fitter now validates the
final glyph/stroke/glow composition against the actual irregular content mask.
Both `hi` and the larger reference title reach rendering with a fresh build.

The project objective, first validated source class, decision criteria, and the
automatic-plus-reviewed production path now live in
[`PROJECT-OBJECTIVE.md`](PROJECT-OBJECTIVE.md). The runnable workflow and its
current artistic limitation are documented in [`README.md`](README.md): 0.3.0 has
reviewed sparse quadrilateral tracks and reusable masks/visibility in its
title-pack, but external matte/visibility import and reusable animated material
profiles remain follow-up work. The historical notes below are retained as the
original failure report.

The latest concrete reference results are:

- the moving reference now completes without `--plaque-hint` in about 41 seconds,
  compared with about 8 minutes 45 seconds before the performance work;
- the complete static reference now takes about 37 seconds instead of 1 minute
  56 seconds after bounding dense structural registration; it still verifies at
  `1.000` overall;
- no-hint verification passes at `0.989` overall and `0.958` tracking lock;
  the equivalent hinted run passes at `0.993` and `0.975`. The former
  background-feature tracker produced only `0.472` tracking lock without a hint;
- SIFT anchors now come from the plaque border. Background motion is no longer
  treated as plaque motion;
- every frame is measured and contributes to the final zero-phase trajectory;
  `--anchor-interval` only controls adaptive feature-reference refreshes;
- automatic selection prefers the full dominant plaque and rejects structureless
  false targets before rendering;
- the rusty parallax reference now detects the full `322,34,651,145` plaque
  without a hint, tracks through the foreground hook, and passes at `0.991`
  overall with `0.965` tracking lock;
- a provisional, mechanically smoothed 21-keyframe CSV validates the supervised
  workflow at `0.9996` overall, with exact scene preservation, valid typography,
  restored occlusion, stable trajectory, and a passing loop seam;
- the original `hi --fit balanced` case renders at 151.83 px on one line without
  clipping;
- these are technical results, not artistic acceptance: the current static
  fill/stroke/glow style remains visibly less integrated than the plaque material.

Current `replace` automatically reanalyzes unknown or different-build packs;
direct `render` rejects them. `--reanalyze` forces a fresh analysis.

Current validation artifacts are under `/tmp/pf-v04-*`.

# Historical failure report

# The current program appears not to work -- whatever text I put and suggested parameter changes
INPUT="$HOME/tmp/video-titles/claude-46-video-title-forge/assets/moving-holographic-plaque.mp4"; FONT="$(fc-match -f '%{file}\n' 'DejaVu Sans' | head -n1)"; rm -rf /tmp/ogre.titlepack /tmp/ogre-diagnostics; ./target/release/plaque-forge replace   --input "$INPUT"   --output /tmp/ogre-custom.mkv   --analysis /tmp/ogre.titlepack   --text "hi"   --font "$FONT"   --plaque-hint 130,160,458,268   --diagnostics /tmp/ogre-diagnostics   --progress always --fit balanced

The results are always the same:
[1/3] Open title-pack and validate source
[1/3] Open title-pack and validate source done in 00:00, source fingerprint and title-pack are valid
[2/3] Shape and fit typography
error: text cannot fit the irregular plaque mask without clipping; reduce --padding, increase --max-lines, or use a narrower font


# Nomenclature

The current state of things is called "MILESTONE-3".

My experimentations with it are listed in CURRENT_STATE.md


# Milestone 3 paln

File PLAQUE-FORGE-MILESTONE-3-CORRECTED-PLAN.md contains a plan sketch that I could not verify
the quality of, as I couldn't run the project. But it, overall, points to the intended direction.


# Full communications that lead to the current state of the code

For you to understand the context 100%, please see the conversations that lead to the
current state of this POC:

https://chatgpt.com/share/6a6f7557-53b8-83e9-8ab3-79d8f2e26c1e
