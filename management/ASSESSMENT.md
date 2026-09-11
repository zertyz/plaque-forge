# Engineering assessment

Repository findings: 2026-09-10 at `7d0ce14aca0440d9f78e8bdd82f7eba97c2a2a72`. Governance conclusions updated 2026-09-11 using [Luiz's decisions](DECISIONS.md). This is an assessment, not a separate policy.

**Adopt requirements-led governance. Enforce it through independent acceptance and protected review authority.** Documents alone cannot stop visual regressions or example-specific solutions.

The objective is repeatable improvement on new videos within an agreed problem domain. The supplied videos illustrate that domain; they do not define a closed list of supported inputs.

## What to take from the references

| Reference | Keep | Adapt for Plaque Forge |
| --- | --- | --- |
| [bot-starter-kit management][starter-management] | Requirements describe desired behavior; backlog items describe changes; implementation, verification, and release are distinct states. | Use concise Markdown and stable references. Its production/staging assumptions and elaborate branch grammar do not automatically apply to this POC. |
| [Requirements discussion][discussion-requirements] | Separate authority from subject: product, engineering, and operations each contain requirements, while policies constrain their evolution. Reports are derived views. | Adopt the information model. Building a management application, GUI, or agent scheduler is outside this task. |
| [Policy discussion][discussion-authority] | Protect policies, acceptance evidence, and the mechanisms that enforce protection. Report conflicts instead of silently choosing a new rule. | Resolve edit authority explicitly. A branch name, commit author, or agent instruction is not an access-control boundary. |
| [Quality-tools discussion][discussion-quality] | Measure architectural dependencies, test effectiveness, duplication, complexity, and the cost of representative changes. | Calibrate tools against this repository. A generic engineering score cannot decide whether a new video renders correctly. |

The conversations contain proposals and revisions, including work on a different management-tool product. They are design references, not evidence that their proposed controls exist here. The policy conversation also contains failed patch-verification claims: completion must refer to the actual checked-out files and actual commands, not a reconstructed substitute.

Reference repository snapshot: `ea541d72055ce69d6450e09bcaff33e4a088a3af`. Useful supporting definitions: [ready/done][starter-ready], [traceability][starter-traceability], and [agent operating map][starter-agents].

## Current foundation

| Area | Repository evidence | Meaning |
| --- | --- | --- |
| Engineering rules | [Consolidated engineering policy](ENGINEERING.md), [agent instructions](../AGENTS.md) | TDD, dependency direction, replaceability, meaningful tests, and preservation of accepted behavior already exist as rules. |
| Product workflow | [README](../README.md), [architecture](../docs/ARCHITECTURE.md) | Analysis is reusable across titles/styles. Rendering consumes analysis; CLI and programmatic entry points share application operations. |
| Acceptance | [Homologation](../docs/HOMOLOGATION.md), [contracts](../assets/homologation/) | Sparse human-reviewed witnesses can protect visible outcomes independently of generated segmentation. |
| Provenance | [Validation](../docs/VALIDATION.md), [CI](../docs/CI.md) | Reports identify the rendered bytes and consumed inputs. Ordinary validation and generated-artifact publication have separate responsibilities. |
| Cheap checks | [check.sh](../scripts/check.sh), [CI workflow](../.github/workflows/ci.yml) | Formatting, Clippy, Rust tests, Python checks, and shell syntax checks are already present. |
| Operational safety | [Safety](../docs/SAFETY.md), [security](../docs/SECURITY.md) | Transactional outputs, bounded cleanup, isolated model setup, and dependency auditing provide useful existing controls. |

These are documented or implemented mechanisms. This assessment did not rerun the full render, ML, or CI suites and does not certify the current outputs.

## Why regressions remain possible

### A. The examples often supply part of the answer

All 19 scenes supply bounds; [candidate selection](../src/analyze/candidate.rs) accepts supplied bounds before automatic discovery. Four scenes supply complete locked trajectories. A successful render using those inputs demonstrates a different capability from discovering and tracking the same surface without them. [Asset assessment](../docs/ASSET_VIDEOS_HUMAN_DEFINITIONS.md#final-considerations).

Supplied bounds also receive fixed confidence values in candidate selection. Those values describe an implementation convention, not a measured probability that automatic discovery succeeded.

**Measure surface selection, motion estimation, foreground estimation, and rendering separately.** Record the human assistance each evaluation consumes. A cache refresh that succeeds by adding a dense trajectory must not be reported as an automatic-tracking improvement.

### B. Recorded acceptance and continuous coverage differ

The [capability inventory](../assets/homologation/capabilities.toml) lists 10 capabilities: 8 have contracts and 5 have continuous render sentinels. Multi-layer foreground/shadow composition and background motion independent of the plaque lack contracts. Cloud and landscape-spider contracts contain visual witnesses but are outside the five-case [continuous script](../scripts/check_homologated_assets.sh).

The landscape wooden-plaque contract has no foreground witnesses. Its existence does not establish protection of its vegetation/lizard interactions. The separate [segmentation inventory](../assets/homologation/segmentation-capabilities.toml) records 6 capabilities, 4 represented and only 1 linked to final homologation; its parallax entry does not link the available chains contract.

**Track capability, accepted evidence, and execution coverage separately.** Eleven videos without individual contracts do not imply eleven distinct untested capabilities. Conversely, a green representative suite does not certify every example or every capability combination.

### C. Some scene declarations have surprising consumers

| Finding | Consequence | Required disposition |
| --- | --- | --- |
| Imported `active_frames` does not independently limit the packaged runtime layer. | Shortening the scene interval can leave effective pixels unchanged. | Implement the conflict rule in P-003 through WI-016. |
| Imported artifact metadata supplies `affects_layout`. | A scene-only change can appear effective while leaving layout unchanged. | Reject effective disagreement under P-003. |
| `in_front_of` names a surface; it is not a general depth graph. | Richer ordering cannot be inferred from the field name. | Specify the approved expanded behavior and implement it through WI-021/WI-038. |
| A writing-surface mask defines usable material; it does not create mesh-based text deformation. | A “deforming” example can overstate demonstrated motion capability. | Distinguish changing support from deformation of the typography itself. |

Sources: [scene schema](../src/scene.rs), [layer consumers](../src/layers.rs), [prior findings](../docs/ASSET_VIDEOS_HUMAN_DEFINITIONS.md#confirmed-implementation-limitations-relevant-to-editing-the-data). Current timing/layout values are coherent with their supplied pixels; these findings do not prove visible defects in the present examples.

### D. Protection must include the acceptance mechanism

There is no tracked `CODEOWNERS` file or dedicated protected-authority diff guard in this snapshot. Live GitHub rulesets and credential permissions were not inspected. Luiz has since confirmed that agents use his personal Git account: account identity alone cannot distinguish his approval from an agent action. [Q-012](DECISIONS.md#resolved-questions).

A candidate change can weaken acceptance through thresholds, masks, test selection, skips, workflow filters, verifier code, or generated baselines. Protecting only a Markdown policy leaves those routes open.

**The implementation author must not control the authority that accepts the implementation.** Protect the registry, approval path, gate definitions, evidence selection, and acceptance changes. Test candidate code without giving it authority to replace the expected result or publish its own approval.

GitHub supports required reviews/checks and code ownership, but these require configured repository rules. Protect `CODEOWNERS` itself. Separate contributor credentials from approval/bypass credentials. A trusted guard must inspect candidate changes as data; running candidate build scripts under privileged `pull_request_target` permissions defeats that separation. [Rulesets][github-rules], [code owners][github-owners], [trusted workflow guidance][github-target].

### E. Existing prose contains authority drift

[SECURITY.md](../docs/SECURITY.md) and [the audit helper](../scripts/audit_dependencies.sh) describe a pinned `cargo-audit` version. [CI](../.github/workflows/ci.yml) installs it without `--version`. That is a confirmed discrepancy. [Q-013](DECISIONS.md#resolved-questions) now requires mismatches and dependency failures to stop with actionable remediation; implementation is WI-030.

Policies also coexist with imperative statements in architecture, validation, safety, segmentation, and workflow documentation. Consolidation must identify the authoritative statement for each obligation, then make other documents reference it. Merely adding another policy layer increases the ambiguity.

### F. Source-bound human data needs its own lifecycle

The prior assessment found limited review provenance for dense trajectories and historical imported masks, unusually templated spider prompts, and a very short hologram-opacity transition that merits visual review. Source inspection supports a real flicker; changing the opacity simply because its numbers look odd would be unjustified.

Source bytes, frame coordinates, geometry, masks, and trajectories form one review context. Replacing a source at the same filename must trigger dependency review. Historical annotations need honest provenance; missing review history must not be invented. [Detailed findings](../docs/ASSET_VIDEOS_HUMAN_DEFINITIONS.md#suspicious-or-review-worthy-choices-with-the-evidence-qualified).

### G. Hardcoding needs a broader definition than filenames

A search for the supplied videos' literal stems found no matches in Rust/Python source files. A broader motif search found comments and tests. This is a limited inspection, not proof against specialization.

Specialization can also hide in source hashes, dimensions, frame intervals, tuned defaults, model-selection rules, or increasingly complete per-video answers. Those values may also be legitimate inputs or generic constraints. Review their purpose and evidence; do not ban every constant or scene definition.

**A fix must explain the general failure mechanism and demonstrate it beyond the triggering example.** Renaming a video must not change algorithm selection; changing real content may. New examples should exercise combinations of motion, shape, lighting, occlusion, and typography rather than only reproducing the existing compositions.

## Acceptance assessment

Luiz's answers identify an additional failure mechanism: the accepted visual result and the current algorithm's ability to reconstruct it were insufficiently separated. A sparse correction set can stop working after an analysis change even when the former result remains the desired target.

The [acceptance protocol](ACCEPTANCE.md) therefore freezes complete resolved scene data and lossless reference renders, then evaluates renderer replay, fresh analysis with existing inputs, and proposed remapping separately. Intrinsic quality, fidelity, and human correction effort answer different questions.

This supports useful evolution without silently rewriting the target or hiding increased assistance. Independent witnesses and deliberately broken cases still matter: a renderer can faithfully reproduce a poor reference, and a reconstruction can appear automatic while consuming a stored answer.

Useful generalization probes include renamed inputs, transformed geometry, different frame rates, changed title lengths, partial offscreen motion, occlusion entry/exit, and independently moving backgrounds. Each probe needs an explicit expected relationship and valid transformation of its annotations. Arbitrary augmentation is not automatically a valid oracle.

Full-frame pixel comparisons must enforce the agreed absence of visual drift; artifact hashes separately bind reports to exact bytes. Numerical allowances need calibration and cannot conceal visible changes. No finite suite establishes correctness for every possible future video.

## Quality measurement and enforcement

These are candidates for later work, not installed tools or measured results.

| Concern | Candidate mechanism | How to use it |
| --- | --- | --- |
| Basic code health | Existing fmt/Clippy/tests | Keep enforced; do not confuse passing them with visual acceptance. |
| Architectural dependencies | [arch-lint][arch-lint], explicit dependency tests | First define allowed dependencies. Prove forbidden imports/cycles are detected, including the project's relevant features and generated code. |
| Test reach | [cargo-llvm-cov][llvm-cov] | Locate unexercised behavior. Branch coverage has additional toolchain requirements; choose a supported profile explicitly. |
| Test effectiveness | [cargo-mutants][mutants] and deliberate visual faults | Examine survivors. Separate caught, missed, unviable, and timed-out mutations; a timeout is not evidence that an assertion detected the defect. |
| Textual duplication | [jscpd][jscpd] | Find clone candidates in production and test code. Review shared knowledge before introducing abstractions. |
| Complexity | [rust-code-analysis][rust-analysis] | Review difficult, frequently changed functions. Track local changes; repository averages can hide concentrated problems. |
| Coupling/change impact | Dependency inspection, Git history, optional [cargo-coupling][coupling] | Evaluate representative changes and unexpected affected components. The named tool is experimental; its score should not become an unreviewed quality threshold. |
| Dependency risk | Existing RustSec gate; evaluate complementary checks only for identified gaps | Pin scanner behavior, record advisory-data identity, and require human-reviewed exceptions. |
| Governance integrity | Stable IDs, references, protected diffs, trusted approval evidence | Reject missing references, unauthorized authority changes, weakened gate configuration, and completion without evidence. |

Start new metrics as observations. Ratify baselines, exclusions, tool versions, and blocking criteria after measuring cost and false positives. An implementation change must not alter those criteria to pass. AI review can identify suspected drift; deterministic checks establish only what their specified mechanisms actually test.

Architecture should be tested against concrete changes: add an unfamiliar video without source-identity dispatch; replace a segmentation backend through its boundary; add a text effect without rebuilding analysis; replace an encoder without changing title semantics; exercise application operations without the CLI. Small edit counts alone do not prove good design.

## Order of work

The [management entry point](README.md) gives the current sequence; domain backlogs retain the earlier asset-assessment suggestions and the new acceptance/capability work. Establish trusted acceptance before broad algorithm, model, or architecture changes. The current code is the implementation to assess, not the specification to canonize.

[starter-management]: https://github.com/zertyz/bot-starter-kit/blob/ea541d72055ce69d6450e09bcaff33e4a088a3af/management/MANAGEMENT.md
[starter-ready]: https://github.com/zertyz/bot-starter-kit/blob/ea541d72055ce69d6450e09bcaff33e4a088a3af/management/DEFINITION_OF_READY_DONE.md
[starter-traceability]: https://github.com/zertyz/bot-starter-kit/blob/ea541d72055ce69d6450e09bcaff33e4a088a3af/management/TRACEABILITY.md
[starter-agents]: https://github.com/zertyz/bot-starter-kit/blob/ea541d72055ce69d6450e09bcaff33e4a088a3af/management/README.ai.md
[discussion-requirements]: https://chatgpt.com/share/6aa323ff-0038-83e9-8965-0733c6db9d78
[discussion-authority]: https://chatgpt.com/share/6aa32442-9df4-83e9-b943-9069e9e892c5
[discussion-quality]: https://chatgpt.com/share/6aa32382-3714-83e9-a6fc-b61cbb46e56b
[github-rules]: https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/available-rules-for-rulesets
[github-owners]: https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/about-code-owners
[github-target]: https://docs.github.com/en/actions/reference/security/securely-using-pull_request_target
[arch-lint]: https://github.com/ynishi/arch-lint
[llvm-cov]: https://github.com/taiki-e/cargo-llvm-cov
[mutants]: https://mutants.rs/using-results.html
[jscpd]: https://github.com/kucherenko/jscpd/blob/master/docs/rust.md
[rust-analysis]: https://github.com/mozilla/rust-code-analysis
[coupling]: https://github.com/nwiizo/cargo-coupling
