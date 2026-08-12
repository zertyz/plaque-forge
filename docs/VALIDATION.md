# Validation

Run the code gate:

```bash
./scripts/check.sh
```

Validate the checked-in acceptance analysis caches/renders:

```bash
./scripts/validate_assets.sh
```

This writes verification reports without changing analysis or refinements. Acceptance requires:

- the overall score meets `--minimum-score` (default `0.95`);
- each component meets its verifier-defined threshold recorded in the verification JSON;
- every render has the source frame count and timing;
- source, analysis manifest, rendered video, canonical text mask, optional contact sheet, font, style, encoder arguments, and implementation versions match recorded provenance;
- declared SDR color metadata and display rotation are preserved;
- visual review finds no plaque drift, foreground inversion, hard matte edge, temporal blinking, or obviously poor title composition.

For human triage, run `./scripts/review_assets.sh <asset-stem>` and open the generated `diagnostics/review.html`. Coverage percentages describe scene complexity; they are not treated as quality failures by themselves.

Scene integrity is alpha-aware: title/plaque coverage permits only the change source-over compositing can produce, including antialiased boundaries. Foreground restoration is also evaluated at every nonzero matte level rather than only at opaque pixels.

Tracking uses structural registration when the analyzed surface has stable physical
edges. A deliberately soft, very wide low-texture surface (for example a moving cloud)
is verified instead by registering the rendered title in canonical coordinates on every
frame. This detects visible title drift/blinking without treating evolving cloud texture
as a rigid plaque; the report records which basis was used.

The plaque catalog and generated-path portability are part of `cargo test`. Catalog validation checks unique IDs and paths, both `16:9` and `9:16` members per artistic family, dimensions, SHA-256 identities, and useful transparent/soft alpha.

## Why no hard-coded reference scores

Static score tables drift whenever the corpus, verifier, masks, or encoder changes. The authoritative result is each current `output/<asset>.verification.json`, whose schema includes its source/render/analysis identities and thresholds. A score without matching provenance is not an acceptance record.

## Human quality-report index

After analysis/rendering, run:

```bash
./scripts/review_assets.sh
```

Open `output/review/index.html`. Each asset report uses the complete cache or newest compact retained failure and surfaces prioritized quality findings, candidate/tracking/extraction evidence, ML/Python participation, exact rerun/refinement commands, and render/verification provenance when available. A passing exhaustive rendered-video verification supersedes low raw analysis confidence as a triage blocker; the raw score remains visible as context for low-texture or explicitly refined surfaces.

## Validation levels

- `./scripts/check.sh` is the cheap deterministic code gate: formatting, Clippy with warnings denied, Rust tests, plaque/path checks, Python syntax, and shell syntax/parser regression.
- `cargo run -- migrate-analysis --root assets/analysis` is a read-only portability/current-schema audit.
- `./scripts/validate_assets.sh ...` performs real lossless render + full-frame verification and can be expensive; it never invokes scene analysis or Python ML.
- Human review remains mandatory for artistic composition. Numeric metrics reject known failure modes but cannot decide taste.
