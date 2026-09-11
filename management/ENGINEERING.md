# Engineering policy and requirements

**Normative code engineering policy**, consolidated from the former `docs/engineering-policy.md` under [Q-005](DECISIONS.md#resolved-questions). It governs production and test code. Acceptance ownership follows [Governance](GOVERNANCE.md).

## E-001 Requirements before implementation

New behavior and bug fixes start from an agreed requirement and bounded work item. Write a meaningful failing test before the corresponding implementation, then make it pass and refactor. Record the observed failure and relevant passing checks.

Existing tests do not prove historical TDD. Assess existing code through observable coverage, design, and evidence. Record discrepancies between intention, code, and tests instead of silently making them agree.

Acceptance: traceable requirements, tests, work, and completion evidence under [G-006](GOVERNANCE.md#g-006-requirements-and-work).

## E-002 Protected acceptance

Enforce [G-002–G-004](GOVERNANCE.md#g-002-human-ownership), including the mechanism enforcing those rules. Protect expected behavior, tolerances, evidence inputs, suite selection, and gates.

Acceptance: unauthorized direct and indirect amendments fail; authorized amendments work. Candidate code cannot select or replace the authority judging its own change. Implementation remains [WI-002](ENGINEERING.backlog.md#wi-002-protect-authority-and-its-enforcing-gate).

## E-003 Replaceable components

Organize broad components into progressively detailed modules: entry points → concepts → implementation details. Directory structure must expose the architecture.

Business/application logic depends on stable contracts, not CLI parsing, infrastructure, model internals, or storage/process details. Components with independent reasons to vary must be replaceable without unrelated changes. Instantiate components independently where practical; otherwise expose explicit, easily obtained dependencies.

Expose interface-independent operations programmatically as well as through the CLI. Use dependency inversion where it clarifies a real boundary or improves replaceability; do not impose a crate layout or abstract every implementation mechanically.

Acceptance: representative backend, encoder, style, and interface replacement scenarios; checks for agreed dependency direction.

## E-004 Tests constrain behavior

Test observable requirements in proportion to their importance. Protect homologated behavior while allowing internal contracts and implementation details to change. Use diagnostic assertions; a failure must identify the violated intention.

Test-edit authority follows [G-002](GOVERNANCE.md#g-002-human-ownership): protect accepted intentions while allowing refactoring and approved requirement-driven changes.

Tests should follow the production hierarchy: broad interactions near component entry points, narrower behaviors deeper inside. Group related requirements by functionality; keep their individual assertions identifiable. Prefer real implementations where practical, including component integration tests.

Expensive, nondeterministic, and external dependencies must support straightforward substitutes through the same contracts used by production. This includes models, clocks, devices, filesystems, services, and networks. Do not add special test-only branches to production behavior.

Independent acceptance uses reviewed evidence under [Acceptance](ACCEPTANCE.md). Generated analysis and diagnostic traces cannot certify themselves. Test critical evaluators with deliberate faults and targeted mutations; classify missed, unviable, and timed-out mutations separately.

Acceptance: meaningful failing/passing evidence, independently testable components, readable specifications, and demonstrated detection of relevant regressions.

## E-005 Generalization evidence

Evaluate automatic analysis, human-assisted reconstruction, and rendering separately. Keep tuning inputs and evaluation material identifiable. Use unfamiliar cases, interacting capabilities, and transformations with defined expected relationships.

An automatic-stage evaluation cannot consume the approved dense answer. Supplied geometry, trajectories, prompts, and masks remain assistance even when a tool generated them.

Acceptance: source/assistance provenance, per-stage baseline/candidate results, and independent examples. Renaming identical source content must not change semantic behavior.

## E-006 Maintainability and effective tests

Give shared knowledge one implementation. Generalize repeated rules and reusable infrastructure; do not couple unrelated concepts because their code looks similar.

Apply this standard to tests and production alike. Share duplicated test infrastructure while keeping case inputs, outputs, nuances, and intentions locally understandable. Do not hide specifications behind elaborate helper layers.

Measure dependency violations, duplicated knowledge, complexity hotspots, test reach/effectiveness, and representative change impact separately. Calibrate blocking criteria and exclusions from observed results and human-approved policy; do not invent a universal engineering score.

Acceptance: reproducible measurements, tool/configuration identity, reviewed findings, and maintainable behavior-oriented tests.

## E-007 General rules over example patches

Do not select production behavior by example filename, source hash, or disguised equivalent. Content-driven rules and explicit human intent need a reusable explanation and independent evidence. New settings must express general concepts rather than conceal per-video algorithms.

Identity hashes remain valid for integrity, provenance, and compatible caching. Reviewed per-video inputs are legitimate assistance under [P-002](PRODUCT.md#p-002-human-assistance).

Acceptance: cause, scope, alternatives, and independent cases for exceptional rules; no input-identity dispatch masquerading as analysis.

## E-008 Discoverable knowledge

README is the concise entry point to mission, architecture, representative usage/output, and management. Incorporate relevant linked documentation into `lib.rs`/generated Rust documentation where appropriate.

Follow the conceptual module hierarchy: introduce broad concepts before detail. Give each rule one authoritative home. Documentation supplies context, rationale, constraints, and guidance; link executable specifications rather than restating them.

Document usable public and internal entities where names and module context are insufficient. Prefer clear naming or renaming over redundant comments. Keep IDE navigation and generated documentation useful.

Acceptance: valid links, clear architecture/usage entry points, and traceability from requirements to work and evidence.

## E-009 Controlled improvement

Apply this policy fully to new code and substantially modified code. Improve directly affected design within scope. Do not perform unrelated architectural migrations or introduce abstractions for mechanical compliance.

Internal details and contracts may change freely during refactoring while intended and homologated observable behavior remains preserved. Deliberate behavior changes update the human-approved requirement and corresponding acceptance evidence in the same change.

Performance and model changes satisfy the same acceptance rules and the validation coverage in [O-005](OPERATIONS.md#o-005-required-validation-coverage).

Acceptance: bounded diff, simplest adequate design, relevant code checks, and required visual regression evidence.
