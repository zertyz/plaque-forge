# Operational requirements

> **Status: NORMATIVE**
> **Requirement prefix: `REQ-OPS`**
> **Primary policies:** `OPS-*`, `SEC-*`

### REQ-OPS-001 — Optional ML runtime is isolated
The supported Python/model runtime shall remain isolated below the Plaque Forge temporary runtime root rather than
installing packages/models into the user's normal Python environment or user model caches.

### REQ-OPS-002 — Normal processing is offline-capable
Normal analysis (after optional ML runtime setup), rendering and verification shall not require network access. Network use
for dependency/model installation shall be explicit setup behavior.

### REQ-OPS-003 — Explicit analysis reset is narrowly scoped
The high-level full analysis reset shall delete generated scene-analysis caches only. It shall preserve source videos,
authored scene intent, plaque assets, rendered output, homologation authority and the installed ML runtime.

### REQ-OPS-004 — Failure evidence is bounded
Failed analysis/work may retain compact diagnostics needed for troubleshooting, but retention shall be bounded and
successful replacement shall remove obsolete failure evidence for the affected asset as appropriate.

### REQ-OPS-005 — Ordinary CI is read-only validation
Normal push/pull-request validation shall not commit generated analysis or otherwise rewrite the branch under test.

### REQ-OPS-006 — Trusted generation publishes a separately validated state
If automation produces repository/release artifacts, generation shall occur in an explicit producer workflow whose output
is independently validated before publication, with provenance pointing to the exact producing source state/runtime.

### REQ-OPS-007 — Render/analysis failures provide actionable remediation
When stale/missing/incompatible caches, runtime setup, validation or rendering block progress, the user-facing failure shall
identify the affected asset/artifact and provide the smallest practical remediation or rerun command.

### REQ-OPS-008 — Cleanup never relies on arbitrary recursive paths
Cleanup/destructive helpers shall validate that their target belongs to the operation-owned root before recursive deletion
and shall refuse paths outside that boundary.

### REQ-OPS-009 — Published sample artifacts are accepted before write publication
A workflow that publishes public sample videos shall separate read-only rendering/validation from the narrow write step and
shall publish only after the configured acceptance checks for those artifacts complete.
