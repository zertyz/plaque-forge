# Engineering policy

> **Status: NORMATIVE**
> **Owner: Maintainer**
> **Rule prefix: `ENG`**

This policy owns software-engineering concerns. Product/quality acceptance belongs to the Business policy and operational
execution, CI, artifact lifecycle, and provenance belong to the Operations policy.

## Documentation and discoverability

### ENG-DOC-001 — README as entry point and index
The project mission, architecture, and representative usage/output examples must be discoverable from `README.md`.
`README.md` should remain concise and may link to detailed documentation.

### ENG-DOC-002 — Rust documentation follows architecture
Project documentation that usefully belongs in generated Rust documentation should be incorporated directly or indirectly
through `lib.rs`. Code documentation should introduce broad concepts at higher module levels and increasingly specific
knowledge deeper in the hierarchy.

### ENG-DOC-003 — Documentation adds knowledge
Documentation should be concise and should explain concepts, rationale, constraints, or guidance not already expressed
clearly by names, types, executable tests, or examples. Prefer clearer naming/refactoring over redundant doc comments.

### ENG-DOC-004 — Usable entities remain navigable
Public and internally usable entities should have documentation where names/module placement alone do not adequately
express their role, so IDE and generated-documentation navigation remain useful.

## Architecture, layering and replaceability

### ENG-ARCH-001 — Architecture visible in code organization
Directory/module structure should progress from broad system components and concepts toward increasingly specific
implementation detail. The top level of `src/` should make major responsibilities and entry points discoverable.

### ENG-ARCH-002 — Dependency direction
Implementation details must depend on appropriate stable contracts/policies. Business/domain logic must not depend on
infrastructure details merely for convenience.

### ENG-ARCH-003 — Explicit dependencies and independent instantiation
Components should be independently instantiable where practical. When that is not practical, required dependencies must be
explicit and straightforward to obtain.

### ENG-ARCH-004 — Replace independent reasons to vary
Components with independent reasons to vary should be replaceable without unrelated changes elsewhere. Infrastructure
choices such as database/external-process implementations must not leak unnecessarily into business logic.

### ENG-ARCH-005 — Interface adapters remain adapters
Operations exposed through a CLI/UI should also be callable programmatically when the underlying operation is not
inherently tied to that interface. Interface parsing types must not become the domain/application API by accident.

### ENG-ARCH-006 — Dependency inversion is purposeful
Use dependency inversion where it improves testability, replaceability, or clarity of architectural boundaries. Do not
create interfaces/abstractions for ordinary pure implementation details without an independent reason to vary.

## TDD, testability and regression protection

### ENG-TDD-001 — Test-first behavior changes
New behavior should be expressed by tests before production implementation. A bug fix should first acquire a regression
test that demonstrates the defect, then the corrective implementation.

### ENG-TEST-001 — Important behavior has proportional automated protection
Core components and meaningful behavior should be covered to a degree proportional to their importance. The suite should
enable safe feature work and substantial internal refactoring with reasonable regression confidence.

### ENG-TEST-002 — Test observable behavior over incidental implementation
Tests should primarily verify externally meaningful behavior/contracts. Internal refactoring that preserves intended
behavior should not require widespread test rewrites merely because implementation structure changed.

### ENG-TEST-003 — Replace expensive and nondeterministic boundaries
Dependencies involving significant CPU/memory cost, nondeterministic/environmental inputs, network, filesystem, services,
databases, external processes, clocks, devices, or similar boundaries should be substitutable by deterministic test
implementations through the same production contracts where practical.

### ENG-TEST-004 — Replaceability supports testing
Test substitutes should already exist where broadly useful or be straightforward to implement without special-purpose
changes to production logic.

### ENG-TEST-005 — Test structure mirrors conceptual hierarchy
Higher-level tests should exercise broader interactions and deeper tests increasingly isolated behavior. Organization
should let a maintainer navigate from broad functionality toward specific requirements and details.

### ENG-TEST-006 — Failures identify the violated requirement
Related requirements may share a functionality-oriented test when that improves understanding, but individual requirements
must remain identifiable and assertion failures should clearly reveal what was violated.

### ENG-TEST-007 — Prefer real implementations when isolation adds no value
Use real implementations where doing so remains deterministic, practical, and clearer. A higher-level test may reasonably
look like a sub-integration test rather than maximizing mock isolation.

## Duplication and abstraction

### ENG-DUP-001 — One authoritative implementation of knowledge
Knowledge, rules, and intentions should have a single authoritative implementation whenever practical. Repeated code that
represents the same underlying knowledge should be generalized rather than duplicated.

### ENG-DUP-002 — Abstract shared concepts, not similar syntax
Abstractions must represent genuinely shared concepts. Do not couple unrelated concepts merely because their current code
looks similar. Eliminating duplicated knowledge is more important than eliminating superficial textual duplication.

### ENG-DUP-003 — Test infrastructure follows the same standard
Production and test code are subject to the same duplicated-knowledge and maintainability standards. Share reusable test
infrastructure when it removes repeated knowledge and improves readability, while keeping each test's behavior, inputs and
intent locally understandable.

## Scope and judgment

### ENG-SCOPE-001 — No unrelated compliance expansion
Apply policy fully to new and substantially modified code and improve directly affected code when reasonable. Do not
introduce unrelated architectural changes merely to bring old code into compliance during a narrowly scoped task.

### ENG-SIMPLE-001 — Prefer the simplest compliant design
Prefer the simplest design that satisfies product requirements and governance. Do not introduce abstractions merely to
comply mechanically with policy.
