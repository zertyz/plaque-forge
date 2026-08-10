# Validation

Run the code gate:

```bash
./scripts/check.sh
```

Run the six-video acceptance set from clean generated directories:

```bash
./scripts/render_assets.sh
```

Acceptance requires:

- all six analysis and verification runs complete;
- every verification score is at least `0.95`;
- every output has the source frame count and HEVC video;
- full-size visual review finds no plaque drift, foreground inversion, hard matte edge, or temporal blinking.

The three holographic sources exercise fully automatic analysis. The rusty chain, swamp rusty, and swamp wooden sources exercise checked-in refinements.

## Acceptance run

Fresh run on 2026-08-10:

| Video | Overall |
| --- | ---: |
| `moving-holographic-plaque` | 0.9920 |
| `rusty-plaque-with-object-in-front-parallax-and-plaque-moves` | 0.9881 |
| `static-holographic-plaque-with-background-movement` | 0.9996 |
| `static-holographic-plaque` | 1.0000 |
| `swamp-rusty-plaque` | 0.9960 |
| `swamp-wooden-plaque-with-foreground-objects` | 0.9986 |

All six passed.
