# Human refinement v2 plan

Status: design target, not implemented.

## Problem

The current refinement files mix two very different things:

1. **human intent/corrections**, such as which plaque or foreground object matters;
2. **dense generated state**, such as long per-frame quadrilateral tracks.

The first belongs in an editable refinement manifest. The second is an artifact that a human may inspect or replace, but should not be expected to author.

The goal is not to invent prettier machine data. The goal is to reduce the amount of machine data a person has to touch.

## Proposed split

```text
assets/refinements/<scene>/
  refinement.toml          sparse human intent and corrections
  artifacts/               generated/reviewable data
    motion.json
    masks/
    previews/
```

`refinement.toml` should remain short enough to understand in one screen for an ordinary scene.

## Human-authored concepts

A refinement should primarily say:

- which plaque is intended when automatic selection is ambiguous;
- which frame is a good reference frame;
- sparse motion anchors only where automatic tracking is wrong;
- foreground objects that must pass over the title;
- sparse positive/negative segmentation prompts when automatic masks need help;
- optional quality intent such as “this object boundary matters more than background foliage.”

Dense interpolated tracks, propagated masks, confidence series, and backend-specific tensors belong under `artifacts/` or the analysis cache.

## Coordinate ergonomics

Raw pixel coordinates are sometimes necessary, but they should not be the primary human interface.

A future review/refine UI should let the user click on a diagnostic frame and write normalized coordinates into the manifest. The text form can still store normalized points for reproducibility:

```toml
[[foreground]]
name = "lizard"

[[foreground.prompt]]
frame = 87
include = [[0.63, 0.54], [0.71, 0.58]]
exclude = [[0.79, 0.61]]
```

The important improvement is that the person chooses points visually; they do not calculate them manually.

## Motion refinement

Dense `frame -> quad` files should become generated artifacts. Human motion correction should be sparse:

```toml
[[motion.anchor]]
frame = 0
quad = [[0.21, 0.31], [0.78, 0.30], [0.80, 0.67], [0.19, 0.68]]

[[motion.anchor]]
frame = 143
quad = [[...], [...], [...], [...]]
```

The analyzer/tracker interpolates or re-solves between anchors. An explicit `authoritative = true` escape hatch can remain for scenes where a complete external track really is required.

## Backend isolation

Human refinement must describe scene intent, not Python model internals. Backend/model/device selection belongs to segmentation configuration or a generated provenance record unless the user explicitly chooses a backend for reproducibility.

For example, “foreground object named lizard with these prompts” is human intent. “SAM2 checkpoint X with tensor shape Y” is implementation/provenance data.

## Migration strategy

1. Keep the current schema readable indefinitely.
2. Introduce a new schema version that supports sparse normalized anchors/prompts.
3. Convert old dense motion files into `artifacts/` references without changing their meaning.
4. Add a `plaque-forge refine migrate` command rather than silently rewriting user files.
5. Add a visual click-based refinement helper after the HTML review workflow is stable.
