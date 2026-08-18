# Historical visual-quality recovery

This document describes a deliberately conservative way to recover visual quality that may
have been lost while Plaque Forge evolved through many independent engineering/agent passes.

The governing rule is:

> Historical output is a challenger, never an automatic rollback target.

A later implementation may be architecturally safer or algorithmically better while a small
piece of authored geometry or styling from an older revision remains visually superior. The
bakeoff therefore moves old *intent* through the current implementation instead of reviving an
entire old binary by default.

## Why this exists

Repository history contains concrete examples of visual-policy oscillation. In particular,
writing-surface selection was changed to defeat small high-contrast false positives and was
later changed again when area preference allowed larger architectural regions to steal the
surface. This makes "latest value wins" an unsafe acceptance rule for visual parameters.

The historical snapshots also contain authored geometry and style changes that are materially
large enough to deserve direct comparison rather than assumption.

## Recovered challengers

The first recovery pass intentionally contains only values with clear historical provenance and
material visual impact.

### v0.8 scene geometry

The following v0.8 authored geometry is replayed against the current scene files. Only the first
(default) surface geometry is changed; current prompts, depth semantics, foreground declarations,
segmentation policy, tracking code, renderer, and verifier remain current.

| Asset | Recovered change |
|---|---|
| `16_9_swamp_wooden_plaque` | surface `[314,24,622,183]`; writable `[346,53,558,116]`, radius `18` |
| `9_16_background_ogre_dear` | ellipse radii `[238,198]` instead of the later tighter ellipse |
| `9_16_dungeon_spider_iron_plaque` | tracking surface `[142,172,437,251]`; writable region unchanged |
| `9_16_swamp_wooden_plaque` | surface `[14,0,693,394]`; writable `[58,70,605,238]`, radius `30` |

These values come from the archived Plaque Forge v0.8 authored refinements. The corresponding
repository history identifies v0.8 as commit `884f76e` on `engineering_gemini3.7`. The 16:9
dungeon v0.8 geometry was reviewed and rejected: it places text outside the plaque and becomes
severely unstable during the moving-web crossing, so the harness no longer reruns that challenger.

### v0.5-v0.8 bronze relief

`styles/bronze-relief-banded.toml` is the recovered four-band bronze/gold treatment, retained as a
normal permanent style rather than overwriting the existing `bronze-relief` preset. Its key difference from the later style is the
four-band procedural gold/bronze material:

```toml
[material]
type = "gold"
dark = "#4B260DFF"
mid = "#B87333FF"
light = "#E8B86CFF"
highlight = "#FFE2A4FF"
```

The later style retained the shadow, extrusion, stroke, bevel and shine stack but replaced that
material with a two-color linear gradient. The older review notes identify bronze-relief as the
preset specifically tuned toward the dark iron dungeon reference, so this is a high-value A/B
candidate rather than an arbitrary old setting.

## Running the bakeoff

For the strongest evidence, install the current ML runtime and use the canonical deterministic
quality policy:

```bash
./scripts/setup_segmentation.sh
./scripts/bakeoff_historical_quality.sh --ml on
```

A single remaining geometry case can be run while iterating, for example:

```bash
./scripts/bakeoff_historical_quality.sh --ml on 16_9_swamp_wooden_plaque
```

Use the exact same text, font, and policy for both candidates in any comparison.

The harness writes only under `output/quality-bakeoff/` plus a temporary
`assets/.quality-bakeoff/` directory that is removed on exit. It does not replace canonical
scenes, canonical analysis, normal renders, or homologation contracts.

Each variant is analyzed with the current binary. This is important: the geometry comparison is
not polluted by simultaneously changing tracking/segmentation/rendering implementations.
Compatible bakeoff analysis caches are reused by default, so a failed render does not trigger another
expensive ML analysis. Use `--force-analysis` only when deliberately rebuilding the candidate caches.
Every rendered candidate is lossless FFV1, receives the ordinary verification pass, and retains
render/verification diagnostics even when a score fails.

For ordinary geometry cases, inspect:

```text
output/quality-bakeoff/<asset>/geometry-side-by-side.mp4
```

Left is current geometry; right is recovered v0.8 geometry.

For the remaining portrait-dungeon geometry experiment, inspect:

```text
output/quality-bakeoff/9_16_dungeon_spider_iron_plaque/dungeon-geometry-style-2x2.mp4
```

The quadrants are:

```text
current geometry + current bronze     | current geometry + banded bronze
--------------------------------------+--------------------------------
v0.8 geometry   + current bronze      | v0.8 geometry   + banded bronze
```

That factorial comparison distinguishes a geometry regression from a material/style regression.

### Settled: 16:9 dungeon spider plaque

Human review selected the current authored geometry as the champion. The recovered v0.8 geometry
was rejected because it placed text outside the plaque and became severely unstable during the
web crossing. The optical-web challenger improved foreground recognition but did not materially
improve the final title behavior, so it is not promoted into the canonical scene.

The accepted contract protects the current plaque geometry and the opaque spider/source
preservation witnesses. The remaining translucent-web disturbance is deliberately deferred for a
later tracker/matting investigation rather than growing more one-off scene policy in this pass.

### Settled: moving holographic plaque

Human review selected the recovered dense v0.8 trajectory as the perfect result. That exact
240-frame source-pixel trajectory is now promoted into the canonical scene as reviewed locked
motion. The temporary archived moving-holographic recovery payload is removed from the harness.

A later generic-tracker improvement may use this canonical reviewed trajectory as an oracle, but
production rendering no longer depends on rediscovering a motion solution that is already known
to be correct for this asset.

### Settled: rusty moving plaque with crossing chains

Human review selected the current automatic tracker as the perfect plaque-motion solution. The
reviewed chain alpha sequence, which is byte-identical across archived releases 0.5 through 0.8,
was promoted as an authored foreground artifact. The canonical scene keeps that foreground
compositing-only (`affects_tracking = false`), so the recovered chains cannot perturb the accepted
tracker. The resulting canonical render was reviewed as perfect.

The temporary archived rusty trajectory payload and its bakeoff branch are therefore removed.

## Promotion rule

A candidate may replace current authored intent only when all of the following are true:

1. the side-by-side result is visibly superior for the intended title surface;
2. verification does not expose a tracking, source-preservation, occlusion, temporal or typography regression;
3. foreground-heavy scenes have been rerun with the same current ML policy on both candidates;
4. the winning result is encoded into a homologation contract or equivalent reviewed witness;
5. the historical challenger is then removed or retained only as history, so there is one canonical winner.

Do not make the bakeoff script part of normal CI. It is forensic/review tooling. CI should protect
*promoted* winners through homologation, not repeatedly re-run historical experiments.

## What this pass does not solve

This first pass recovers historical **authored intent and styling**. It deliberately does not claim
to identify the best historical implementation of tracking, extraction, occlusion, or ML itself.
Those require a second experiment because building an old commit changes many variables at once.

The next forensic phase should use git worktrees for only the commits that materially changed a
specific subsystem, render the same fixed corpus/scene/style through each implementation, and
then port a winning algorithmic idea forward into the current architecture. Do not cherry-pick an
old implementation wholesale merely because one output looks better.
