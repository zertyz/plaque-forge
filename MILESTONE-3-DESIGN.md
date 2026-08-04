# Milestone 3 implementation contract

## Consolidated release

Milestone 3 is delivered as one public release, version 0.3.0. The internal work order is implementation sequencing, not three user-visible releases.

## Corrected source invariant

The production pipeline always supplies a title-free plaque. Therefore:

> Every decoded source pixel remains unchanged unless it receives new text or must be restored above that text as a foreground occluder.

There is no old-title detection, erasure, inpainting, plaque replacement, or background reconstruction.

## Tracking interpretation

“Reference” has two simultaneous meanings:

- a permanent root reference that prevents cumulative drift;
- a mutable adaptive reference that follows large changes in scale and appearance.

Every frame is compared with both when possible. A credible root estimate anchors the global trajectory; the adaptive estimate rescues frames for which the root has become visually remote.

Movement inertia is implemented after robust estimation as zero-phase temporal regularization of all four plaque corners. It incorporates neighboring frames without introducing forward-only lag. Because all corners are regularized, sudden resizes and perspective impulses are suppressed along with center-position jumps.

The local PlaqueLock stage then minimizes stable border/decoration differences in canonical space with subpixel translation and scale correction.

## Typography behavior

All text-rendering flags are optional and have documented defaults. `--fit maximize` is the default.

A layout is accepted only when all glyphs, stroke and glow fit the actual content mask. Impossible layouts produce an error with remedies. Empty output is never considered success.

## Verification and recovery

Verification is a release gate and a diagnostic system. Each failing score emits concrete recovery commands or parameter classes. The report identifies worst frames and never returns only a scalar with no way forward.

## Definition of done

- source plaque/background are not repainted;
- tracking is visually locked for the supported class;
- the largest safe typography is selected by default;
- errors contain stage, path/frame and cause chain;
- progress is visible during expensive stages;
- verification is aggressive and prescriptive;
- analysis is cached and transactionally committed.
