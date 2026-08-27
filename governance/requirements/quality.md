# Quality requirements

> **Status: NORMATIVE**
> **Requirement prefix: `REQ-QUAL`**
> **Primary policy:** `BUS-QUAL-*`

### REQ-QUAL-001 — Surface lock
Inserted title/plaque content that belongs to a physical scene surface shall follow that surface's observed/projective
motion rather than becoming an unintended screen-fixed overlay.

### REQ-QUAL-002 — Writable-region constraint
Typography shall remain inside the declared/reconstructed writable region. Tracking support may use surrounding stable
material, but that support must not silently expand the area allowed to receive text.

### REQ-QUAL-003 — Foreground remains foreground
A scene object/layer accepted as being in front of the writing surface shall remain visually in front of inserted title
pixels where they overlap. Background/negative-depth evidence shall not erase the title.

### REQ-QUAL-004 — Porous and soft foregrounds preserve complementary visibility
For porous or translucent foregrounds, rendering shall preserve both the foreground material that must remain source-like
and the gaps/transparency through which title content must remain visible. Treating a porous foreground as one opaque sheet
is not acceptable merely because it protects foreground pixels.

### REQ-QUAL-005 — Temporal stability
Accepted output shall not introduce visually significant plaque/title drift, temporal blinking, hard matte edges,
foreground inversion, or composition instability beyond the tolerances defined by the relevant verification/homologation
evidence.

### REQ-QUAL-006 — Color/alpha semantics
Raster resampling and compositing shall preserve the project's alpha semantics, including soft alpha. Declared SDR
color/rotation metadata shall be preserved as supported; unsupported HDR/BT.2020 cases shall fail explicitly rather than be
silently misinterpreted.

### REQ-QUAL-007 — Exhaustive verification remains exhaustive
Checks defined as full-frame/exhaustive verification shall examine every relevant frame and pixel. Performance work shall
not replace that contract with sampling without an explicit requirement/policy review.

### REQ-QUAL-008 — Canonical quality mode is explicit
The default canonical ML/analysis quality path shall use its declared model/precision semantics. Preview/balanced modes may
trade quality for speed only through explicit user selection; hardware device choice alone shall not redefine numeric
precision or quality policy.

### REQ-QUAL-009 — Human acceptance for artistic composition changes
Establishing or deliberately changing artistic composition accepted as a regression baseline requires human visual review.
Automated metrics may provide gates/evidence but shall not create human acceptance by themselves.
