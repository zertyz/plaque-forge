CODE ENGINEERING POLICY

## 1. Project mission, documentation, and discoverability
* The implemented functionality should be consistent with the project mission declared in `README.md`.
  * The `README.md` may reference more detailed documents rather than contain all information directly.
  * The `README.md` should remain concise enough to serve as an effective entry point to the project.
* The `README.md` should describe or reference the project's architecture.
* The `README.md` should provide or reference representative examples of what the software can do.
* At minimum, the project's mission, architecture, and representative usage or output examples should be discoverable from the `README.md`.
* The `README.md` should serve both as an informative introduction and as an index into more detailed project documentation.
* Documentation referenced by the `README.md` should be incorporated, directly or indirectly, into `lib.rs` where appropriate, so that the same documentation is available through the generated Rust documentation site.
* Code documentation should follow the conceptual structure of the architecture.
  * Problem-specific terminology and knowledge should be introduced gradually at higher levels of the module hierarchy and become more detailed as the reader moves into deeper modules.
  * Documentation should be concise and precise rather than verbose.
  * The same knowledge should not be unnecessarily documented in multiple places.
  * Information that is expressed more clearly and verifiably by a test should not be duplicated unnecessarily as explanatory prose: documentation should not unnecessarily restate behavior that is already made clearer by executable tests or examples. It should instead add conceptual context, rationale, constraints, or guidance that the tests do not communicate effectively.
  * All usable entities should have appropriate documentation comments so that IDE navigation is effective and the generated Rust documentation forms a useful representation of the public and internally usable system.
    * However, documentation should express what is not already expressed in the name, sub-module, or module of the entity;
    * Of course, if the names are enough, any doc comments would be irrelevant -- therefore shouldn't be added;
    * Refactorings like renaming should be preferred over adding the doc comments -- or should be considered to allow more succinct and expressive doc comments to be written.

## 2. Architecture, layering, and replaceability
* The architecture should be clear and discoverable from the organization of the code.
* It should consider the distinctions:
  * Directory organization: broad component → progressively more detailed submodules.
  * Conceptual organization: entry point → concepts → implementation details.
  * Dependency direction: implementation details should depend on appropriate stable contracts/policies. Specifically, business logic should not depend on infrastructure details.
* The module directory hierarchy should communicate increasingly detailed levels of implementation.
  * The first level of `src/` should expose the core system components and provide obvious entry points into them.
  * Core components may include areas such as domain logic, data access, models, APIs, and other major system responsibilities.
  * Deeper directories should progressively expose the implementation details of those components.
* Higher levels of the module directory hierarchy should emphasize broader concepts and abstractions, while deeper levels should contain increasingly specific implementation details.
* Components should be independently instantiable whenever practical.
  * When independent instantiation is not practical, their required dependencies should be explicit and easily obtainable.
* Components that have independent reasons to vary should be replaceable without requiring unrelated changes elsewhere in the system.
  * Adding or replacing a database driver should not require changes to business logic.
  * Operations exposed through a UI or CLI should also be callable programmatically by other modules when the underlying operation is not inherently tied to that interface.
* Dependency inversion should be used where it improves replaceability and makes architectural boundaries explicit.
  * A convention such as a `module/contract/` sub-module may be used to make these boundaries and dependency contracts readily identifiable.

## 3. TDD, testability, and test infrastructure
* Development should follow the TDD process.
  * Tests should express requirements before the corresponding production behavior is implemented.
  * Tests can be used as documentation for requirements, but -- from the docs -- requirement intentions & project mission should be documented and reachable via the README.md.
  * The design should evolve in response to testability needs, encouraging explicit dependencies and decoupling concerns that have independent reasons to change.
  * Bugs should first have a test, later the corrective implementation should be done.
* Regardless of the TDD process -- which cannot be inferred for existing codebases -- the project should contain the following measurable characteristics:
  * Core components and their meaningful behaviors should be covered by tests -- which should cover a significant portion of the intended functionality of those components.
  * As required by the architecture, components should be independently instantiable where practical, and otherwise have dependencies that are explicit and easily obtainable.
* Dependencies that make tests expensive, non-deterministic, or externally dependent should support stubs/mocks/spy/fake implementation substitutes, or equivalent test implementations.
  * This includes expensive operations with significant CPU or memory requirements.
  * This includes non-deterministic or environment-driven inputs such as keyboards, microphones, cameras, dates, and time.
  * This includes network, filesystem, service, database, or other external accesses.
* Test substitutes should either already exist where they are broadly useful or be straightforward to implement through the component's existing contracts.
* Architectural replaceability should directly support testing.
  * External dependencies should be replaceable by test implementations through the same contracts used by production implementations, without requiring special-purpose changes to production logic.

## 4. Test coverage, regression protection, and behavior
* The observable functionality of the software should be backed by automated tests to a degree proportional to its importance.
  * Functionality demonstrated through homologated outputs should have corresponding automated regression protection.
  * The test suite should support the safe addition of new functionality and modification of existing code.
  * The test suite should make substantial internal refactoring possible with a reasonable expectation that regressions affecting homologated outputs will be detected.
* Tests should primarily verify externally meaningful behavior rather than incidental implementation details.
* The test suite should continue to pass when internal implementation details are changed without altering the intended observable behavior:
  * Mocking should be used to isolate meaningful boundaries, not to encode unnecessary assumptions about internal implementation.
* A high score in this area should indicate strong confidence that externally visible or homologated behavior is preserved across internal changes.

## 5. Code and test organization, duplication, and abstraction
### Production code
* Knowledge should have a single authoritative implementation whenever practical.
* Repeated code that represents the same underlying knowledge, rule, or intention should be generalized rather than duplicated.
* Abstractions should represent genuinely shared concepts.
* Unrelated concepts should not be coupled merely because their implementations currently look similar.
* Avoiding duplicated knowledge should take precedence over eliminating superficial textual duplication.
### Test code
* The same principle of avoiding duplicated knowledge and intention should apply to test code.
* Test infrastructure should be shared when doing so improves maintainability and human readability.
* The desire to keep individual requirements locally understandable should not be used as a blanket justification for duplicating test infrastructure.
* Reusable test abstractions should remove incidental repetition while keeping the behavior, inputs, nuances, and intention of each test locally understandable.
* Keep the distinction of duplicated infrastructure vs duplicated input/output specification. The former should always be abstracted out. The latter, depends. It should not hide the test's intention behind complex layers of abstraction.

## 6. Test architecture, hierarchy, and navigation
* The test suite should exhibit a hierarchy analogous to the layered architecture of the production code.
  * Tests located higher in the conceptual or module hierarchy should exercise broader interactions and therefore behave more like integration tests.
  * Tests located deeper in the hierarchy should exercise increasingly isolated components and therefore behave more like unit tests.
* Test organization should make it possible to navigate from broad functionality toward increasingly specific requirements and implementation details.
* Test functions should preferably be organized around functionality rather than producing a flat collection of unrelated requirement-named functions.
  * A functionality-oriented test may contain multiple related requirements when doing so makes their relationship and hierarchy clearer.
  * Related requirements should remain visibly identifiable within the functionality-oriented test.
* Assertions should make failures diagnostically useful.
  * When multiple requirements are verified within the same test function, assertion messages should make clear which specific requirement was violated.
* The resulting test structure should favor human navigation: broad functionality should be easy to locate, and the individual requirements associated with that functionality should be easy to identify within it.
* Real implementations should be prefered where possible -- even if this would losed the "unit test" aspect. From above: the upper we are in the directory hierarchy, the more the unit tests may look like (sub)integration tests.
