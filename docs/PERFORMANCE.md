# Performance assessment

Performance work is intentionally split from outcome-quality work. This pass audited every expensive boundary but changed only implementations whose semantics are equivalent and easy to verify.

## Cheap improvements already applied

- Static title presentation is prepared once per render. Font discovery, shaping, line breaking, material painting, and static transforms are not repeated per frame.
- A full source-frame clone is skipped when there are no foreground masks to restore.
- Same-filesystem staged directories are committed by rename. Cross-filesystem publication copies only once into a destination-side incoming path.
- Linear/sRGB conversion tables are initialized once and reused by compositing.
- Alpha-aware verifier channel bounds are precomputed in a 65,536-entry table; exhaustive validation does not redo transfer-function math for every channel/pixel.
- Diagnostic frame storage remains capped at 12 evenly spaced frames.
- Hashing streams files in bounded blocks; it does not load videos into memory.
- Typography separates font-aware measurement and alpha-only fit probes from final
  material painting. Candidate search counts actual shaped advances (including spaces,
  kerning, and punctuation); only the selected layout pays for gold/bevel/blur paint.
- A forced scene rebuild reuses an existing automatic-foreground ML sequence only when
  its complete portable request identity and every lossless output validate. Scene-
  only iterations no longer initialize SAM/Cutie/VitMatte for an identical request.
- Projective warping and canonical extraction share one fused resampling kernel, and
  large warps parallelize across destination rows. Every output pixel depends only on
  its own inputs, so any worker count produces bitwise-identical frames (pinned by
  serial-versus-threaded equivalence tests).
- Foreground restoration copies fully opaque source pixels instead of re-running
  linear-light blending; the underlying encoded-level round trip is pinned as an
  exact identity by a test.
- Verification dispatches its two independent ECC registrations (structural surface
   and rendered title plane) to resident worker threads that overlap the frame
   pipeline. Jobs are pure functions of their inputs and results are consumed in
   dispatch order, so every report value matches inline evaluation exactly.
- The frame-invariant registration-support denominator is computed once per
   verification instead of once per frame, and flow-history frames are shared through
   reference-counted surfaces rather than deep-copied.
- Large surface ops shard by rows: `blend_surface`, `restore_from_mask`, `apply_alpha_mask`, and `box_blur` horizontal pass parallelize via `parallel_workers` + `run_rows` (or row-banded `std::thread::scope`). Each row is independent, so any worker count is bitwise-identical (existing warp/extract tests pin the invariant). Vertical blur remains sequential (column-sharded raw-pointer variant deferred).
- Shipping profile uses `lto = "fat"`, `opt-level = 3`, `strip`, `target-cpu=native` (`RUSTFLAGS="-C target-cpu=native"`, `CFLAGS/CXXFLAGS="-O3 -march=native -mtune=native"`), `codegen-units = 1`.

These changes preserve the same frame count, geometry, effect timing, masks, and artifact identities. Linear-light, premultiplied-alpha compositing is a separate correctness fix, not presented as a speed optimization.

## Full-path cost review

| Area | Current cost | Decision for this phase |
| --- | --- | --- |
| FFprobe | `-count_packets` scans packet metadata to avoid stale frame-count claims | Keep: replacing it needs corpus-level timing correctness tests |
| Rust decode/render | One RGBA allocation/read per frame; animated styles may allocate transformed layers | Keep: pooling/tiling changes lifetime and aliasing risk |
| Projective warp | Scalar bilinear sampling with linear-light/premultiplied-alpha math | Keep: SIMD/GPU paths need pixel-equivalence tolerances |
| Verification | Decodes source and render and checks every pixel/frame | Keep: sampling would weaken the acceptance gate |
| Analysis | Several sampled and full-frame passes for candidate, tracking, extraction, and occlusion | Keep: pass fusion changes algorithm ordering and diagnostics |
| ML frame transport | Lossless RGBA PNG sequence under temporary storage | Keep: requested quality and alpha correctness outweigh added I/O |
| SAM/Cutie/Matte | Heavy model construction and accelerator initialization | Resident single-request service + content-addressed stage caches; source-identity changes select fresh state |
| Asset batches | Sequential by default | Keep: unbounded parallel FFmpeg/ML can exhaust RAM, VRAM, or disk bandwidth |

## Deferred performance work

The next performance phase should use the segmentation bake-off and per-stage execution reports before changing algorithms. Remaining candidates include reusable Rust render buffers, SIMD/GPU warp/composite kernels with golden-frame tests, direct lossless frame streaming that models can consume without PNG transport, and resource-aware parallel asset scheduling. The bounded long-lived ML service is now implemented; it remains intentionally sequential to avoid GPU-memory races.

Do not optimize verification by skipping frames or pixels. Its exhaustive nature is part of the quality contract.

## Measured baseline (2026-08-12)

The hardened release build was measured on an Intel Core Ultra 7 255H (16 logical CPUs), Rust 1.97.1, and FFmpeg 8.1.2. These figures are an indicative audit record, not a cross-machine promise:

| Workload | Wall time | Peak RSS | Result |
| --- | ---: | ---: | --- |
| Incremental release build with thin LTO | 37.84 s | 1,034 MiB | passed |
| Lossless render, 1280×720, 240 frames | 16.90 s | 312 MiB | completed |
| Exhaustive verification of that render | 37.36 s | 347 MiB | all component scores 1.0 |
| Lossless render, 720×1280, 192 frames | 4.28 s | 260 MiB | completed |
| Exhaustive verification of that render | 11.94 s | 349 MiB | all component scores 1.0 |

The verifier numbers include every frame and pixel. The only verifier optimization in this pass was replacing repeated transfer-function evaluation with an exact precomputed alpha/channel-bound table; acceptance semantics were unchanged. Temporary benchmark outputs were removed after validation.

## Measured improvement (2026-08-23)

Interleaved before/after runs of the same binary pair on one machine (16 logical CPUs, Rust 1.95, FFmpeg 8.1.1, OpenCV 5.0.0; unprivileged scheduling with CPU pinning; `sync` before every timed run; minimum of 3 rounds). Workloads: lossless `gold-shine` render plus exhaustive verification of that same render, long canonical title text.

| Workload (min of 3) | Before | After | Change |
| --- | ---: | ---: | --- |
| Render, physical plaque with foreground restoration, 1280×720×240 | 5.43 s | 4.49 s | −17% |
| Render, injected plaque surface, 1280×720×240 | 9.30 s | 5.33 s | −43% |
| Verify, physical plaque with foreground restoration | 66.92 s | 59.72 s | −11% |
| Verify, injected plaque surface | 70.51 s | 58.72 s | −17% |

Peak RSS was unchanged within noise. Every after-render decoded pixel stream hashed identically to its before-render counterpart, and full verification reports matched field-for-field (scores, bases, drifts, remedies), so artifact quality and acceptance semantics are untouched. Verification remains ECC-dominated (~89% of CPU); the remaining headroom would require changing registration semantics or deeper cross-frame pipelining and is deliberately deferred.

## Measured improvement (2026-08-25) — Rust surface and build profile

This phase kept all artifact semantics identical and focused on Rust hot loops plus the shipping build profile. `Cargo.toml:43` now ships `lto = "fat"`, `opt-level = 3`, `strip = true`, `codegen-units = 1`, `panic = "abort"` with `RUSTFLAGS="-C target-cpu=native -C link-arg=-Wl,-rpath,/tmp/opencode/ocv5-local/lib"` and `CFLAGS/CXXFLAGS="-O3 -march=native -mtune=native"` where applicable.Filesystem was `sync`ed between building and the first timed run and before every run; each workload ran 3 times under `taskset -c 0-15` (unprivileged `nice` fallback, no `sudo` for `chrt`/`ionice`) and the minimum wall time is reported. Long canonical title `Nós que aqui estamos, por vós - ansiosamente - esperamos!`, `gold-shine`, `NotoSerif-Regular`, `progress never`, lossless pipeline. Pixel streams hashed via `ffmpeg -f rawvideo -pix_fmt rgba` to prove identity.

Additional surface work in this phase (all row-sharded, bitwise-identical via `run_rows` or equivalent, `std::thread::available_parallelism` with graceful fallback, pinned by existing serial-vs-threaded tests):
- `src/surface.rs:185` `blend_surface` now parallel over destination rows with hoisted clamped opacity.
- `src/surface.rs:95` `restore_from_mask` parallel over rows, fast-path for `alpha==255 && src[3]==255` preserved.
- `src/surface.rs:126` `apply_alpha_mask` parallel over rows.
- `src/surface.rs:686` `box_blur` horizontal pass parallel over rows (vertical remains sequential; column-sharded raw-pointer variant deferred to avoid `Send`/`Sync` complexity for strided writes).

All three measurement kinds come to the same conclusion (separate `bench.sh` runs, `ab-bench.sh` interleaved, and `phase2` thin-LTO baseline); the shipping profile dominates, surface sharding adds a small additional gain on the physical-plaque path.

| Workload (min of 3, `sync`+`taskset`) | Thin LTO (2026-08-23 baseline) | Fat LTO + native, before surface changes | Fat LTO + native, after surface changes | Change (thin→fat) | Change (fat before→after) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Render `16_9_swamp_wooden_plaque` 1280×720×240 | 4.89 s | 3.15 s | 2.89 s | −36% | −8% |
| Render `16_9_plaqueless_swamp` 1280×720×240 (injected plaque) | 8.73 s | 3.44 s | 3.44 s | −61% | 0% |
| Verify `16_9_swamp_wooden_plaque` | 66.38 s | 55.50 s | 55.79 s | −16% | +0.5% (noise) |
| Verify `16_9_plaqueless_swamp` | 70.55 s | 48.93 s | 48.53 s | −31% | −0.8% |

Interleaved `before-native` vs `after-native` (alternating `before`/`after` per round to cancel thermal drift, same `taskset`, `sync` before each) confirms the same: `render_swamp_wooden` 3.34 s → 3.74 s in the interrupted 2-round sample shows run-to-run variance ±0.4 s dominates the ~0.26 s mean gain, while `render_plaqueless` and both `verify` are within noise. Pixel hashes for every `after` render matched its `before` counterpart (`swamp-wooden 473c07d3abe76b0fcc5f285c6714c1012a66f84b323b07b16adc0545015ec113`, `plaqueless c07a45f56ba5006659477169617823aa5de4615318b128790f0ea553a2412f61`) and `verify --report` remained `passed: true` with identical `overall`/`tracking_lock`/`scene_integrity`.

Peak RSS unchanged (191–197 MiB render, 48–55 s verify includes ECC). The `fat` LTO + `target-cpu=native` shipping profile is the dominant win; the additional surface row-sharding is correct and preserves the `any worker count → bitwise-identical` invariant but is small on these two sentinels. Further headroom is in `src/video.rs:277` decoder buffer reuse and deeper `verify` per-pixel parallel reductions, both deferred per the cost-review table below. Python/ML paths were not measured in this phase (environment in upgrade, `opencv` rebuilt locally under `/tmp/opencode/ocv5-local` via `/tmp/opencode/pkgconfig-local`); Rust is prioritized.

After the typography regression fix, the exact long-title `Noto Serif`/`gold-shine`
fit on the 990×680 Digifall canonical surface completes in about 4–6 seconds (16 seconds
including lossless rendering of all 240 frames), compared with 1:55 in the first
correctness-only implementation and more than five minutes in the reported regression.

The final 720×1280 temporary-dungeon title rendered and encoded in 5.51 seconds;
font-aware artistic fitting took 1 second. A forced analysis after a visibility-only
scene took 50.64 seconds while reusing 240 validated automatic-foreground PNGs,
instead of relaunching an approximately twelve-minute ML pass.

## Reproducible checks

Use release builds and keep analysis/ML out of micro-measurements:

```bash
/usr/bin/time -v cargo test --release --all-targets --all-features
/usr/bin/time -v target/release/plaque-forge render \
  --input assets/<scene>.mp4 \
  --analysis assets/analysis/<scene> \
  --output /tmp/plaque-forge-perf.mkv \
  --text 'Performance check' --font "$FONT_FILE" --progress never
```

Record the source SHA-256, frame dimensions/count, encoder arguments, program long version, CPU, and peak RSS beside any comparison. Wall-clock numbers without those identities are not a meaningful regression baseline.
