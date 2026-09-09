# Architectural & Engineering Decisions

Accepted architectural and engineering decisions are recorded here under version control. If a decision is superseded, append the new decision and mark the prior decision as superseded rather than deleting history.


## DEC-0001 -- Requirements and Backlogs Are Git-Controlled Management Sources

Status: Accepted
Date: 2026-09-09

Context:
Plaque Forge is transitioning from an exploratory proof-of-concept into a formally engineered computer vision product. Clear separation is required between desired behavior, planned work, actual code, and verification evidence.

Decision:
Desired behavior is governed exclusively by [PRODUCT.md](file:///root/opencode/plaque-forge/management/PRODUCT.md), [ENGINEERING.md](file:///root/opencode/plaque-forge/management/ENGINEERING.md), and [OPERATIONS.md](file:///root/opencode/plaque-forge/management/OPERATIONS.md). Planned work is governed by matching `.backlog.md` files. Discrepancies represent requirement drift and must be explicitly reconciled.

Consequences:
Automated tools and agents may detect requirement drift, but only human owners may update authoritative requirements.

Related:
[MANAGEMENT.md](file:///root/opencode/plaque-forge/management/MANAGEMENT.md), [PRODUCT.md](file:///root/opencode/plaque-forge/management/PRODUCT.md), [ENGINEERING.md](file:///root/opencode/plaque-forge/management/ENGINEERING.md), [OPERATIONS.md](file:///root/opencode/plaque-forge/management/OPERATIONS.md)


## DEC-0002 -- Dual-Witness Homologation Verification Standard

Status: Accepted
Date: 2026-06-15

Context:
Video encoding (HEVC / H.264) produces non-deterministic byte variations across different FFmpeg versions, CPU threading, and hardware encoders. Brittle whole-video byte-equality assertions fail on benign encoder updates.

Decision:
Homologation verification uses dual-witness objective testing:
1. `source_preservation`: Verifies that pixels outside the plaque bounding box remain unaltered relative to source footage within empirical encoder noise thresholds.
2. `title_visibility`: Verifies that text pixels inside the plaque maintain sharp contrast, continuous presence, and readable typography.

Consequences:
Video renders can be verified objectively and reliably across diverse environments without brittle MD5/SHA256 checksums of compressed video containers.

Related:
[src/homologation.rs](file:///root/opencode/plaque-forge/src/homologation.rs), [assets/homologation/](file:///root/opencode/plaque-forge/assets/homologation), `E.Hom.01`


## DEC-0003 -- External Python ML Worker Isolation via JSON-Lines IPC

Status: Accepted
Date: 2026-05-20

Context:
Modern vision foundation models (SAM 2, Florence-2) are written in Python and PyTorch with complex CUDA and C++ extension dependencies (`sam2._C`). Directly embedding Python via FFI (e.g. PyO3) into the Rust core risks process memory corruption, CUDA driver conflicts, and GIL contention.

Decision:
The Rust core delegates segmentation to an external Python subprocess running [tools/segmentation_worker.py](file:///root/opencode/plaque-forge/tools/segmentation_worker.py) communicating over standard I/O pipes using the versioned JSON-lines protocol `plaque-forge.segmentation-request/2`.

Consequences:
The Rust engine remains memory-safe and isolated. If a segmentation model exhausts VRAM or crashes, the Rust engine captures the error gracefully.

Related:
[src/segmentation.rs](file:///root/opencode/plaque-forge/src/segmentation.rs), [tools/segmentation_worker.py](file:///root/opencode/plaque-forge/tools/segmentation_worker.py), `E.Seg.01`


## DEC-0004 -- Lease-Held Atomic Staging for All Output Artifacts

Status: Accepted
Date: 2026-05-15

Context:
Video rendering and multi-frame analysis operations take significant time and write large files. A crash, interrupt, or out-of-disk condition mid-render can leave truncated MKV files or corrupted analysis caches.

Decision:
All file writing is routed through [src/staged_output.rs](file:///root/opencode/plaque-forge/src/staged_output.rs), which holds an advisory file lock, writes to a temporary sibling file, and performs an atomic filesystem replace only upon full pipeline success.

Consequences:
Corrupt, zero-byte, or partial files are never left in destination directories.

Related:
[src/staged_output.rs](file:///root/opencode/plaque-forge/src/staged_output.rs), `E.Arch.03`


## DEC-0005 -- Behavioral Capability Sentinels for Fast Continuous Integration

Status: Accepted
Date: 2026-07-01

Context:
Rendering all 19 video scenes and executing full dual-witness checks takes over 30 minutes on standard developer machines, making per-commit CI feedback unacceptably slow.

Decision:
CI executes a fast sentinel suite ([scripts/check_homologated_assets.sh](file:///root/opencode/plaque-forge/scripts/check_homologated_assets.sh)) consisting of 5 behaviorally diverse scenes covering planar tracking, moving perspective, foreground occlusions, portrait geometry, and temporary retraction. The full 19-scene matrix is reserved for nightly and pre-release gates.

Consequences:
Developers receive rapid (<5 min) PR feedback while maintaining coverage of all distinct behavioral modes.

Related:
[assets/homologation/capabilities.toml](file:///root/opencode/plaque-forge/assets/homologation/capabilities.toml), `E.Hom.03`, `O.CiCd.01`


## DEC-0006 -- Immutable Commit Pinning for Machine Learning Models

Status: Accepted
Date: 2026-06-05

Context:
Hugging Face model repositories frequently update weights or code without warning. If Plaque Forge loads unpinned models, silent weight drift can subtly shift segmentation masks.

Decision:
All foundation models must be pinned to explicit git commit revisions in [tools/segmentation_runtime.py](file:///root/opencode/plaque-forge/tools/segmentation_runtime.py). Dynamic querying of unpinned refs is prohibited in production.

Consequences:
Segmentation outputs are reproducible across time and isolated from upstream repository updates.

Related:
[tools/segmentation_runtime.py](file:///root/opencode/plaque-forge/tools/segmentation_runtime.py), `E.Seg.02`


## DEC-0007 -- Per-Scene Model & Hyperparameter Isolation to Prevent Cross-Video Regressions

Status: Accepted
Date: 2026-09-09

Context:
Tuning segmentation models, prompts, or post-processing dilation filters to solve edge cases in one challenging video frequently broke existing, previously approved videos due to shared global heuristics.

Decision:
Segmentation models, prompt configurations, and post-processing filters must be explicitly declared and scoped per scene in `scene.toml`. Global shared heuristic mutations that alter default behavior across all scenes are strictly prohibited. Any proposed global model upgrade must pass a multi-asset bake-off against all frozen baselines before adoption.

Consequences:
Optimizing a difficult video is safely partitioned to that video's configuration, protecting all previously approved videos from accidental regression.

Related:
[management/BUGS.md](file:///root/opencode/plaque-forge/management/BUGS.md), `P.AstNrg.01`, `P.AstNrg.02`, `E.Seg.03`, `E.Seg.04`, `EF.Seg.03-001`
