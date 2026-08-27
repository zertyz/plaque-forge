# Homologation requirements

> **Status: NORMATIVE**
> **Requirement prefix: `REQ-HOM`**
> **Primary policies:** `BUS-QUAL-*`, `OPS-ART-*`, `GOV-AUTH-*`

### REQ-HOM-001 — Contract represents human-accepted observable behavior
A homologation contract shall describe behavior already visually reviewed and explicitly accepted by a human, and future
internal changes shall preserve that behavior until human authority deliberately changes it.

### REQ-HOM-002 — Contracts prefer semantic invariants over implementation artifacts
Homologation shall protect stable geometry, typography constraints and sparse reviewed visual witnesses where practical,
rather than requiring whole-video byte equality or implementation facts such as a generated mask being non-empty.

### REQ-HOM-003 — Generated analysis is not its own oracle
Generated analysis/cache data shall not be sufficient to establish that the output generated from it is accepted.
Homologation evidence must be independently reviewed or derived from an explicitly accepted render.

### REQ-HOM-004 — Source-preservation witnesses protect foreground material
Pixels selected by a reviewed source-preservation witness shall remain sufficiently source-like in the rendered artifact,
according to the accepted tolerance, regardless of which segmentation/compositing implementation produced the result.

### REQ-HOM-005 — Title-visibility witnesses protect open regions
Pixels selected by a reviewed title-visibility witness shall remain sufficiently changed from the source so foreground
restoration cannot pass by turning porous material/gaps into an opaque erasure of the title.

### REQ-HOM-006 — Evidence is bound to exact artifact/provenance
A homologation/verification report shall identify the exact rendered bytes and relevant manifest/provenance it certifies.
Replacing the render or provenance shall make evidence for the prior artifact stale.

### REQ-HOM-007 — Contract weakening is not a regression fix
A failing homologation contract shall not be weakened merely to make a regression green. A deliberate accepted behavior
change requires human review of the requirement and replacement acceptance evidence.

### REQ-HOM-008 — Capability matrix exposes coverage and review debt
Homologation coverage shall be organized by behavioral capability with representative assets. A capability without a human-
accepted contract shall remain explicit review debt; tooling/CI shall not fabricate a golden contract to improve coverage.

### REQ-HOM-009 — CI sentinels are representative, not exhaustive
The permanent expensive CI homologation set may be deliberately small, provided it represents materially different
geometry/tracking/compositing/aspect/depth behaviors and does not hide unaccepted capabilities.

### REQ-HOM-010 — Decision traces explain but do not certify
Generated decision traces may explain surface selection, tracking, typography and matte choices and shall be provenance-
bound to the relevant render, but they shall not serve as self-certifying acceptance evidence.
