# Plaque Forge governance

> **Status: Descriptive index**

This directory is the canonical entry point for Plaque Forge policies, requirements, and protected human-authority
artifacts. The machine-readable registry is [`manifest.toml`](manifest.toml).

## Authority surface

| Kind | Purpose | Canonical location | Agent-editable? |
|---|---|---|---:|
| Policy | Durable constraints on how the project may evolve | `policies/` | **No** |
| Requirement | Human-owned intended behavior and acceptance constraints | `requirements/` | **No** |
| Human authority artifact | Reviewed acceptance evidence or executable acceptance policy | registered in `manifest.toml` | **No** |
| Mechanism | Code, tests, CI, scripts and validators that implement/enforce authority | repository implementation | Normally yes |
| Documentation | Explanations, architecture descriptions, workflows and runbooks | `docs/`, root `README.md` | Normally yes |

Only authority registered by `manifest.toml` is normative. Descriptive documentation and implementation mechanisms may
refer to authority IDs, but they do not create or override policy by themselves.

## Policies

- [`policies/governance.md`](policies/governance.md) — authority, agent behavior, conflicts, escalation, and policy changes.
- [`policies/business.md`](policies/business.md) — product mission, user-visible quality, and accepted behavior.
- [`policies/engineering.md`](policies/engineering.md) — architecture, testability, TDD, maintainability, dependency
  direction, and documentation discipline.
- [`policies/operations.md`](policies/operations.md) — execution, CI/CD, artifacts, provenance, diagnostics, lifecycle,
  and destructive actions.
- [`policies/security.md`](policies/security.md) — dependency security, untrusted inputs, isolation, and security
  exceptions.

## Requirements

- [`requirements/product.md`](requirements/product.md)
- [`requirements/quality.md`](requirements/quality.md)
- [`requirements/analysis-and-rendering.md`](requirements/analysis-and-rendering.md)
- [`requirements/homologation.md`](requirements/homologation.md)
- [`requirements/operations.md`](requirements/operations.md)

Requirements use stable IDs so tests, work items, diagnostics, and descriptive documents can refer to the intended behavior
without copying normative prose.

## Protected human authority

In addition to policy and requirement prose, the manifest protects reviewed data that can redefine acceptance if changed:

- `assets/homologation/**`
- `assets/segmentation/policy.toml`
- `governance/security-exceptions.toml`

Root [`AGENTS.md`](../AGENTS.md) bootstraps automated agents into this authority model. Ordinary implementation agents may
propose authority changes when blocked, but must not apply them.

## Enforcement

[`scripts/check_governance.sh`](../scripts/check_governance.sh) rejects ordinary diffs that modify protected authority or
the controls protecting it. [`.github/workflows/governance-integrity.yml`](../.github/workflows/governance-integrity.yml)
runs the trusted base-branch guard for pull requests.

A deliberate human authority change uses the `governance-change` review lane. Repository settings should make
`Governance integrity / policy-integrity` required on protected branches, restrict direct pushes to `main`, and require
appropriate maintainer/code-owner review for protected authority.
