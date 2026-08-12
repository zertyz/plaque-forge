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
- visual review finds no plaque drift, foreground inversion, hard matte edge, temporal blinking, or obviously poor title composition.

For human triage, run `./scripts/review_assets.sh <asset-stem>` and open the generated `diagnostics/review.html`. Coverage percentages describe scene complexity; they are not treated as quality failures by themselves.

The holographic sources exercise automatic analysis. The rusty chain, swamp rusty, and swamp wooden sources exercise checked-in refinements.

## Reference scores

Reference results from the included acceptance assets:

| Video | Overall |
| --- | ---: |
| `moving-holographic-plaque` | 0.9920 |
| `rusty-plaque-with-object-in-front-parallax-and-plaque-moves` | 0.9881 |
| `static-holographic-plaque-with-background-movement` | 0.9996 |
| `static-holographic-plaque` | 1.0000 |
| `swamp-rusty-plaque` | 0.9960 |
| `swamp-wooden-plaque-with-foreground-objects` | 0.9986 |
