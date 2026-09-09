# Plaque Forge Governance Management

This project is conducted under a formal management procedure adapted for high-assurance video processing and computer-vision engineering. The top-level rules are:

1) Every development must be backed by formal requirements. Requirements fall into Product, Engineering, or Operations categories and reside in their respective files: [PRODUCT.md](file:///root/opencode/plaque-forge/management/PRODUCT.md), [ENGINEERING.md](file:///root/opencode/plaque-forge/management/ENGINEERING.md), and [OPERATIONS.md](file:///root/opencode/plaque-forge/management/OPERATIONS.md). Those files contain numbered requirements referenced throughout the project and represent authoritative desired behavior.

2) When the software state diverges from requirement files, a synchronization must occur by:
   - Reviewing and updating the requirement (human owner responsibility);
   - Adjusting implementation code;
   - Adding a missing requirement;
   - Removing unauthorized behavior or dead code.
   A `requirement_drift` detection pass may be automated, but requirement updates remain human-governed.

3) Requirements are formally grouped into sections bearing a 3-6 letter abbreviated tag in quotes (e.g., `# Video Overlay Mission "OvlMsn"`), allowing deterministic cross-referencing.

4) Sources of truth:
   - Requirements are authoritative about desired behavior;
   - Unit tests, integration tests, and homologation contracts are evidence of verified behavior;
   - Code in `main` is evidence of actual current behavior; code in topic branches represents behavior under development;
   - Disagreements constitute "requirement drift" (defects, undocumented accepted behavior, incomplete implementation, or obsolete requirements).

5) Planned work items (epics, stories, tasks, bug fixes, technical-debt remediations, spikes) are tracked in [PRODUCT.backlog.md](file:///root/opencode/plaque-forge/management/PRODUCT.backlog.md), [ENGINEERING.backlog.md](file:///root/opencode/plaque-forge/management/ENGINEERING.backlog.md), and [OPERATIONS.backlog.md](file:///root/opencode/plaque-forge/management/OPERATIONS.backlog.md). Unverified intake belongs in [BUGS.md](file:///root/opencode/plaque-forge/management/BUGS.md)'s `## Open Reports`. Validated defects link to `F`-motivation work items and remain open until the corrective work reaches `Rolled Out`.

6) Version control and release quality: `main` contains production-ready code. No known release-blocking technical debt may merge into `main`. Non-blocking debt must be documented in the relevant requirement file with justification and remediation deadline.

7) Work item and branch naming grammar:
   - Topic branches: `<developer>/RM.HHH.rr.ss-ttt`
   - Feature branches: `feature/RM.HHH.rr.ss-ttt`
   - Grammar:
     - `R`: Requirement document — `P` (Product), `E` (Engineering), `O` (Operations);
     - `M`: Motivation — `N` (New), `R` (Revisit/Refactor), `F` (Fix for defect/regression), `X` (Spike/Prototype);
     - `HHH`: Abbreviated section tag from requirements document;
     - `rr`: Requirement number within that section;
     - `ss`: Requirement sub-topic (`a`, `b`, ...);
     - `ttt`: Zero-padded sequence number avoiding collision.

8) Continuous Integration and Rollout:
   - Commits to `main` must pass fast continuous homologation sentinels and unit/integration tests;
   - Semver git tags (e.g. `v0.10.0`) trigger production package releases; release candidate tags (e.g. `v0.10.0-rc.1`) gate manual or staging QA.

9) Work item lifecycle states:
   - Primary path: `Under Planning` → `Planned` → `Started` → `In Code Review` → `Integrated` (feature child branches) → `QA` → `Merged` → `Rolled Out`.
   - Terminal non-happy paths: `Rejected`, `Cancelled`, `Superseded by <ID>`.
   - `Blocked` is an independent condition, not a lifecycle state. Every state change retains an immutable date stamp.

10) Non-Regression Covenant for Video Assets & ML Models:
   - Changing models, updating dependencies, or refactoring algorithms to optimize a specific video scene must NEVER degrade previously homologated video scenes.
   - Every candidate model modification or segmentation heuristic change must execute an automated multi-scene bake-off against golden baseline contracts before merge.
   - Model parameters and prompt strategies must be scene-isolated; global defaults must not silently alter the output of frozen assets.

11) Supporting management ledgers:
   - [BUGS.md](file:///root/opencode/plaque-forge/management/BUGS.md): Defect intake and resolution tracking;
   - [DECISIONS.md](file:///root/opencode/plaque-forge/management/DECISIONS.md): Architectural and algorithmic decisions;
   - [DEFINITION_OF_READY_DONE.md](file:///root/opencode/plaque-forge/management/DEFINITION_OF_READY_DONE.md): State gate criteria and verification rules;
   - [RISKS.md](file:///root/opencode/plaque-forge/management/RISKS.md): Project, hardware, model, and operational risks;
   - [TRACEABILITY.md](file:///root/opencode/plaque-forge/management/TRACEABILITY.md): Canonical graph linking requirements, work items, code, and evidence;
   - [README.ai.md](file:///root/opencode/plaque-forge/management/README.ai.md): Fast navigation index for autonomous agents.
