# Project instructions

Before designing, modifying, or reviewing production or test code,
read and follow `docs/engineering-policy.md`.

That document defines the project's engineering policy and is normative.

In particular:
- Follow TDD for new behavior and bug fixes.
- Preserve architectural dependency direction and replaceability.
- Prefer tests of observable behavior over implementation details.
- Keep project mission, architecture, and representative usage discoverable
  from README.md.
- Treat production and test code as subject to the same standards regarding
  duplicated knowledge and maintainability.

Do not introduce unrelated architectural migrations merely to bring old code
into compliance while performing a narrowly scoped task. New and substantially
modified code should follow the policy.

When refactoring, internal implementation details and internal contracts may change freely,
but intended and homologated observable behavior must be preserved unless the task explicitly
changes that behavior. A deliberate behavior change must update the corresponding requirement
and homologation evidence in the same change.

## Application of this policy

These principles govern design decisions; they are not a mandate to
refactor unrelated existing code.

When modifying existing code:
- apply the policy fully to new code;
- improve directly affected code where doing so is reasonably within scope;
- do not perform unrelated architectural cleanup unless explicitly requested;
- prefer the simplest design that satisfies these principles;
- do not introduce abstractions merely to comply mechanically with the policy.
