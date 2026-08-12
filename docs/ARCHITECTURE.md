# Architecture

## Pipeline

```text
source video + optional injected plaque image
    |
    v
analyze ----------------------------------------------------+
  resolve writing-surface source (detected/manual/injected)   |
  track placement / scene anchor                              |
  recover source underlay + writable mask                     |
  detect/import foreground layers                             |
    |                                                        |
    v                                                        |
assets/analysis/<name>/                                      |
    |                                                        |
    +-------------------------> render <----- title + font ---+
                                  |
                                  v
                              output video
                                  |
                                  v
                                verify
```

Analysis is intentionally reusable. Rendering must not rebuild or delete analysis data.

## Rust layers

### CLI and workflow orchestration

- `main.rs` is a thin executable entry point.
- `lib.rs` owns command dispatch and the single module graph used by the executable and library tests.
- `cli.rs` defines user-facing arguments.
- `analyze/`, `render/`, `verify/`, `review.rs`, `refinement_commands.rs`, and `segmentation.rs` implement application workflows.

These modules coordinate operations. They should not contain asset-specific scene data.

### Scene and domain model

- `model.rs` contains geometry-independent data exchanged across workflows.
- `geometry.rs` contains projective geometry helpers.
- `refinement.rs` defines sparse reviewed-input schemas, normalized human coordinates, legacy compatibility, and provenance.
- `analysis.rs` defines the generated cache schema.
- `layers.rs` resolves and packages authored scene layers.

### Image and video implementation

- `surface.rs`, `image_io.rs`, and `color.rs` provide image primitives.
- `digest.rs` owns generic streaming content hashes used for cache/provenance identity.
- `analyze/candidate.rs` proposes plausible writing-surface enclosures. Selection preserves the strongest surface hypothesis by default, rescues clear compact plaques from broad architectural enclosures, and uses guarded area dominance only to escape small high-contrast props such as magnifying glasses.
- `analyze/tracking.rs` estimates placement/scene motion and supports authoritative screen-fixed trajectories.
- `analyze/extraction.rs` recovers canonical source-underlay and structural data used by source surfaces and foreground analysis.
- `analyze/occlusion.rs` estimates automatic foreground occlusion. When a crossing benefits from semantic refinement, `segmentation.rs` can automatically sharpen those masks through the replaceable Python worker.
- `render/typography.rs` shapes and fits text and owns line-layout decisions.
- `render/effects.rs` paints mask-derived text effects such as stroke, glow, and shadow.

Text effects are intentionally split by the data they operate on. Layout/glyph transforms, mask effects, material/fill effects, and plaque-surface effects are separate extension points; see [TEXT_EFFECTS.md](TEXT_EFFECTS.md).

OpenCV and `cosmic-text` are implementation libraries inside these layers. They are not part of the refinement or analysis-cache interfaces.

## Surface sources

A writing surface has two orthogonal properties: **placement/pose** and **writable mask**. Its visual source is either already present in the video or injected from a transparent image. Injected surfaces skip automatic plaque selection because their placement is explicit, but they still reuse motion/underlay/foreground analysis so scene objects can cross in front. The injected image hash is part of refinement/cache provenance.

## External processes

External executables are kept behind two boundaries:

1. `video.rs` owns FFmpeg/FFprobe process invocation for Rust workflows.
2. `segmentation.rs` owns the versioned segmentation-worker protocol. Rust decides when authored or automatic foreground segmentation is useful; `tools/segmentation_worker.py` exists only because the supported ML ecosystems are Python-native. Worker/backend/model/device configuration participates in analysis cache provenance.

Shell scripts do not implement scene-analysis algorithms. They build the Rust binary, choose assets, translate convenient command-line/environment settings, and invoke high-level commands.

## Cache compatibility

Analysis cache compatibility is explicit in `build_info::ANALYZER_CACHE_VERSION` and `analysis::ANALYSIS_SCHEMA_VERSION`.

- Change `ANALYSIS_SCHEMA_VERSION` when the serialized schema itself becomes incompatible.
- Change `ANALYZER_CACHE_VERSION` when analysis semantics change such that an old cache must not be reused, even if its schema still parses.
- Do not invalidate analysis caches for renderer-only, CLI-only, documentation, or unrelated refactoring changes.

No custom `build.rs` source hashing is used. Renderer-only text effects and styles therefore do not invalidate scene analysis.

## Human diagnostics

Machine-readable JSON remains canonical for automated validation. `review.rs` is a presentation layer that also accepts failed partial analyses and turns metrics, candidate alternatives, refinement intent, and diagnostic imagery into prioritized `review.html`/`review.txt` triage. Its browser-only coordinate helper emits human-copyable normalized points; it does not alter caches or analysis decisions.
