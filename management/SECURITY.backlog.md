# Security backlog

All items: Planning since 2026-09-10 · Owner: Unassigned. [Lifecycle and evidence](GOVERNANCE.md#g-007-lifecycle) apply.

## WI-029 Establish the real approval boundary

Requires: [S-001](SECURITY.md#s-001-separate-contribution-from-approval).
Depends: WI-001.

Start from the known shared personal account. Inspect live protections, ownership/bypass permissions, credential scopes, and required-check sources. Select and establish a boundary that agents cannot use to approve themselves, supported by the actual repository/account configuration. Keep approval/admin credentials outside agent and candidate execution.
Done: the human can approve the exact reviewed change; ordinary agent credentials cannot edit protected authority through merge, bypass the gate, or grant themselves approval. Record actual configuration and negative-check evidence.

## WI-030 Resolve audit policy and exception drift

Requires: [S-003](SECURITY.md#s-003-dependency-checks-and-exceptions).
Depends: WI-001.

Resolve the pinned-scanner claim versus unpinned CI install. Make version mismatch, dependency failure, and unaccepted findings stop with expected/observed state and remediation. Reassess the recorded `RUSTSEC-2026-0192` exception using current evidence and human disposition. Do not auto-install, suppress, or renew to obtain green CI.
Done: one authoritative scanner policy, matching execution, attributable exception decision, and checks that detect unauthorized/expired exceptions. Do not silently suppress new findings.

## WI-031 Audit trusted build and model inputs

Requires: [S-002](SECURITY.md#s-002-trusted-execution-and-dependencies), [O-006](OPERATIONS.md#o-006-reproducible-execution).
Depends: WI-001.

Inventory CI images/actions/toolchains, Rust/Python dependencies, model weights/repos, caches, and worker execution. Identify mutable inputs and privilege boundaries that affect reproducibility or protected validation; propose only justified controls.
Done: recorded identities and permissions, measured gaps, and approved remediation. Existing runtime isolation and pinned worker identities are retained where correct; no unrelated dependency migration is implied.
