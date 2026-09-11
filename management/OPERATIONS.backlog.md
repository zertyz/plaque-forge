# Operations backlog

Unless overridden: Planning since 2026-09-10 · Owner: Unassigned. [Lifecycle and evidence](GOVERNANCE.md#g-007-lifecycle) apply.

## WI-023 Define and enforce visual CI coverage

Requires: [O-005](OPERATIONS.md#o-005-required-validation-coverage).
Depends: WI-002, WI-012, WI-015, WI-034, WI-035.

Run every accepted contract before output-affecting merges, including accepted replay/reconstruction lanes and currently noncontinuous contracts. Preserve unchanged-input failures during remapping review. Test source, scene, model, analysis, render, font/style, dependency, verifier, and CI selection. Give case/title/style selection one authority.
Done: approved checks actually execute for representative changes; missing/skipped/stale required cases fail; measured runtime and retained coverage evidence match the decision.

## WI-024 Qualify model and runtime promotion

Requires: [O-004](OPERATIONS.md#o-004-model-and-runtime-promotion).
Depends: WI-002, WI-006, WI-012, WI-034, WI-035.

Compare existing auxiliary tools with suitable alternatives on automatic and assisted quality. Measure actual quality configurations, including model size, input resolution, temporal processing, alpha refinement, and precision. Qualify CPU and Luiz's AMD/Intel iGPU hardware; do not depend on unavailable CUDA. Record correction effort, runtime/memory, provenance, and device fallback. A default profile name is not quality evidence.
Done: reproducible comparison and authorized promotion/rejection; accepted quality survives; candidate-generated masks or reports cannot redefine expected behavior.

## WI-025 Audit validation and publication identity

Requires: [O-003](OPERATIONS.md#o-003-exact-validation-and-publication), [O-008](OPERATIONS.md#o-008-release-and-recovery).
Depends: WI-002, WI-012.

Trace ordinary CI, generated-analysis validation, sample publication, lossless validation, and delivery transcodes to the exact revision/artifact each certifies. Compare public quality claims with actual evidence coverage. Investigate first; preserve existing correct producer/validator separation.
Done: each publication's scope is accurate and its evidence is current; wrong-byte reports, stale generated commits, and unvalidated substitutions are detected; gaps receive specific corrective work.

## WI-026 Verify source-bound data and recovery

Requires: [O-001](OPERATIONS.md#o-001-owned-data-and-transactions), [O-002](OPERATIONS.md#o-002-identity-and-provenance).
Depends: WI-010, WI-017.

Audit source replacement at the same path, annotation dependencies, cache invalidation, interrupted publication, concurrent work, and guarded cleanup. Distinguish immutable acceptance inputs from replaceable generated results.
Done: stale dependencies are rejected/reviewed, prior complete bundles recover, and no source/human authority data is lost. Extend existing tests only for demonstrated gaps.

## WI-027 Qualify media and portable workflows

Requires: [P-009](PRODUCT.md#p-009-media-fidelity-and-scope), [P-012](PRODUCT.md#p-012-usable-delivery), [O-006](OPERATIONS.md#o-006-reproducible-execution).
Depends: WI-011.

Qualify current media and bundled/programmatic/CLI behavior in clean CPU/AMD/Intel environments. Include audio, rotation, encoding, and advertised-versus-decoded frame counts. Identify gaps for WI-036/WI-041; do not make current implementation ceilings permanent requirements or “correct” valid shorter timelines blindly.
Done: supported behavior is demonstrated, unsupported behavior fails explicitly, and delivery prerequisites/resource limits are documented from evidence.

## WI-028 Repair documentation references and claim drift

Requires: [E-008](ENGINEERING.md#e-008-discoverable-knowledge), [O-007](OPERATIONS.md#o-007-diagnostic-evidence).
Depends: None.
Status: Review 2026-09-11 · Owner: Codex · Authorization: Q-005, this work session.

Repair root-relative links affected by moving the asset assessment into `docs/`. Check management/README navigation and wording that conflates existing-title replacement with the current text-free input requirement, or generated verification with human acceptance.
Done: local links resolve and capability/quality claims match the approved scope. Update explanations without silently changing product requirements.

Evidence: 2026-09-11, 105 assessment links repaired with its findings otherwise unchanged; README and implementation notes distinguish current support/verification from requirements. Structural and scope checks are recorded in WI-001. Human review and integration remain pending.
