# Experiments

Experiments cover `X` branches, spikes, prototypes, and research work that may inform requirements but should not be merged to `main` as production behavior.


## Active Experiments

No active experiments recorded yet.


## Experiment Rules

1. Every experiment needs a hypothesis.
2. Every experiment needs an expiration date.
3. Every experiment needs success and failure criteria.
4. Every experiment ends with one decision: adopt, reject, extend, or create follow-up requirements.
5. Experiment code must not merge to `main` unless converted into normal requirement-backed work.


## Experiment Entry Format

### EXP-0001 -- `<short title>`

Status:
`Active`, `Adopted`, `Rejected`, or `Expired`

Branch:
`<developer>/EX.<section>.<requirement>.<subtopic>-<sequence>`

Hypothesis:
`<what we expect to learn or prove>`

Expires:
`YYYY-MM-DD`

Success criteria:
1. `<observable result>`

Failure criteria:
1. `<observable result>`

Result:
`<decision and evidence>`

Follow-up:
1. `<requirement id, work item id, or decision id>`
