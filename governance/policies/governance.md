# Governance policy

This policy defines the authority model for Plaque Forge. It governs humans and automated agents interacting with the
project.


## Authority and normative sources

1. Policies, requirements, reviewed acceptance contracts, acceptance-policy artifacts, and governance controls stored at
`governance/` are human-authority artifacts. Automated implementation agents must treat them as read-only and, in any
circumstances, change them.
2. When an authority artifact appears wrong, incomplete, stale, contradictory, or unnecessarily restrictive, an agent should
propose a revision and stop working.
3. Only artifacts under `governance/` are normative. Imperative wording anywhere else  does not create policy.
4. If implementation, CI, tests, documentation, or generated evidence disagrees with a registered authority, the mismatch is
drift to investigate. This should be flagged and the work must stop.


## Agent postflight

5. When an implementation plan or approach conflicts with the contents of `governance/`, the agent must make a reasonable
attempt to find a clean, compliant alternative that still satisfies the task. If none exists, the current set of authority &
normative rules must be declared as "blocking progress". The situation should be exposed and work must stop.


## Agent postflight

6. Before commiting the changes -- or before declaring them as done -- the agent must double validate their work against the
contents of `/governance`.


## No compliance theater

7. An agent must not avoid escalation by introducing material accidental complexity, duplicated knowledge, artificial
abstractions, weaker tests, hidden quality reductions, unsafe fallbacks, security regressions, or disproportionate runtime
or operational burden whose main purpose is to technically remain inside a rule.

8. If every reasonable compliant implementation would materially degrade correctness, maintainability, testability,
architecture, security, safety, quality, or operations, the agent must stop the blocked part of the task and request a
human review of the constraining authority.


## Conflicts and stopping conditions

9. Substantive policy categories do not have a priority ladder. If two registered authorities appear irreconcilable, the
agent must report the conflict and stop the affected work rather than choosing which rule wins.