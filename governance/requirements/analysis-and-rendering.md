# Analysis and rendering requirements

> **Status: NORMATIVE**
> **Requirement prefix: `REQ-PIPE`**
> **Primary policies:** `BUS-*`, `OPS-ART-*`, `OPS-DIAG-*`

### REQ-PIPE-001 — Authored intent and generated analysis are separate
Authored scene intent shall remain separate from generated analyzer conclusions, dense trajectories, propagated masks and
quality scores. Regenerating analysis must not silently rewrite human scene intent.

### REQ-PIPE-002 — Complete compatible analysis required for render
Rendering shall require a complete current analysis cache for analysis-dependent scenes and shall not silently launch
expensive analysis as a side effect of render.

### REQ-PIPE-003 — Analysis cache compatibility is explicit
Serialized-schema incompatibility and semantic analyzer incompatibility shall be represented explicitly. A stale cache
shall be regenerated; it shall not be relabelled current without recomputing the affected analysis.

### REQ-PIPE-004 — Renderer changes do not invalidate analysis without cause
A renderer-only/style-only/CLI-only/documentation change shall not invalidate reusable scene analysis when the analysis
schema and semantics are unchanged.

### REQ-PIPE-005 — ML execution plan is sealed by Rust
Rust shall own the selected segmentation strategy/model/precision contract; the Python worker shall execute that plan and
return provenance/evidence rather than independently deciding which candidate is acceptable.

### REQ-PIPE-006 — Device and precision are independent
Selecting CPU/XPU/CUDA/auto shall not silently change the requested numeric precision. If the requested plan cannot be
honored on a fallback device, the worker shall fail explicitly or use an explicitly selected different profile.

### REQ-PIPE-007 — Pure-Rust/no-ML degradation mode
An explicit no-ML analysis mode shall not require Python to be installed merely because optional prompted ML evidence is
missing or incompatible. Valid compatible cached/reviewed evidence may be reused according to its provenance rules.

### REQ-PIPE-008 — Experimental backend selection is explicit
An experimental/gated backend such as SAM 3.1 shall not be selected implicitly by the normal canonical strategy until
measured evidence and human-reviewed output justify changing the acceptance strategy.

### REQ-PIPE-009 — Transactional render bundle
A render publication shall produce its video and associated canonical evidence/manifest members as one recoverable bundle,
with a clear commit marker so interruption cannot publish a mixed old/new state.

### REQ-PIPE-010 — Portable generated identity
Generated manifests shall identify source, analysis inputs, renderer/style/font/encoder information and relevant decisions
without persisting absolute workstation paths.

### REQ-PIPE-011 — Foreground matte semantics remain explicit
Foreground layers shall distinguish semantic-confidence opaque mattes from literal optical transparency so model confidence
is not accidentally rendered as physical translucency, and soft optical alpha is not accidentally hardened.
