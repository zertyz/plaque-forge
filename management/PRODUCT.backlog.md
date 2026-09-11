# Product backlog

Unless overridden: WI-011–WI-022 are Planning since 2026-09-10; later IDs since 2026-09-11. Owner: Unassigned. [Lifecycle and evidence](GOVERNANCE.md#g-007-lifecycle) apply. The [asset assessment](../docs/ASSET_VIDEOS_HUMAN_DEFINITIONS.md#suggested-follow-up-priorities) supplies the earlier improvement suggestions.

## WI-011 Specify capability and assistance cases

Requires: [P-001](PRODUCT.md#p-001-generalization), [P-002](PRODUCT.md#p-002-human-assistance), [P-009](PRODUCT.md#p-009-media-fidelity-and-scope), [P-011](PRODUCT.md#p-011-honest-confidence-and-failure).
Depends: WI-001.

Translate the resolved broad scope into concrete capability/interaction cases, assistance inputs, and failure examples. Include removal, HDR/wide-gamut, multiple surfaces, cuts, deformation, and variable media timing. Describe observable behavior without an asset allowlist or arbitrary ceilings.
Done: cases can be assessed before implementation. Current gaps remain planned work; deferral does not silently remove a requirement.

## WI-012 Establish the human-accepted baseline

Requires: [P-010](PRODUCT.md#p-010-accepted-quality).
Depends: WI-010, WI-032, WI-033.

Review all 19 examples using the complete protocol in [A-007](ACCEPTANCE.md#a-007-establishment-and-promotion). Capture resolved scenes, lossless renders, assistance provenance/effort, quality dimensions, and imperfections. Keep the 8 recorded contracts binding until approved successors replace them.
Done: human-approved packages and explicit unresolved cases; no automatic promotion of caches, invented history, or blanket certification.

## WI-013 Accept the two missing capability contracts

Requires: [P-005](PRODUCT.md#p-005-motion-and-visibility), [P-006](PRODUCT.md#p-006-foreground-and-material), [P-010](PRODUCT.md#p-010-accepted-quality).
Depends: WI-012, WI-002.

Review multi-layer foreground/shadow composition and background motion independent of the title surface. Existing starting examples: the landscape vines/lizard scene and the portrait lonely-ogre hologram. Capture the accepted behavior as independent evidence.
Done: approved contracts fail deliberate ordering/motion errors. Example choice is evidence selection, not a new production special case.

## WI-014 Strengthen incomplete visual witnesses

Requires: [P-006](PRODUCT.md#p-006-foreground-and-material), [P-010](PRODUCT.md#p-010-accepted-quality).
Depends: WI-012, WI-002.

Review foreground protection missing from the landscape wooden-plaque contract, complementary visible-title evidence where needed, and temporal gaps between selected witnesses. Preserve the existing web-thread/web-gap distinction.
Done: accepted title visibility and foreground preservation both constrain the relevant interactions; empty title, opaque gaps, and lost foreground cannot pass the strengthened cases.

## WI-015 Reconcile capability coverage inventories

Requires: [P-010](PRODUCT.md#p-010-accepted-quality), [O-005](OPERATIONS.md#o-005-required-validation-coverage).
Depends: WI-012, WI-002.

Reconcile the main and segmentation inventories. Assess whether the chains contract accepts segmentation parallax; review unrepresented human fine-detail/open-vocabulary capabilities against actual product scope. Distinguish represented, accepted, and continuously executed coverage.
Done: every advertised capability has an explicit evidence status; links do not imply broader acceptance than their witnesses support; no ledger minimum is lowered to hide a gap.

## WI-016 Resolve imported layer timing and layout

Requires: [P-003](PRODUCT.md#p-003-scene-meaning).
Depends: WI-002.

Implement conflict failure for effective `active_frames` and `affects_layout` declarations across scenes and imported artifacts. Exercise conflicting flags and nonzero pixels outside declared windows. Zero padding outside an active interval is not itself conflicting content.
Done: changing an authoritative setting changes observable behavior or produces the specified error; canonical/source-pixel and single/sequence cases follow one clear contract; accepted examples remain valid.

## WI-017 Preserve annotation and review provenance

Requires: [P-002](PRODUCT.md#p-002-human-assistance), [O-002](OPERATIONS.md#o-002-identity-and-provenance).
Depends: WI-010.

Define promotion of generated or manually corrected masks/trajectories into reviewed scene inputs. Record source identity, creation method, correction reason, reviewer, and accepted evidence. Audit the four dense locked trajectories and historical imported masks.
Done: future promotion is attributable and reproducible where claimed; historical unknowns are recorded honestly. Do not delete valid authored inputs merely because their original generator is unavailable.

## WI-018 Review the moving hologram flicker

Requires: [P-005](PRODUCT.md#p-005-motion-and-visibility), [P-010](PRODUCT.md#p-010-accepted-quality).
Depends: WI-012, WI-002.

Review source/render frames around 122–127 and the low visibility on frames 124–125. Decide whether the current two-frame transition matches intended appearance; retain or revise it deliberately. Add a visible temporal witness if warranted.
Done: human decision and supporting artifact identity; any trajectory/contract amendment follows the protected path. Suspicious numbers alone are not a defect.

## WI-019 Review templated foreground prompts

Requires: [P-006](PRODUCT.md#p-006-foreground-and-material), [P-002](PRODUCT.md#p-002-human-assistance).
Depends: WI-012.

Review the landscape spider's repeated box/point pattern against anatomy and motion, including intervals between prompts. Compare prompt agreement with independent silhouettes and final compositing evidence.
Done: supported/rejected findings with visual evidence and a bounded correction only where needed. More prompts or dense supplied masks cannot silently count as an automation improvement.

## WI-020 Build independent capability examples

Requires: [P-001](PRODUCT.md#p-001-generalization).
Depends: WI-011, WI-012.

Select unfamiliar sources and capability combinations beyond the dominant upper-title compositions. Distinguish development examples, regression examples, and evaluation material withheld from tuning. Define who reviews and refreshes the evaluation set.
Done: approved source/use rights, capability coverage, assistance labels, independent acceptance, and a protocol that exposes regressions and tuning leakage. Do not claim statistical coverage from an arbitrary example count.

## WI-021 Clarify expressive scene fields

Requires: [P-003](PRODUCT.md#p-003-scene-meaning), [P-004](PRODUCT.md#p-004-surface-and-writing-region).
Depends: WI-011.

Specify `in_front_of`, canonical `affects_tracking`, and writable/material masks against the approved multi-surface/deformation requirements. Identify unsupported current semantics and dependencies for WI-038/WI-039.
Done: schema meanings and observable specifications agree; reserved names are not presented as implemented capabilities.

## WI-022 Broaden typography and style acceptance

Requires: [P-007](PRODUCT.md#p-007-typography-and-effects), [P-008](PRODUCT.md#p-008-reusable-analysis).
Depends: WI-011, WI-012, WI-002.

Evaluate titles of different lengths, deliberate line breaks, glyph coverage, and representative effects across geometry/depth cases. Define readability constraints beyond the current font-size ceilings and fixed titles.
Done: agreed typography constraints survive variation; title/style changes reuse analysis; deliberate clipping, missing glyphs, and illegible fitting fail their relevant checks.

## WI-032 Capture and replay complete homologations

Requires: [A-001](ACCEPTANCE.md#a-001-approved-baseline), [A-002](ACCEPTANCE.md#a-002-frozen-replay), [P-008](PRODUCT.md#p-008-reusable-analysis).
Depends: WI-002, WI-011.

Define the versioned source/input/resolved-scene/render/approval package. Capture all data needed for analysis-independent rendering; preserve approved lossless frames and original versions. Specify storage, compatibility, and lossless migration before implementing the capture/replay path.
Done: current rendering consumes a frozen scene with analysis disabled; changed renderer behavior is still observable; reset cannot remove acceptance data. Hashes and deliberate missing-data cases validate the package.

## WI-034 Evaluate fresh reconstruction

Requires: [A-003](ACCEPTANCE.md#a-003-reconstruction-and-remapping), [E-005](ENGINEERING.md#e-005-generalization-evidence).
Depends: WI-006, WI-012, WI-033.

Run current analysis from source in separate automatic and unchanged-assistance lanes; render and compare with approved targets. Prevent reads of frozen answers and previous per-video analysis results in fresh-analysis stages.
Done: reports retain both results and their provenance; supplied answers cannot inflate automatic performance; deliberately stale/cross-lane inputs fail.

## WI-035 Compare correction effort and propose remapping

Requires: [A-003](ACCEPTANCE.md#a-003-reconstruction-and-remapping), [A-005](ACCEPTANCE.md#a-005-human-correction-effort), [A-007](ACCEPTANCE.md#a-007-establishment-and-promotion).
Depends: WI-012, WI-034.

Record correction types, scope, measured effort, and machine contribution; compare against current and earlier approved burdens. Generate candidate remappings with BEFORE/AFTER renders and dimension-specific findings.
Done: substantially greater effort is exposed as regression; original failures remain visible; a human can promote a comparable/lower-effort input version without overwriting the old record or silently changing the visual target.

## WI-036 Support HDR and wide-gamut fidelity

Requires: [P-009](PRODUCT.md#p-009-media-fidelity-and-scope), [A-004](ACCEPTANCE.md#a-004-quality-and-fidelity).
Depends: WI-011, WI-023, WI-033.

Specify and implement color-aware decoding, compositing, reference comparison, and delivery for HDR/wide-gamut inputs. Define explicit requested output transformations and metadata handling.
Done: independent color/range/gradient/alpha evidence detects clipping, unintended tone mapping, banding, and color drift; accepted SDR behavior remains protected.

## WI-037 Remove and replace existing titles

Requires: [P-013](PRODUCT.md#p-013-existing-title-removal).
Depends: WI-011, WI-023, WI-024.

Evaluate and implement title identification/removal with temporally consistent reconstruction, followed by normal insertion. Keep removal assistance and uncertainty visible.
Done: independent sources expose residual lettering, material damage, foreground loss, and temporal artifacts; accepted text-free insertion remains intact.

## WI-038 Support multiple interacting title surfaces

Requires: [P-003](PRODUCT.md#p-003-scene-meaning), [P-004](PRODUCT.md#p-004-surface-and-writing-region), [P-006](PRODUCT.md#p-006-foreground-and-material).
Depends: WI-021, WI-023.

Implement independent surface identity, tracking/layout, and visible ordering for multiple titles and crossing surfaces/objects.
Done: independent overlap, motion, and visibility cases satisfy approved ordering without special-casing their sources; single-surface acceptance still passes.

## WI-039 Track and render deforming surfaces

Requires: [P-004](PRODUCT.md#p-004-surface-and-writing-region), [P-005](PRODUCT.md#p-005-motion-and-visibility), [P-011](PRODUCT.md#p-011-honest-confidence-and-failure).
Depends: WI-021, WI-023, WI-024, WI-033.

Implement geometry and validation that represent actual deformation of the title-bearing material. Distinguish changing visible support from deformation of the text mesh; keep failure evidence explicit.
Done: independent deforming cases preserve attachment/readability through time; unsupported observations fail honestly; planar baselines do not regress.

## WI-040 Handle shot changes and reacquisition

Requires: [P-005](PRODUCT.md#p-005-motion-and-visibility), [P-009](PRODUCT.md#p-009-media-fidelity-and-scope).
Depends: WI-011, WI-023, WI-033.

Specify title/surface continuity at cuts and implement detection, identity handling, visibility, and reacquisition without carrying unrelated geometry across shots.
Done: abrupt cuts, absent/reappearing surfaces, and multiple shot sequences have independently reviewed outcomes; uncertain identity is reported rather than silently guessed.

## WI-041 Process variable and large media without arbitrary ceilings

Requires: [P-009](PRODUCT.md#p-009-media-fidelity-and-scope), [O-006](OPERATIONS.md#o-006-reproducible-execution).
Depends: WI-011, WI-023, WI-027.

Assess timing representation, memory/storage scaling, long inputs, frame counts, and variable frame rates. Implement bounded working resources and faithful timeline processing without fixed product ceilings or hidden sampling/downscaling.
Done: measured increasing workloads preserve every required frame and audio timing; real resource failures are actionable; rejecting accepted inputs remains a regression.
