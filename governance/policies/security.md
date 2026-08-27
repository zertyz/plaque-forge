# Security policy

> **Status: NORMATIVE**
> **Owner: Maintainer**
> **Rule prefix: `SEC`**

## Dependency security

### SEC-DEP-001 — Current advisory knowledge
The committed Rust dependency graph must be checked against a current RustSec advisory database in CI. New vulnerabilities,
yanked dependencies, unsoundness notices, and unapproved maintenance warnings fail the dependency-security gate.

### SEC-DEP-002 — Reviewable scanner identity
The dependency scanner/tool identity used by CI should be explicitly versioned so changes in scanner behavior are
reviewable independently from changes in the live advisory database.

### SEC-DEP-003 — Narrow, expiring exceptions
A security/advisory exception must be registered in `governance/security-exceptions.toml`, identify the exact advisory,
explain why it is currently accepted, define the condition for removal, and carry an explicit review trigger/date.
Exceptions must not broaden to unrelated warnings.

## Artifact and input boundaries

### SEC-ART-001 — No secrets or workstation paths in generated project artifacts
Generated manifests and committed evidence may record portable identifiers, hashes, dimensions and tool versions, but must
not contain model credentials, secrets, or absolute workstation paths.

### SEC-ART-002 — Generated caches are untrusted until validated
Analysis/model outputs must be validated for schema, provenance, dimensions, masks/prompts and portable path constraints
before they are accepted as reusable project state.

### SEC-ART-003 — External process invocation avoids shell injection surfaces
External tools should receive structured argument arrays rather than interpolated shell command strings whenever the
operation is implemented programmatically.

### SEC-ART-004 — Temporary external-runtime state is isolated
Intermediate ML frames, requests and model/runtime caches must remain inside explicitly owned temporary roots and cleanup
must validate containment before recursive deletion.

## Security change discipline

### SEC-CHG-001 — Security is a separate review dimension
A feature, refactor, performance improvement, CI repair, or portability change must not weaken security boundaries merely
because the resulting code is simpler or faster. If a security rule blocks a materially better design, escalate it through
governance rather than bypassing it.
