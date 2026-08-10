# Validation

Run the code gate:

```bash
./scripts/check.sh
```

Validate the six existing analysis caches:

```bash
./scripts/validate_assets.sh
```

This writes verification reports without changing analysis or refinements. Acceptance requires:

- every verification score is at least `0.95`;
- every render has the source frame count and timing;
- visual review finds no plaque drift, foreground inversion, hard matte edge, or temporal blinking.

The holographic sources exercise automatic analysis. The rusty chain, swamp rusty, and swamp wooden sources exercise checked-in refinements.

## Reference scores

Validated on 2026-08-10:

| Video | Overall |
| --- | ---: |
| `moving-holographic-plaque` | 0.9920 |
| `rusty-plaque-with-object-in-front-parallax-and-plaque-moves` | 0.9881 |
| `static-holographic-plaque-with-background-movement` | 0.9996 |
| `static-holographic-plaque` | 1.0000 |
| `swamp-rusty-plaque` | 0.9960 |
| `swamp-wooden-plaque-with-foreground-objects` | 0.9986 |
