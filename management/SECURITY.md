# Security requirements

## S-001 Separate contribution from approval

Luiz Silveira is the human approver. Agents currently use his personal Git account. Git authorship and account identity therefore do not establish independent human approval.

Required control: agent/development credentials cannot approve protected authority, bypass its gate, or administer the controls constraining them. Keep human approval/admin capability outside agent and candidate execution.

Acceptance: inspect actual permissions and demonstrate denied unauthorized changes plus a usable human amendment path. This boundary is not yet established; implement [WI-029](SECURITY.backlog.md#wi-029-establish-the-real-approval-boundary).

## S-002 Trusted execution and dependencies

Execute candidate code without approval/admin credentials. Evaluate protected changes using trusted authority and guard logic from outside the candidate's control, including transitive configuration and required-check selection.

Keep secrets out of committed inputs, reports, and caches. Record dependency/model origins, versions, and relevant integrity identities. Do not execute candidate-controlled content with protected authority.

Invoke external tools with argument arrays rather than constructed shell commands. Validate untrusted generated caches under [O-002](OPERATIONS.md#o-002-identity-and-provenance).

Acceptance: adversarial gate tests, least-privilege review, and reproducible build/runtime provenance. [Protection rationale](ASSESSMENT.md#d-protection-must-include-the-acceptance-mechanism).

## S-003 Dependency checks and exceptions

Audit-tool version mismatch, dependency resolution/install failure, or an unaccepted audit finding stops the affected check. Report expected/observed versions or dependency state, the failed operation, and actionable remediation.

Do not silently install a substitute, rewrite policy, suppress findings, broaden an exception, or report success. Remediation is separate authorized work; rerun the check after it.

Pin enforcement-tool behavior and record advisory-data identity. Existing exceptions require explicit human scope, rationale, owner, and review/expiry rules. Unapproved findings and expired exceptions fail; overdue reviews require human disposition, not automatic renewal.

Acceptance: mismatches and failures produce actionable non-success results. [WI-030](SECURITY.backlog.md#wi-030-resolve-audit-policy-and-exception-drift) resolves the recorded scanner discrepancy and reviews the existing exception; this document neither renews nor removes it.

## S-004 Security reporting

Security fixes target the current `main` branch and newest release. Report suspected vulnerabilities privately to the maintainer; keep private media, credentials, workstation paths, and exploit-sensitive details out of public issues and generated artifacts. Track the affected revision, evidence, remediation, and validation through appropriately restricted records.

Acceptance: the reporting path is discoverable in [security implementation notes](../docs/SECURITY.md), with no claim that an unreviewed POC version is supported or secure.
