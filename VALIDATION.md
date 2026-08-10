# Validation status

## Metadata workflow result (2026-08-09)

- `init` selected the expected static reference plaque at frame 20 with bounds
  `331,33,633,152` and retained a distinct commented alternative.
- Automatic, 240-frame guided, mixed `1 locked / 239 guided`, and dense locked
  analyses all completed through the generated sidecar.
- A reviewed visibility value of `0.25` at frame 120 survived automatic occlusion
  analysis exactly; the locked frame also round-tripped with identical quad values.
- Comment-only metadata changes reused the title-pack, while an explicit plaque
  bounds override triggered reanalysis and was recorded in provenance.
- The sidecar plus dense TOML reference gate passed at `1.000` overall, tracking,
  scene integrity, typography, temporal stability, occlusion, and loop continuity.
  The verifier reported `authoritative-human-quad-track`, and packet counts matched
  at 240 frames. Artifacts are under
  `/tmp/plaque-forge-metadata-final.8PtmYs/`.

## Completed locally

- `Cargo.toml` and `config/default.toml` parse successfully.
- `cargo fmt --all` completes successfully.
- `cargo clippy --all-targets --all-features -- -D warnings` passes.
- `cargo test --all-targets --all-features` passes.
- No `todo!()` or `unimplemented!()` placeholders remain.
- Python tools compile successfully.
- Shell scripts pass `bash -n`.
- The release binary builds against Rust 1.97.1 and OpenCV 4.14.0.
- The release binary reports a deterministic source fingerprint through `--version`.

## Required before accepting a reference asset

```bash
./scripts/check.sh

REFERENCE_VIDEO=/path/to/text-free-plaque.mp4 \
REFERENCE_FONT=/path/to/font.ttf \
scripts/validate_reference.sh
```

Set `REFERENCE_METADATA=/path/to/video.plaque.toml` to validate sidecar geometry.
Add `REFERENCE_MOTION_TRACK=/path/to/video.main.track.toml` for the TOML
production path. `REFERENCE_TRACK=/path/to/reviewed-track.csv` retains the legacy
CSV gate. Omit them to measure automatic tracking.

The second command is intentionally slower. It creates a fresh analysis cache,
lossless render, verification report, packet-count comparison, and render contact
sheet. Machine verification and human contact-sheet review are both required.

## Behavioral validation sequence

1. Produce a text-free plaque source and create its sidecar with `init`.
2. Analyze it, export a guided track, then test guided, mixed, and dense locked reimports.
3. Confirm authored visibility and locked quads survive automatic refinement.
4. Run `replace` with lossless FFV1 output and confirm every verification threshold passes.
5. Inspect `tracking-contact-sheet.jpg` and the reported worst tracking frame.
6. Inspect `render-contact-sheet.png` for material integration and occlusion.
7. Render a second title from the same title-pack to confirm cache reuse.
8. Confirm comment-only TOML changes reuse the cache while bounds, track, and CSV changes invalidate it.
9. Test an intentionally oversized fixed font and a font missing a requested glyph; both must fail deterministically.
10. For a low-confidence automatic track, confirm analysis fails unless
   `--allow-low-confidence` is explicitly supplied.

The verifier checks residual structural registration, trajectory smoothness,
circular loop continuity, exact untouched pixels, typography validity, and opaque
occluder restoration. Pixel differences caused by legitimate plaque animation
are reported diagnostically and are not mislabeled as title jitter.

## Current reference result (2026-08-02)

The moving-plaque reference completes without `--plaque-hint` in about 41
seconds and passes at `0.989` overall, `0.958` tracking lock, and `0.994`
temporal stability. The equivalent hinted run passes at `0.993` overall and
`0.975` tracking lock. Plaque-border feature tracking replaces the former
background-motion proxy.
The static reference completes in about 37 seconds and passes at `1.000` overall.
A rusty moving-plaque reference with parallax and a foreground crossing passes
without `--plaque-hint` at `0.991` overall and `0.965` tracking lock. Automatic
selection finds the full `322,34,651,145` plaque instead of its internal strip.
A provisional 21-keyframe smoothed CSV exercises the supervised interface and
passes the machine gate at `0.9996`; it has not received artist approval and is
therefore a review candidate, not a golden result. The rendered typography still
uses the static fill/stroke/glow model and is not an art-perfect material match.
