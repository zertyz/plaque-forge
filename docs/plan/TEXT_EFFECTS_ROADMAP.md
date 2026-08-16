# Text effects roadmap

Status: the capability backlog derived from the original Rust text-art experiments is
implemented in the production style pipeline.

## Implemented families

- shaped text fitting and artistic line composition;
- arc/orbit layout deformation;
- static and procedural materials;
- external PNG texture mapping with provenance;
- stroke, glow, shadow, depth, bevel, letterpress, chromatic split, and trails;
- blueprint and fibrous-paper/collage treatments;
- pulse, shine, flicker, wave, typewriter, dissolve, scramble, split-flap, glitch, and orbit;
- deterministic particle/confetti convergence;
- plaque-sampled laser burn / engraving;
- plaque-sampled height-field emboss/protrusion with cast shadow and inferred light direction.

## Future research, not missing POC coverage

These are possible quality/performance extensions rather than missing named effects:

- retain explicit shaped-glyph objects so curved text can move rigid glyphs instead of
  deforming supersampled coverage;
- GPU acceleration behind the existing renderer boundary;
- multistop/radial gradients and animated texture coordinates;
- optional true 3D plaque geometry/normal maps when scene reconstruction provides them;
- richer particle emitters and collision/force fields.

Those extensions must remain downstream of scene analysis so changing typography never
invalidates tracking/extraction caches.
