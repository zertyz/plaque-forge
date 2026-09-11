# Governance

Human authority: **Luiz Silveira**. Basis: [Decisions](DECISIONS.md).

## G-001 Authority

Policies constrain evolution. Requirements define desired behavior. Work items bound changes. Tests and reports supply evidence. Human acceptance establishes the approved baseline. Code records implementation.

This file and the domain requirements are normative. [Engineering](ENGINEERING.md) contains the code engineering policy; [Acceptance](ACCEPTANCE.md) defines homologation. `AGENTS.md` loads this authority. Documents under `docs/` describe implementation, usage, or historical findings; they cannot independently redefine requirements.

A work item, generated report, implementation, or AI assessment cannot amend accepted authority. Product, engineering, operations, and security do not outrank one another. Resolve conflicts under G-005.

## G-002 Human ownership

| Subject | Agent authority |
| --- | --- |
| Policies and accepted requirements | Analyze and propose amendments. Apply only the human-authorized amendment scope. |
| Existing tests | Refactor while preserving accepted intentions; rework affected tests when the human approves a requirement change. |
| New behavior | Add tests under TDD and implement the approved requirement. Human acceptance establishes the new protected intention. |
| Homologated scenes, renderings, witnesses, and human inputs | Produce separate candidates and comparison evidence. Human approval promotes a successor; preserve the previous record. |
| Thresholds, required cases, exemptions, gates, and ownership controls | Treat as protected authority. Implementation permission does not authorize weakening them. |
| Work records and diagnostics | Maintain factual records within the assigned scope. They cannot grant approval. |

Protect meaning, not incidental test structure. Refactoring may change internal APIs, fixtures, helpers, and organization while preserving accepted cases, assertions, tolerances, and coverage. A changed requirement authorizes corresponding test rework; it does not authorize unrelated weakening.

## G-003 Registered authority

Maintain a human-approved registry identifying protected artifacts, their roles, and amendment authority. Include policies, requirements, accepted test intentions, scene/input identities, resolved baselines, witnesses, fonts/styles used as acceptance inputs, thresholds, capability selection, skip rules, and enforcement configuration.

Trace transitive inputs: moving a value into another file or generating it does not remove protection. Separate editable source intent, approved input versions, frozen acceptance evidence, reproducible caches, and diagnostics.

The registry and guard are [planned](ENGINEERING.backlog.md#wi-002-protect-authority-and-its-enforcing-gate). Their absence does not permit changing known accepted behavior. Unknown historical approval must be investigated, not invented.

## G-004 Amendments

A proposal identifies the affected requirement/intention, reason, exact candidate revision, validation impact, and evidence. Visual changes include BEFORE/AFTER renders and per-dimension findings, with the agent's recommendation.

Human approval of a requirement change authorizes related implementation and test rework within that scope. No additional exact-test-diff authorization is required merely to perform that work. Acceptance of changed observable behavior still requires its corresponding reviewed evidence in the same change.

Policy amendments and baseline promotion need explicit human authorization. Approval covers the reviewed scope and revision; material changes require renewed review. Never overwrite historical homologation or disguise changed behavior as a refactor, cache refresh, or automatic reapproval.

## G-005 Ambiguity and conflicts

Identify the conflicting statements and practical consequences. Ask Luiz before dependent work; continue independent work. Do not choose a requirement because one interpretation seems more likely.

Resolve routine implementation choices within agreed requirements. Do not add complexity, duplicate authority, or weaken verification to avoid raising a real conflict.

## G-006 Requirements and work

Use stable IDs: governance `G-`; product `P-`; acceptance `A-`; engineering `E-`; operations `O-`; security `S-`; decisions `D-`/`Q-`; work `WI-`. Do not renumber when priority, owner, or organization changes.

Each work item links its requirements, scope, dependencies, and completion evidence. State actual status and owner. Investigations distinguish suspicion from confirmed defect and may conclude that an input, expectation, document, or implementation needs correction.

## G-007 Lifecycle

`Planning → Ready → Started → Review → Accepted → Integrated`.

| State | Required record |
| --- | --- |
| Planning | Bounded proposal, requirements, dependencies, and unresolved work |
| Ready | Agreed acceptance, resolved blockers, owner, verification plan |
| Started | Assignment/authorization and actual work context |
| Review | Concrete diff, relevant checks, results, remaining limits, authority impact |
| Accepted | Required checks and human decisions for the reviewed revision |
| Integrated | Accepted change in the target branch, commit, integration evidence |

Record actual transition dates and responsibility. Blocked is a condition with its reason. Cancelled/superseded items retain their disposition. Publication is recorded separately when relevant.

“Done” means integrated with evidence. A prepared patch or an agent's declaration is insufficient. Do not reconstruct unrecorded historical transitions.

## G-008 Evidence

Record requirement/work IDs, revision, inputs, execution profile, command, result, and retained artifacts as applicable. Human acceptance records reviewer, decision, date, and reviewed revision/artifact.

Keep traceability in the work item; derive other views. Missing, stale, skipped, failed, and passed evidence are distinct states.

## G-009 Agent operating sequence

Read the applicable authority and assigned work. Resolve material questions. Follow TDD and the engineering policy. Run required checks. Present the actual diff, evidence, and unresolved limits.

A backlog entry does not itself authorize implementation, acceptance, merge, or publication. Use authorization already granted for the task; do not repeatedly request it.
