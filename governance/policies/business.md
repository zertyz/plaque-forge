# Business / Product & Quality policy

> **Status: NORMATIVE**
> **Owner: Maintainer**
> **Rule prefix: `BUS`**

This policy owns product mission and user-visible quality constraints. Concrete observable behavior is specified in
`governance/requirements/`.

## Product intent

### BUS-MIS-001 — Mission consistency
Implemented functionality must remain consistent with Plaque Forge's mission: adding/replacing artistic title content on
video writing surfaces while preserving the scene relationships that make the insertion visually belong there.

### BUS-MIS-002 — Automation with bounded human refinement
The product should perform as much analysis and placement automatically as practical, while preserving explicit paths for
small, reviewable human corrections when automatic intent is insufficient.

### BUS-MIS-003 — Analyze once, render many
Reusable scene understanding is a product capability. Rendering new titles/styles should not require repeating expensive
analysis when the analysis inputs and semantics remain valid.

## Quality and accepted behavior

### BUS-QUAL-001 — Observable quality is a product requirement
Camera/surface lock, geometry, readable typography, foreground depth, alpha/compositing behavior, and temporal stability
are observable product behavior, not implementation details.

### BUS-QUAL-002 — No silent quality trade
Performance, portability, convenience, hardware fallback, or CI cost must not silently weaken canonical output quality or
acceptance semantics. Lower-quality/preview behavior is allowed only when explicitly selected and clearly identified.

### BUS-QUAL-003 — Accepted behavior survives refactoring
Human-homologated observable behavior must be preserved across internal implementation changes unless a deliberate human
review changes the corresponding requirement and acceptance authority.

### BUS-QUAL-004 — Human artistic acceptance remains human
Automated scores and generated diagnostics may reject measurable regressions and assist review, but they do not establish
or replace human artistic acceptance of a new composition/behavior.

### BUS-QUAL-005 — Visible depth semantics
Foreground/depth correctness must be expressed in visible compositing terms. The existence of a mask, layer, model output,
or diagnostic record is not sufficient evidence that the foreground actually remains visually in front of inserted text.

### BUS-QUAL-006 — Quality evidence must be independent
Generated analysis or decision traces must not certify themselves. Acceptance evidence must be independent of the artifact
whose implementation produced the behavior, or explicitly derived from a human-accepted output.

### BUS-QUAL-007 — Capability-oriented regression protection
Expensive visual regression protection should be organized by materially distinct behavioral capability rather than by
raw asset count. Representative sentinels may be sparse, but missing human acceptance must remain visible review debt.

### BUS-QUAL-008 — Performance changes preserve semantics
An optimization advertised as quality-neutral must preserve the governed output/acceptance semantics and provide evidence
appropriate to the affected behavior. If it intentionally trades quality for speed, that trade must be an explicit product
mode rather than an implicit optimization.
