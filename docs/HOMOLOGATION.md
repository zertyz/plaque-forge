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
- the intended tracking and writable geometry of the homologated surface, optionally including the reviewed trajectory identity;
- title text, selected font file, style identity, line layout, and safe typography limits;
- sparse source-preservation masks on selected frames, including mask identity and minimum witness coverage;
- sparse title-visibility masks where reviewed pixels must remain visibly changed from the source;
- provenance of the rendered video, its render manifest, every consumed analysis input, and the exact renderer-source identity.

A source-preservation mask means: **pixels selected by this reviewed mask must still
look like the source in the new render**. How the implementation achieves that is not
part of the contract. The renderer may use structural masks, ML segmentation, a new
matting algorithm, or a different compositing implementation.

A title-visibility mask expresses the complementary requirement: **reviewed pixels must
remain visibly changed from the source**. Its minimum mean and median RGB errors prove that
the title is still present there. Pairing both witness types is useful for porous foregrounds:
source-preservation pixels pin the web threads, while title-visibility pixels pin the open
gaps. An implementation that turns the web into an opaque sheet fails the latter even if it
passes every thread witness.

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
real video render and decode. The permanent CI set is deliberately representative rather
than exhaustive: static fitting, reviewed projective motion, moving foreground/parallax,
and retracting portrait occlusion each have one accepted sentinel.

## Adding a new homologated case

1. Reach an output that has been visually reviewed and explicitly accepted.
2. Write the smallest contract that captures the behavior that must not regress.
3. Prefer invariants and sparse reviewed witnesses over a whole-video golden comparison.
4. For foreground crossings, choose a few representative frames and masks covering the
   pixels whose depth ordering matters. For porous material, pair source-preservation masks
   on material with title-visibility masks in reviewed gaps.
5. When the case is important enough to pay the CI runtime cost, set `ci = true` for its
   capability and add the matching contract render to `scripts/check_homologated_assets.sh`.
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


## Capability coverage

`assets/homologation/capabilities.toml` is the catalog-level regression map. It groups the asset
set by behavioral capability instead of requiring one expensive golden test per input video. Each
entry chooses a representative scene. A `contract` is added only after a human has explicitly
accepted that representative output; `ci = true` marks the small subset worth a real render on every
CI run.

Inspect coverage without rendering:

```bash
target/release/plaque-forge homologation-coverage \
  --matrix assets/homologation/capabilities.toml \
  --report output/homologation-coverage.json
```

Use `--require-complete` only for a release/process gate that intentionally requires every declared
capability to have been human-homologated. Normal CI must not fabricate acceptance for candidate
representatives merely to make the percentage green.

## Failure diagnostics

Pass `--diagnostics output/regressions` to `homologate`. A failed visual witness emits
a directory containing `source.png`, `rendered.png`, `diff-3x.png`, `witness-overlay.png`, and the
reviewed `witness-mask.png`. The homologation JSON points to that directory. This makes a failed
depth/foreground contract visually diagnosable without weakening it or replaying the comparison by
hand.

The CI homologation job uploads these compact reports/images when the gate fails, so the visual
evidence survives the ephemeral runner. It intentionally does not upload the full rendered video.

## Decision traces

The render manifest hashes the adjacent decision trace. Homologation validates that trace before
accepting an artifact. The trace explains *why* a render made important choices (surface selection,
tracking model, typography, foreground tracking participation, matte semantics) while the
homologation contract remains the independent statement of *what observable behavior must remain
true*. Never use a generated decision trace as its own acceptance oracle.

## Segmentation capability ledger

Final-render homologation remains the authority for visible behavior, but ML regressions need a map of which semantic situations have representative media. `assets/homologation/segmentation-capabilities.toml` records generic opaque foregrounds, temporal reappearance, parallax, soft/translucent boundaries, human fine detail, and open-vocabulary segmentation. The ledger may name a representative without claiming acceptance. `final_homologation` is present only when a human-reviewed render contract protects that capability.

Run `./scripts/check_segmentation_capabilities.sh` to emit `output/segmentation-capability-coverage.json`. Do not turn missing human/open-vocabulary representatives into synthetic green contracts.
