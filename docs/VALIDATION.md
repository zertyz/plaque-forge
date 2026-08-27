# Validation

> **Status: Descriptive**
> **Normative references:** `BUS-QUAL-*`, `REQ-QUAL-*`, `REQ-HOM-*`, `OPS-ART-003`

Run the code gate:

```bash
./scripts/check.sh
```

Validate selected renders exhaustively against their source/analysis:

```bash
./scripts/validate_assets.sh
```

Lossless verification artifacts, render manifests, and reports are retained together under
`output/validation/`. The report therefore remains inspectable against the exact bytes it
certifies, and is deliberately not named as though it certifies the separately encoded
`output/*.hevc.mkv` delivery file. `render_assets.sh` removes stale acceptance reports after
replacing a delivery render.

For previously human-accepted observable behavior, run the representative homologation gate:

```bash
./scripts/check_homologated_assets.sh
```

This produces `output/<asset>.homologation.json`, cryptographically bound to the exact delivery
render and its render manifest. See [Homologation](HOMOLOGATION.md).

Full-frame verification acceptance requires:

- the overall score meets `--minimum-score` (default `0.95`);
- each component meets its verifier-defined threshold recorded in the verification JSON;
- every render has the source frame count and timing;
- source, every consumed analysis input, rendered video, canonical text mask, render decision trace, optional contact sheet, font, style, encoder arguments, and the exact renderer-source identity match recorded provenance;
- declared SDR color metadata and display rotation are preserved;
- visual review finds no plaque drift, foreground inversion, hard matte edge, temporal blinking, or obviously poor title composition.

For human triage, run `./scripts/review_assets.sh <asset-stem>` and open the generated
`diagnostics/review.html`. Review accepts verification evidence only together with the exact
render manifest whose SHA-256 and source/analysis/render identities match the report. Stale or
cross-wired evidence is rejected. The same page surfaces the provenance-bound render decision trace so surface selection, tracking participation, typography, and matte semantics can be inspected causally. Coverage percentages describe scene complexity; they are not
treated as quality failures by themselves.

Scene integrity is alpha-aware: title/plaque coverage permits only the change source-over compositing can produce, including antialiased boundaries. Foreground restoration is also evaluated at every nonzero matte level rather than only at opaque pixels.

Rigid plaque tracking is verified from independent source material flow at consecutive
and longer frame baselines. Every comparison detects fresh source features, follows
them with a forward/backward optical-flow check, rejects coherent foreground motion
with a robust projective model, and then measures the stored four-corner trajectory's
prediction. A complete lossless source-pixel writing-surface sequence limits those
features to material that belongs to the plaque. This remains the primary evidence for
automatic or partially authored trajectories.

A different rule applies to a dense, fully locked, explicitly reviewed four-corner
trajectory. Once every frame is reviewed and provenance-pinned, direct source-subtracted
title registration against that exact plane is authoritative for render tracking. This
prevents moving web/spider texture from overruling both the reviewed geometry and a
measured zero-drift title. Independent source-flow distributions remain in the report as
diagnostics; an incomplete review, guide frame, or missing direct title evidence restores
the automatic evidence path. A hard trajectory-residual failure still rejects either
path. Appearance-template corrections remain diagnostics because animated light can
suggest false motion.

Verification JSON and `review.html` also report p95/p99 residuals separately at
1-, 6-, and 12-frame lags. The one-frame baseline catches a transient jump; the
longer baselines make slow drift and a screen-fixed trajectory observable. Acceptance
uses the lag-1 tail for localized slips and each baseline's p95 for sustained drift.
Long-baseline p99 values remain diagnostics: a few tracks can stop describing the same
material point across a thin foreground crossing or a large perspective change even
when the sustained distribution is subpixel. The aggregate distribution is never a
substitute for the per-baseline evidence or its worst-frame diagnostic.

Raw trajectory curvature is reported separately from temporal stability. Curvature is
only evidence of tracker jitter when neither independent source-material flow nor a
fully reviewed trajectory with direct title-plane evidence corroborates the motion;
physically observed acceleration must not be failed merely because the plaque does not
move at constant velocity. The localized four-corner residual limit remains mandatory.

The verifier separately subtracts the source from every rendered frame, rectifies that
actual title difference with the expected plaque homography, and registers it against
a canonical title signature. This catches a compositor that leaves text in screen
coordinates even when the stored trajectory itself looks plausible. It supplements
automatic tracking evidence and is authoritative only for the fully reviewed
dense-trajectory case described above.

The plaque catalog and generated-path portability are part of `cargo test`. Catalog validation checks unique IDs and paths, both `16:9` and `9:16` members per artistic family, dimensions, SHA-256 identities, and useful transparent/soft alpha.

## Why no hard-coded reference scores

Static score tables drift whenever the corpus, verifier, masks, or encoder changes. The
authoritative result is the report whose recorded source/render/analysis identities match the
artifact being discussed. `validate_assets.sh` stores lossless verifier reports under
`output/validation/`; homologated delivery renders use `output/<asset>.homologation.json`. A
score or green-looking JSON file without matching provenance is not an acceptance record.

## Human quality-report index

After analysis/rendering, run:

```bash
./scripts/review_assets.sh
```

Open `output/review/index.html`. Each asset report uses the complete cache or newest compact retained failure and surfaces prioritized quality findings, candidate/tracking/extraction evidence, ML/Python participation, exact rerun/scene commands, and render/verification provenance when available. Passing metrics remove only the automated blockers they actually measure. They never certify artistic acceptance or suppress the required visual review.

## Validation levels

- `./scripts/check.sh` is the cheap deterministic code gate: formatting, Clippy with warnings denied, Rust tests, plaque/path checks, Python syntax, and shell syntax/parser regression.
- `cargo test --test project_assets` audits checked-in path portability and plaque metadata.
- `./scripts/validate_assets.sh ...` performs real lossless render + full-frame verification,
  retains the certified artifact under `output/validation/`, and can be expensive; it never
  invokes scene analysis or Python ML.
- `plaque-forge homologation-coverage` validates the behavioral capability matrix and reports which representative cases still await explicit human acceptance.
- `./scripts/check_homologated_assets.sh` performs a real delivery render and enforces sparse human-accepted regression contracts; CI runs this as a separate integration gate. Failed semantic witnesses retain source/render/diff/overlay diagnostics under `output/regressions/`.
- Human review remains mandatory when establishing or deliberately changing artistic composition. Once accepted, the corresponding homologation contract makes that behavior executable.
