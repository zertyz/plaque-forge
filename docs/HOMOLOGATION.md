# Homologated output regression protection

A **homologation contract** records behavior that a human has already accepted and that
future internal changes are expected to preserve. It is deliberately narrower than a
pixel-perfect golden video: the contract stores stable scene geometry, typography
constraints, and sparse visual witnesses for important interactions such as a foreground
object crossing newly rendered text.

Contracts live under:

```text
assets/homologation/<asset>/contract.toml
```

Reviewed masks and other small evidence files live beside the contract. Generated
analysis data remains under `assets/analysis`; it is never itself accepted as the oracle.

## What a contract can protect

The current schema (`plaque-forge.homologation/1`) pins:

- the source-video identity;
- the intended tracking and writable geometry of the homologated surface;
- title text, selected font file, style identity, line layout, and safe typography limits;
- sparse source-preservation masks on selected frames, including mask identity and minimum witness coverage;
- provenance of the rendered video, its render manifest, and the analysis manifest.

A source-preservation mask means: **pixels selected by this reviewed mask must still
look like the source in the new render**. How the implementation achieves that is not
part of the contract. The renderer may use structural masks, ML segmentation, a new
matting algorithm, or a different compositing implementation.

This makes foreground regression tests semantic rather than mechanical. A test such as
"the segmentation PNG is non-empty" is not a substitute for proving that the spider,
branch, hand, or other foreground object actually remains in front of the title.

## Run the homologated integration gate

```bash
./scripts/check_homologated_assets.sh
```

The script renders the representative delivery artifact and then runs:

```bash
plaque-forge homologate \
  --contract assets/homologation/<asset>/contract.toml \
  --rendered output/<asset>.hevc.mkv \
  --report output/<asset>.homologation.json
```

The report contains the SHA-256 of the exact rendered bytes and render manifest. A report
for older bytes is not valid for a replacement render. `render_assets.sh` therefore
removes stale verification/homologation reports after a successful replacement.

CI runs the homologation gate separately from the fast code gate because it performs a
real video render and decode.

## Adding a new homologated case

1. Reach an output that has been visually reviewed and explicitly accepted.
2. Write the smallest contract that captures the behavior that must not regress.
3. Prefer invariants and sparse reviewed witnesses over a whole-video golden comparison.
4. For foreground crossings, choose a few representative frames and masks covering the
   pixels whose depth ordering matters.
5. Add the case to `scripts/check_homologated_assets.sh` when it is important enough to
   pay the CI runtime cost.
6. Verify that the new test fails when the accepted behavior is intentionally broken.

Do not silently weaken a contract to make a regression green. If intended behavior
changes, update the requirement and homologation evidence together, and review the new
output before accepting the contract change.

## Verification versus homologation

`plaque-forge verify` performs broad algorithmic/full-frame checks against the source and
analysis. `plaque-forge homologate` protects selected human-accepted observable behavior.
They are complementary:

- **verification** asks whether the render satisfies general measurable correctness rules;
- **homologation** asks whether behavior previously accepted for this particular case was
  preserved.

Neither report is valid for a rendered artifact whose SHA-256 differs from the hash stored
in that report.
