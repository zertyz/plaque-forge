# Decisions

Authority: Luiz Silveira. Sources: [original answers, 2026-09-10](decisions/2026-09-10-luiz.md) and [clarifications, 2026-09-11](decisions/2026-09-11-luiz.md). Later clarifications resolve earlier wording.

## Direction

| ID | Decision |
| --- | --- |
| D-001 | Engineer the POC toward reliable agentic development. |
| D-002 | Generalize beyond the supplied examples; preserve accepted behavior. |
| D-003 | Establish governance first. This phase changes documentation only. |
| D-004 | Separate requirements from work; use concise Markdown under `/management`. |
| D-005 | Humans own policy and accepted intentions. Apply the amendment rules in Q-001/Q-002. |
| D-006 | Ask about material ambiguity or conflicting requirements before dependent work. |

## Resolved questions

| ID | Decision | Authority |
| --- | --- | --- |
| Q-001 | Agents propose protected changes with reasons and BEFORE/AFTER evidence where applicable. Human authorization controls application and acceptance. | [G-002](GOVERNANCE.md#g-002-human-ownership) |
| Q-002 | Agents may add tests, refactor without changing accepted intentions, and rework related tests for a human-approved requirement change. Preserve unrelated intentions. | [E-004](ENGINEERING.md#e-004-tests-constrain-behavior) |
| Q-003 | Analyze automatically first; refine when needed. Freeze the approved resolved scene and rendering; separately assess reconstruction and correction effort. | [Acceptance](ACCEPTANCE.md) |
| Q-004 | Review all examples under the new protocol. Existing accepted contracts remain binding until reviewed successors replace them. | [A-007](ACCEPTANCE.md#a-007-establishment-and-promotion) |
| Q-005 | Consolidate existing authority now, including the project entry points. | [G-001](GOVERNANCE.md#g-001-authority) |
| Q-006 | Include existing-title removal, HDR/wide-gamut, multiple title surfaces, and shot changes. | [P-009](PRODUCT.md#p-009-media-fidelity-and-scope), [P-013](PRODUCT.md#p-013-existing-title-removal) |
| Q-007 | Conflicting effective scene/artifact timing or layout declarations fail. | [P-003](PRODUCT.md#p-003-scene-meaning) |
| Q-008 | Run every accepted contract before an output-affecting merge. Use relevant checks for unrelated changes. | [O-005](OPERATIONS.md#o-005-required-validation-coverage) |
| Q-009 | Impose no arbitrary duration, resolution, frame-rate, shot-change, or deformation ceiling. Reject when accurate processing cannot be established. | [P-005](PRODUCT.md#p-005-motion-and-visibility), [P-011](PRODUCT.md#p-011-honest-confidence-and-failure) |
| Q-010 | Prefer known visual perfection; imperfect homologations are usable. Later renders must be visually faithful to approved data, with no drift. Develop evidence-based evaluators; do not invent permissive thresholds. | [A-004](ACCEPTANCE.md#a-004-quality-and-fidelity) |
| Q-011 | No arbitrary quality-reducing compute/storage budget. Qualify auxiliary tools and actual quality settings on available AMD/Intel iGPUs and CPU; NVIDIA/CUDA is unavailable to Luiz. | [O-004](OPERATIONS.md#o-004-model-and-runtime-promotion), [O-006](OPERATIONS.md#o-006-reproducible-execution) |
| Q-012 | Luiz Silveira approves. Agents currently use his personal Git account; independent approval enforcement remains work. | [S-001](SECURITY.md#s-001-separate-contribution-from-approval) |
| Q-013 | Audit-tool mismatch or dependency failure stops the check with the cause and remediation. Do not silently repair, suppress, or waive it. The earlier root-file answer was a misunderstanding. | [S-003](SECURITY.md#s-003-dependency-checks-and-exceptions) |

No governance question above remains unanswered. Device qualification, metric calibration, artifact formats, and permission configuration are planned work; their results must satisfy these decisions.
