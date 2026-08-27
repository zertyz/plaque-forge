# Operations policy

> **Status: NORMATIVE**
> **Owner: Maintainer**
> **Rule prefix: `OPS`**

This policy owns operational behavior: command execution, artifacts, provenance, lifecycle, CI/CD, diagnostics and
controlled destructive actions.

## Artifacts and provenance

### OPS-ART-001 — Reusable analysis is not render scratch space
Rendering must consume a complete compatible analysis cache without rebuilding, deleting, or silently mutating reusable
analysis as a side effect.

### OPS-ART-002 — Transactional publication
Analysis and render outputs should be staged privately, validated, and published transactionally. An interrupted operation
must not replace a previously complete artifact bundle with a partial one.

### OPS-ART-003 — Exact artifact identity
Verification, homologation and diagnostic evidence that claims to describe/certify an artifact must identify the exact
artifact/provenance it refers to. Evidence for previous bytes is stale after replacement.

### OPS-ART-004 — Portable persisted paths
Generated project artifacts must not persist workstation-specific absolute paths. Persist portable identifiers, content
hashes, dimensions, versions and other reproducible provenance instead.

### OPS-ART-005 — Semantic cache invalidation
Cache format/semantic identities must be changed when their governed schema or analysis semantics become incompatible.
Renderer-only, documentation-only, CLI-only, or unrelated refactors must not invalidate reusable analysis without cause.
Stale semantic output must be rebuilt rather than relabelled as current.

## Destructive operations and lifecycle

### OPS-SAFE-001 — Explicit destructive intent
Existing reusable analysis/segmentation data must not be replaced through destructive behavior without explicit user intent
such as the appropriate force/reset operation.

### OPS-SAFE-002 — Bounded deletion ownership
Recursive deletion/cleanup must be restricted to paths the operation owns and has validated. Source videos, authored scene
intent, reviewed authority evidence, and unrelated outputs must not be collateral damage of cache/work cleanup.

### OPS-SAFE-003 — Failed work remains diagnosable but bounded
Failed operations may retain compact diagnostics sufficient for triage, but retained failure state must be bounded and must
not masquerade as a complete reusable artifact.

## CI and artifact production

### OPS-CI-001 — Validation evaluates its input commit
Ordinary push/PR validation must answer whether the checked-in commit is acceptable. It must not silently change the branch
head it is evaluating and then report success for a different commit.

### OPS-CI-002 — Validators and producers are distinct responsibilities
Generated-artifact publication should be performed by an explicit trusted producer rather than smuggled into ordinary
validation. The producer must validate the exact generated state before publication.

### OPS-CI-003 — Narrow write authority
Jobs that only validate must remain read-only. A producer that needs repository/release writes must use the narrowest write
permission in the smallest responsible job and must not write from untrusted fork execution.

### OPS-CI-004 — Canonical producer profile is a policy decision
When generated ML/analysis bytes are published as canonical project state, the runner/runtime/device/profile must be
explicitly chosen from reproducibility/quality evidence. Hardware convenience alone must not silently redefine canonical
output semantics.

### OPS-CI-005 — Conservative gate scope optimization
Change-scope detectors may avoid expensive CI only as an optimization. They must prefer unnecessary execution over missing
a relevant correctness/acceptance gate.

## Diagnostics and operation feedback

### OPS-DIAG-001 — Failures are actionable
Operational failures should identify what failed, the affected artifact/asset where relevant, and the smallest useful next
action or command. Generated diagnostics should explain causal choices without being treated as acceptance authority.

### OPS-DIAG-002 — No silent fallback across quality contracts
Fallbacks that change model, precision, acceptance semantics, depth behavior, or other governed quality characteristics must
be explicit in the selected mode and recorded in provenance. A hardware fallback may not silently change the requested
numeric/quality contract.

### OPS-DIAG-003 — Sensitive/local data stays out of persisted diagnostics
Logs, manifests and review artifacts must not persist credentials or workstation-specific paths when portable identities
are sufficient.
