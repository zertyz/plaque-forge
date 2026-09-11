# Plaque Forge management

**Mission: add or replace artistic titles on unfamiliar videos, with stable accepted quality and decreasing human correction effort. Supplied assets are examples.**

This is the authoritative requirements and governance entry point, consolidated under [Luiz's decisions](DECISIONS.md). Requirements describe the intended product; [implementation documentation](../docs/ARCHITECTURE.md) describes the POC.

| Authority | Purpose | Work |
| --- | --- | --- |
| [Governance](GOVERNANCE.md) | Human ownership, amendments, work lifecycle | [Engineering backlog](ENGINEERING.backlog.md) |
| [Product](PRODUCT.md) | Capabilities and observable behavior | [Product backlog](PRODUCT.backlog.md) |
| [Acceptance](ACCEPTANCE.md) | Frozen homologation, reconstruction, quality, correction effort | [Product backlog](PRODUCT.backlog.md#wi-032-capture-and-replay-complete-homologations) |
| [Engineering](ENGINEERING.md) | Code policy, architecture, tests, quality controls | [Engineering backlog](ENGINEERING.backlog.md) |
| [Operations](OPERATIONS.md) | Evidence, CI, model qualification, delivery | [Operations backlog](OPERATIONS.backlog.md) |
| [Security](SECURITY.md) | Approval boundary, trusted execution, dependency failures | [Security backlog](SECURITY.backlog.md) |

[Decisions](DECISIONS.md) retain the answers and their original records. [Assessment](ASSESSMENT.md) contains repository findings, reference analysis, and tooling rationale.

**Current phase: documentation only.** Implementation work remains unassigned and under planning. Documentation items identify their actual review state. No production, test, CI, dependency, model, or asset changes are authorized by merely listing them here.

**Enforcement gap:** agents currently use Luiz's Git account. Written ownership rules are binding instructions, but an independent access boundary and protected gates are still [planned work](SECURITY.backlog.md#wi-029-establish-the-real-approval-boundary).

Sequence: review the consolidated documents → establish protected authority → implement and validate the acceptance protocol → review baselines → improve generalization and capabilities.
