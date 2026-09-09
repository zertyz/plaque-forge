# Definition of Ready and Done

These criteria define state transition gates for work items in Plaque Forge. They formalize [MANAGEMENT.md](file:///root/opencode/plaque-forge/management/MANAGEMENT.md) and guarantee that visual non-regression across video assets is preserved at every stage.


## Ready for Planning

A requirement is ready for planning when:
1. It has a stable requirement ID in [PRODUCT.md](file:///root/opencode/plaque-forge/management/PRODUCT.md), [ENGINEERING.md](file:///root/opencode/plaque-forge/management/ENGINEERING.md), or [OPERATIONS.md](file:///root/opencode/plaque-forge/management/OPERATIONS.md).
2. Its expected visual, mathematical, or operational outcome is explicitly stated.
3. System boundaries, interfaces, and failure modes are defined.
4. Dependencies, model constraints, and potential cross-asset risks are identified.


## Ready to Start

A work item is ready to move from `Planned` to `Started` when:
1. It references a valid requirement ID.
2. Its branch and work-item identifier adhere to management grammar (`RM.HHH.rr.ss-ttt`).
3. Exactly one responsible engineer or team is assigned.
4. The verification path (unit test, synthetic fixture, or homologation contract) is identified.
5. Known blockers are explicitly recorded as blockers, not hidden in descriptions.


## Ready for Code Review

A work item is ready to move to `In Code Review` when:
1. Production implementation satisfies the referenced requirement and work item description.
2. New behavior is backed by automated tests created per [docs/engineering-policy.md](file:///root/opencode/plaque-forge/docs/engineering-policy.md).
3. The branch passes all static checks (`cargo fmt --check`, `cargo clippy`, `cargo test`).
4. The fast continuous homologation sentinel suite ([scripts/check_homologated_assets.sh](file:///root/opencode/plaque-forge/scripts/check_homologated_assets.sh)) passes completely.
5. Any discovered requirement drift is reported rather than silently accommodated.
6. If machine learning models or segmentation heuristics were modified, the multi-scene bake-off script ([scripts/bakeoff_segmentation_matrix.sh](file:///root/opencode/plaque-forge/scripts/bakeoff_segmentation_matrix.sh)) was executed and showed zero boundary regression on baseline masks.


## Ready for QA

A work item is ready for `QA` when:
1. Code review comments affecting observable behavior are resolved.
2. Rendered video outputs or diagnostic artifacts are generated for visual inspection.
3. For visual changes, side-by-side comparisons against baseline renders are provided.


## Ready to Merge

A work item is ready to move to `Merged` when:
1. Code review and applicable QA sign-offs are complete.
2. All CI gates succeed (`check.sh`, `check_analysis_cache.sh`, `check_segmentation.sh`, `check_homologated_assets.sh`).
3. An authoritative row is added or updated in [management/TRACEABILITY.md](file:///root/opencode/plaque-forge/management/TRACEABILITY.md) citing concrete verification evidence.
4. Backlog state trails record all passed states with immutable dates.
5. No release-blocking technical debt or uncontracted visual regressions enter `main`.


## Ready to Roll Out

A work item is ready to move to `Rolled Out` when:
1. The target release version is identified.
2. The full 19-scene homologation matrix ([scripts/run_homologation_matrix.sh](file:///root/opencode/plaque-forge/scripts/run_homologation_matrix.sh)) has executed and passed all dual-witness contracts.
3. Release documentation is recorded in [management/RELEASES.md](file:///root/opencode/plaque-forge/management/RELEASES.md).
4. Release candidate validation passes without regressions.


## Done

A work item is Done when:
1. Its accepted behavior is merged into `main`.
2. All verification evidence is recorded and accessible in [management/TRACEABILITY.md](file:///root/opencode/plaque-forge/management/TRACEABILITY.md).
3. The backlog trail contains the final `Rolled Out` state and date.
4. No undocumented drift remains between requirements, code, tests, and visual assets.
