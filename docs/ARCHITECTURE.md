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

### Application API, CLI, and workflow orchestration

- `main.rs` is a thin executable entry point.
- `lib.rs` owns command dispatch and the single module graph used by the executable and library tests.
- `application.rs` is the programmatic API for the major analyze/render/verify/homologate workflows. Its request types are interface-independent. `ApplicationServices` lets tests or embedding programs replace supported external-command boundaries without impersonating a CLI.
- `cli.rs` is an adapter: it parses `clap` arguments and translates them into application requests. Core analyze/render/verify/homologate workflows do not consume CLI argument types.
- `analyze/`, `render/`, `verify/`, `review.rs`, `scene_commands.rs`, and `segmentation.rs` implement application workflows.

These modules coordinate operations. They should not contain asset-specific scene data.

### Scene and domain model

- `model.rs` contains geometry-independent data exchanged across workflows.
- `geometry.rs` contains projective geometry helpers.
- `scene.rs` defines the strict scene/trajectory/layer contracts, normalized human coordinates, and provenance. Writable regions are domain-invariant subsets of their tracked surface.
- `analysis.rs` defines the generated cache schema.
- `layers.rs` resolves and packages authored scene layers. Layout influence, tracking influence, and foreground matte semantics are independent contracts rather than accidental consequences of declaring a layer.

### Image and video implementation

- `surface.rs`, `image_io.rs`, and `color.rs` provide image primitives.
- `digest.rs` owns generic streaming content hashes used for cache/provenance identity.
- `portable_path.rs` is the only serialized-path boundary. Generated project references are relative, slash-separated, and reject workstation roots; bundle-local references cannot escape their owner.
- `analyze/candidate.rs` proposes plausible writing-surface enclosures. Selection preserves the strongest surface hypothesis by default, rescues clear compact plaques from broad architectural enclosures, and uses guarded area dominance only to escape small high-contrast props such as magnifying glasses.
- `analyze/tracking/` estimates a physical four-corner trajectory across modular submodules:
  - `tracking/types.rs`: Motion models, tracking result representations, and screen-fixed fallbacks.
  - `tracking/constraints.rs`: Quad transformations, keyframe constraints, and trajectory loop closure.
  - `tracking/mod.rs`: Feature tracking, rigid-plane flow integration, and projective homography solving.
- `analyze/extraction.rs` recovers canonical source-underlay and structural data used by source surfaces and foreground analysis.
- `analyze/occlusion.rs` estimates automatic foreground occlusion. When a crossing benefits from semantic scene, `segmentation.rs` can automatically sharpen those masks through the replaceable Python worker.
- `render/typography.rs` shapes and fits text and owns line-layout decisions.
- `render/effects/` paints mask-derived text effects such as stroke, glow, and shadow across modular submodules:
  - `effects/filters.rs`: Disk dilation and basic morphology filters.
  - `effects/advanced.rs`: Arc warping and perspective deformation transforms.
  - `effects/mod.rs`: Multi-layer style composition, drop shadow, neon glow, and shader rendering.

Text effects are intentionally split by the data they operate on. Layout/glyph transforms, mask effects, material/fill effects, and plaque-surface effects are separate extension points; see [TEXT_EFFECTS.md](TEXT_EFFECTS.md).

OpenCV and `cosmic-text` are implementation libraries inside these layers. They are not part of the scene or analysis-cache interfaces.

## Surface sources

A writing surface has two orthogonal properties: **placement/pose** and **writable mask**. Its visual source is either already present in the video or injected from a transparent image. Injected surfaces skip automatic plaque selection because their placement is explicit, but they still reuse motion/underlay/foreground analysis so scene objects can cross in front. Their PNG alpha controls plaque compositing, not title writability: the declared writable region or inset remains authoritative even for glass and holographic interiors. The injected image hash is part of scene/cache provenance.

## External processes

External executables are kept behind explicit boundaries:

1. `infrastructure.rs` contains deliberately small replaceable external-process contracts. The FFprobe path uses the production `CommandExecutor` contract and can be replaced by a deterministic test implementation without launching a process.
2. `video.rs` owns FFmpeg/FFprobe video semantics and streaming encode/decode for Rust workflows.
3. `segmentation.rs` owns the versioned segmentation-worker protocol. Rust decides when authored or automatic foreground segmentation is useful; `tools/segmentation_worker.py` exists only because the supported ML ecosystems are Python-native. `tools/segmentation_runtime.py` is the lightweight shared contract for pinned model revisions and deterministic SAM2 checkpoint resolution, used by both setup and the worker. Worker implementation, prompt/seed-mask content, source, runtime, backend, model revision, and requested device participate in generated-cache identity. Generic scene layer artifacts may carry partial historical/authored generator metadata; complete worker provenance is required at the segmentation acceptance/cache boundary rather than being falsely invented for reviewed static assets. Frames and masks cross this boundary as lossless PNG, including soft alpha; Rust validates every generated output image, frame range, coverage, and identity before accepting it. A replacement analysis may copy a prior automatic sequence into its private stage only after this complete validation succeeds.

Shell scripts do not implement scene-analysis algorithms. They build the Rust binary, choose assets, translate convenient command-line/environment settings, and invoke high-level commands.

## Cache compatibility

Analysis cache compatibility is explicit in `build_info::ANALYZER_CACHE_VERSION` and `analysis::ANALYSIS_FORMAT`.

- Change `ANALYSIS_FORMAT` when the serialized schema itself becomes incompatible.
- Change `ANALYZER_CACHE_VERSION` when analysis semantics change such that an old cache must not be reused, even if its schema still parses.
- Do not invalidate analysis caches for renderer-only, CLI-only, documentation, or unrelated refactoring changes.

No custom `build.rs` source hashing is used. Renderer-only text effects and styles therefore do not invalidate scene analysis.

Generated manifests never contain absolute paths. A format or semantic analyzer
identity mismatch requires a real rebuild; no migration command can relabel stale
geometry, depth, or model output as current.

## Artifact lifecycle

Authored intent and generated state are deliberately separate:

```text
assets/*.mp4 + assets/scenes/ + assets/plaques/   authored/project inputs
assets/analysis/                                      complete generated caches only
output/                                               complete published render bundles
/tmp/plaque-forge/work/                               in-progress transactional work
/tmp/plaque-forge/failures/                           compact bounded failure evidence
/tmp/plaque-forge-python/                             optional disposable ML runtime/cache
```

Analysis/segmentation stages are RAII-owned and committed by rename when possible. Render publishes video, text mask, decision trace, optional contact sheet, and manifest as one recoverable file bundle with the manifest last. The manifest hashes the decision trace; verification and homologation reject a missing, changed, or provenance-inconsistent trace. Stale work is reaped after 24 hours; successful analysis purges that asset's retained failures.

## Human diagnostics

Machine-readable JSON remains canonical for automated validation. `review.rs` is a presentation layer that accepts complete analyses or compact retained failure evidence and turns metrics, candidate alternatives, scene intent, and diagnostic imagery into prioritized `review.html`/`review.txt` triage. Its browser-only coordinate helper emits human-copyable normalized points; it does not alter caches or analysis decisions.

## Color and alpha contract

Decoded frames stay in encoded pixel coordinates (`-noautorotate` in FFmpeg and disabled OpenCV autorotation), while display rotation is carried as output metadata. Declared SDR color range/space/transfer/primaries are copied to rendered video. HDR PQ/HLG and BT.2020 input currently fail explicitly because the compositor is intentionally 8-bit SDR.

Raster resampling and source-over compositing operate in linear light with premultiplied alpha. This prevents dark/color fringes around translucent plaque edges. Foreground restoration and verification both honor soft alpha rather than reducing mattes to binary cutouts.


## Render decision trace

Every new render emits `<output>.decision-trace.json`. It records the causal choices that are
otherwise difficult to infer from pixels alone: selected surface and selection reason, canonical
plane geometry, trajectory model and authored keyframe counts, foreground layers excluded from
tracking, typography resolution, and compositing-layer matte/layout/tracking semantics. The trace
is diagnostic evidence, not an independent source of truth: its SHA-256 is pinned by the render
manifest and its source/analysis/render identities are cross-checked whenever the manifest is
verified or homologated. Human review pages surface the same trace.
