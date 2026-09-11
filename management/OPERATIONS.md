# Operations requirements

## O-001 Owned data and transactions

Distinguish source media, editable human intent, approved input versions, immutable homologations, generated analysis, runtime/model caches, and outputs. Operations replace only their authorized data.

Publish complete bundles transactionally. Interruption, concurrency, reset, and cleanup preserve authoritative data and the previous complete result. Frozen references are not caches.

Acceptance: interruption/recovery and owned-cleanup evidence. [Current filesystem implementation](../docs/SAFETY.md).

## O-002 Identity and provenance

Bind analysis to source, human inputs, relevant implementation, and model/runtime identities. Reject incompatible or stale data; do not relabel it. Replacing a source at the same path triggers review of all source-bound definitions.

Record creation, correction, and review provenance when inputs become acceptance authority. Historical unknowns remain explicit.

Generated project manifests use portable identifiers and content hashes, not workstation paths or credentials. Validate schemas, dimensions, masks/prompts, provenance, and path confinement before consuming caches.

Acceptance: mutations invalidate affected caches/evidence; approved baselines and their transitive identities remain recoverable.

## O-003 Exact validation and publication

Validate the requested revision and exact certified artifact. Reports for other bytes are stale. Keep validation and generated-artifact production distinct, with explicit write authority and validation of the resulting revision before promotion.

Acceptance: delivered artifact/report identities match. A lossless render's evidence cannot silently certify a delivery transcode. [Current validation](../docs/VALIDATION.md), [CI](../docs/CI.md).

## O-004 Model and runtime promotion

Reassess auxiliary tools against relevant alternatives; existing choices are not presumed best. Qualify their actual quality settings, including model variant, input resolution, temporal strategy, refinement, precision, device, and fallback behavior.

Compare automatic stages, assisted reconstruction, frozen replay where affected, and correction effort using the same approved targets. Record runtime/memory as evidence, not permission to lower quality.

Required workflows must work on Luiz's available hardware under O-006. Optional accelerator paths do not establish that baseline. A label such as “canonical” or FP32 alone does not demonstrate optimal quality.

Acceptance: reproducible comparative evidence and explicit promotion/rejection; all accepted behavior remains protected. [Current segmentation implementation](../docs/SEGMENTATION.md).

## O-005 Required validation coverage

**Every accepted contract must pass before an output-affecting merge.** This includes source/scene, analysis/model, renderer/style/font, relevant dependency, verifier, and selection/gate changes. Unknown impact is treated as potentially output-affecting.

Use relevant checks for changes proven unrelated to output. Capability inventories organize and expose gaps; they do not authorize replacing the full accepted suite with five representative cases.

Run the accepted replay and reconstruction lanes defined in [Acceptance](ACCEPTANCE.md). Preserve failures with existing assistance when evaluating candidate remapping. Missing, skipped, failed, stale, or unapproved evidence cannot pass. Release/publication validates the exact delivered revision and artifacts under O-003.

Acceptance: selection tests cover each change class; every required case actually executes. The current smaller suite remains an explicit implementation gap.

## O-006 Reproducible execution

Provide isolated setup, declared dependencies, offline normal analysis/render/review after setup, and actionable recovery. Record versions, actual device, numeric profile, nondeterminism, time, memory, and storage.

Qualify CPU and the available AMD/Intel iGPU environments; do not require NVIDIA/CUDA for Luiz's workflow. Determine exact device/driver compatibility during qualification. Hardware selection must not silently change the requested quality profile.

Do not impose an arbitrary CI/runtime/storage budget that weakens accepted quality. Finite resource failures stop with a useful diagnosis; do not silently subsample, downscale, skip contracts, or discard acceptance evidence. Preserve bounded ownership/cleanup safety.

Acceptance: clean-environment qualification and explicit distinction among artifact identity, numerical repeatability, and visual fidelity. Dependency-audit refresh is a declared network operation separate from normal offline workflows.

## O-007 Diagnostic evidence

Identify the affected stage, failure evidence, assistance, model participation, and precise rerun/remediation. Retain enough provenance-bound evidence to diagnose the failure; never present diagnostic traces as independent acceptance.

Acceptance: useful reports for analysis, rendering, evaluation, dependency, and resource failures, with valid artifact references.

## O-008 Release and recovery

Record the published revision/artifacts, validation scope, and required human acceptance. Preserve a recoverable approved baseline when promoting code, models, or data.

Acceptance: matched publication evidence and recovery for the changed artifact class. Label previews and unaccepted outputs accurately; a branch name does not establish production readiness.
