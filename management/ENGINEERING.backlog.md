# Engineering backlog

Unless overridden: WI-001–WI-031 are Planning since 2026-09-10; later IDs since 2026-09-11. Owner: Unassigned. [Lifecycle and evidence](GOVERNANCE.md#g-007-lifecycle) apply. Dependencies are work IDs; [decisions](DECISIONS.md) are resolved. Listing work does not authorize implementation.

## WI-001 Adopt one governance authority

Requires: [E-001](ENGINEERING.md#e-001-requirements-before-implementation), [E-008](ENGINEERING.md#e-008-discoverable-knowledge).
Depends: None.
Status: Review 2026-09-11 · Owner: Codex · Authorization: Q-005, this work session.

Consolidate the approved decisions and existing engineering obligations; connect README and agent instructions; distinguish implementation documentation. Preserve original human answers.
Done: human-reviewed corpus, valid references, and integration evidence. This documentation task installs no CI protection; WI-002/WI-029 establish it.

Evidence: working tree based on `7d0ce14aca0440d9f78e8bdd82f7eba97c2a2a72`. On 2026-09-11, `python3 /tmp/plaque-governance-verify.py` checked 30 Markdown files, 347 local references, 91 canonical IDs, and 41 work items with acyclic dependencies; original human answers matched exactly. `git diff --check` passed. All changes are documentation; 6,157 hashed source/asset files are unchanged. The temporary check report is `/tmp/plaque-governance-check.json`. Human review and integration remain pending; no commit was created.

## WI-002 Protect authority and its enforcing gate

Requires: [E-002](ENGINEERING.md#e-002-protected-acceptance), [S-002](SECURITY.md#s-002-trusted-execution-and-dependencies).
Depends: WI-001, WI-010, WI-029.

Implement the approved protected-artifact registry, trusted diff/approval checks, and protection of their own configuration. Cover additions, deletion, rename, indirect generation, thresholds, suite selection, exclusions, and verifier/gate changes according to the agreed ownership model.
Done: deliberate bypass attempts fail; approved amendments work; candidate code cannot forge approval or redefine the guard it is judged by.

## WI-003 Check requirement/work integrity

Requires: [E-001](ENGINEERING.md#e-001-requirements-before-implementation).
Depends: WI-001.

Validate stable IDs, references, dependencies, required work fields, and completion evidence using the smallest suitable mechanism. Keep semantic disagreements as review findings. Do not build a management application.
Done: malformed links, unknown requirements, contradictory lifecycle records, and unsupported completion claims are detected; valid ordinary Markdown remains easy to edit.

## WI-004 Prove that acceptance checks detect failure

Requires: [E-004](ENGINEERING.md#e-004-tests-constrain-behavior).
Depends: WI-002, WI-012.

Challenge the evaluators with absent/invisible text, wrong motion, lost foreground, opaque porous gaps, stale reports, and omitted cases. Pilot targeted mutation testing of critical non-ML logic.
Done: each intentional fault fails the corresponding requirement; surviving, unviable, and timed-out mutations receive distinct dispositions. Candidate-generated expected results cannot make the negative cases pass.

## WI-005 Establish generalization tests

Requires: [E-005](ENGINEERING.md#e-005-generalization-evidence), [P-001](PRODUCT.md#p-001-generalization).
Depends: WI-011, WI-020.

Add behavior/property tests for appropriate geometry, timing, title, and input transformations. Include renaming identical input, offscreen geometry, occlusion boundaries, and independent background motion where in scope. Transform annotations correctly.
Done: explicit expected relationships hold for valid variations; deliberately broken relationships fail; invalid transformations are rejected as tests.

## WI-006 Evaluate automatic analysis independently

Requires: [E-005](ENGINEERING.md#e-005-generalization-evidence), [P-002](PRODUCT.md#p-002-human-assistance).
Depends: WI-011, WI-020.

Separate surface discovery, tracking, foreground estimation, assisted analysis, and final rendering evaluations. Exclude cached answers and prohibited human inputs from each automatic-stage evaluation.
Done: reports state assistance and compare baseline/candidate quality on approved independent evidence; dense supplied trajectories cannot count as successful automatic tracking.

## WI-007 Establish architecture fitness checks

Requires: [E-003](ENGINEERING.md#e-003-replaceable-components).
Depends: WI-001, WI-011.

Map current dependencies and approved variation points. Evaluate replacing a worker/backend, adding a style, changing an encoder, and invoking application operations without CLI parsing. Select checks that detect forbidden dependencies under the project's relevant build features.
Done: agreed dependency rules, positive/negative checks, and a prioritized list of demonstrated obstacles. Do not refactor unrelated modules during the assessment.

## WI-008 Baseline maintainability and test effectiveness

Requires: [E-006](ENGINEERING.md#e-006-maintainability-and-effective-tests).
Depends: WI-001.

Pilot the [assessment's tools](ASSESSMENT.md#quality-measurement-and-enforcement) for duplication, complexity, dependencies, coverage, and mutation. Measure production and test code; review hotspots and false positives. Propose thresholds/exclusions from results.
Done: reproducible baseline, measured runtime cost, human-approved blocking criteria, and bounded remediation items. No blanket threshold or repository-wide cleanup is introduced by the pilot.

## WI-009 Audit hidden specialization

Requires: [E-007](ENGINEERING.md#e-007-general-rules-over-example-patches).
Depends: WI-011.

Trace source identity, dimensions, frame ranges, defaults, thresholds, prompt growth, and model-selection branches to their purpose. Distinguish integrity checks, explicit input intent, and general algorithms from example workarounds.
Done: every suspicious rule has evidence and a disposition; confirmed specialization becomes a separate corrective item with independent counterexamples. Do not remove legitimate scene inputs by filename heuristics.

## WI-010 Classify protected tests and acceptance inputs

Requires: [E-002](ENGINEERING.md#e-002-protected-acceptance), [P-010](PRODUCT.md#p-010-accepted-quality).
Depends: WI-001.

Inventory existing contracts, witness images, thresholds, accepted scene/trajectory/style/font inputs, capability selection, and test intentions, including tests embedded in production modules. Distinguish immutable baseline versions, refactorable test infrastructure, approved requirement-driven test changes, and diagnostics. Extend the registry when new baseline formats land; do not wait for all examples to be re-homologated.
Done: the human approves explicit ownership and amendment boundaries; transitive acceptance inputs cannot evade protection through another file; internal implementation details are not accidentally promoted to requirements.

## WI-033 Build and calibrate visual evaluators

Requires: [A-004](ACCEPTANCE.md#a-004-quality-and-fidelity), [A-006](ACCEPTANCE.md#a-006-trustworthy-evaluation), [E-004](ENGINEERING.md#e-004-tests-constrain-behavior).
Depends: WI-002, WI-011.

Define frame/pixel comparisons, per-dimension quality evidence, color/timing normalization, and numerical-repeatability allowances. Bootstrap with independently reviewed cases and controlled faults; do not require a newly accepted full corpus before building its evaluator.
Done: visible local/temporal defects fail, uncertainty is explicit, comparator settings are human-approved, and no aggregate or relaxed threshold hides drift. Retain fixtures and calibration evidence.
