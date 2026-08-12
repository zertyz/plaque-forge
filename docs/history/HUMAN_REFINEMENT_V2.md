# Human refinement v2

Status: **implemented in 0.7.0**. This file is retained as the design record for the transition.

The implemented split is:

- `refinement.toml` contains sparse human intent;
- schema-1 files remain readable;
- sparse motion corrections can live directly in `[[plaques.motion]]`;
- sparse segmentation prompts may use resolution-independent normalized coordinates;
- dense exported motion now defaults under `artifacts/`;
- failed analysis automatically produces a human `review.html` and `review.txt`;
- the HTML review contains a click-based coordinate helper;
- machine analysis caches remain separate under `assets/analysis/`.

The remaining future enhancement is a richer interactive editor for dragging complete regions/boxes, rather than merely clicking coordinates. The static HTML helper intentionally requires no server or GUI dependency.
