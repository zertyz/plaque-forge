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
  its complete portable request identity and every lossless output validate. Refinement-
  only iterations no longer initialize SAM/Cutie/VitMatte for an identical request.

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
| SAM/Cutie/Matte | Models initialize per worker request | Defer a long-lived worker until isolation, cache invalidation, and GPU recovery are designed |
| Asset batches | Sequential by default | Keep: unbounded parallel FFmpeg/ML can exhaust RAM, VRAM, or disk bandwidth |

## Deferred performance work

The next performance phase should profile representative `16:9` and `9:16` sources before changing code. High-value candidates are reusable frame buffers, SIMD/GPU warp/composite kernels with golden-frame tests, a bounded long-lived ML service, lossless frame streaming that SAM2 can consume directly, and resource-aware parallel asset scheduling.

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

After the typography regression fix, the exact long-title `Noto Serif`/`gold-shine`
fit on the 990×680 Digifall canonical surface completes in about 4–6 seconds (16 seconds
including lossless rendering of all 240 frames), compared with 1:55 in the first
correctness-only implementation and more than five minutes in the reported regression.

The final 720×1280 temporary-dungeon title rendered and encoded in 5.51 seconds;
font-aware artistic fitting took 1 second. A forced analysis after a visibility-only
refinement took 50.64 seconds while reusing 240 validated automatic-foreground PNGs,
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
