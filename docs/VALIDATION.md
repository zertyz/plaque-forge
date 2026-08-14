# Validation

Run the code gate:

```bash
./scripts/check.sh
```

Validate the checked-in acceptance analysis caches/renders:

```bash
./scripts/validate_assets.sh
```

This writes verification reports without changing analysis or scenes. Acceptance requires:

- the overall score meets `--minimum-score` (default `0.95`);
- each component meets its verifier-defined threshold recorded in the verification JSON;
- every render has the source frame count and timing;
- source, analysis manifest, rendered video, canonical text mask, optional contact sheet, font, style, encoder arguments, and implementation versions match recorded provenance;
- declared SDR color metadata and display rotation are preserved;
- visual review finds no plaque drift, foreground inversion, hard matte edge, temporal blinking, or obviously poor title composition.

For human triage, run `./scripts/review_assets.sh <asset-stem>` and open the generated `diagnostics/review.html`. Coverage percentages describe scene complexity; they are not treated as quality failures by themselves.

Scene integrity is alpha-aware: title/plaque coverage permits only the change source-over compositing can produce, including antialiased boundaries. Foreground restoration is also evaluated at every nonzero matte level rather than only at opaque pixels.

Rigid plaque tracking is verified from independent source material flow at consecutive
and longer frame baselines. Every comparison detects fresh source features, follows
them with a forward/backward optical-flow check, rejects coherent foreground motion
with a robust projective model, and then measures the stored four-corner trajectory's
prediction. When a scene declares a complete lossless source-pixel writing-surface
sequence, the verifier instead refits its visible four-corner silhouette and directly
compares that absolute geometry with the stored trajectory. This is the appropriate
primary evidence for a low-texture or softly deforming surface such as a cloud; source
flow remains a reported secondary diagnostic. Clipped silhouettes are not treated as
four-corner observations.

The rendered title is never registered to certify source tracking. A physical surface
without enough independent material or silhouette evidence fails as unmeasurable
instead of receiving an optimistic score. Reports expose median/p95/p99 residuals,
support counts, spatial coverage, and the worst frame. Interpolated frames are never
relabelled as observations. Appearance-template corrections remain diagnostics because
animated light can make them suggest false motion.

Verification JSON and `review.html` also report p95/p99 residuals separately at
1-, 6-, and 12-frame lags. The one-frame baseline catches a transient jump; the
longer baselines make slow drift and a screen-fixed title observable. The aggregate
score is never a substitute for these distributions or their worst-frame evidence.

The verifier separately subtracts the source from every rendered frame, rectifies that
actual title difference with the expected plaque homography, and registers it against
a canonical title signature. This catches a compositor that leaves text in screen
coordinates even when the stored trajectory itself looks plausible. It supplements—
and never replaces—the independent source-structure test above.

The plaque catalog and generated-path portability are part of `cargo test`. Catalog validation checks unique IDs and paths, both `16:9` and `9:16` members per artistic family, dimensions, SHA-256 identities, and useful transparent/soft alpha.

## Why no hard-coded reference scores

Static score tables drift whenever the corpus, verifier, masks, or encoder changes. The authoritative result is each current `output/<asset>.verification.json`, whose schema includes its source/render/analysis identities and thresholds. A score without matching provenance is not an acceptance record.

## Human quality-report index

After analysis/rendering, run:

```bash
./scripts/review_assets.sh
```

Open `output/review/index.html`. Each asset report uses the complete cache or newest compact retained failure and surfaces prioritized quality findings, candidate/tracking/extraction evidence, ML/Python participation, exact rerun/scene commands, and render/verification provenance when available. Passing metrics remove only the automated blockers they actually measure. They never certify artistic acceptance or suppress the required visual review.

## Validation levels

- `./scripts/check.sh` is the cheap deterministic code gate: formatting, Clippy with warnings denied, Rust tests, plaque/path checks, Python syntax, and shell syntax/parser regression.
- `cargo test --test project_assets` audits checked-in path portability and plaque metadata.
- `./scripts/validate_assets.sh ...` performs real lossless render + full-frame verification and can be expensive; it never invokes scene analysis or Python ML.
- Human review remains mandatory for artistic composition. Numeric metrics reject known failure modes but cannot decide taste.
