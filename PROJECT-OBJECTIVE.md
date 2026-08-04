# Plaque Forge objective and quality contract

## Project objective

Plaque Forge turns designated plaque-like regions in videos into reusable title
templates. It must detect and follow a plaque through movement, scale, perspective,
and temporary disappearance; render arbitrary text so it looks native to the
plaque's material and animation; preserve foreground depth and occlusion; support
appearance, disappearance, and continuous text effects; leave unrelated image
pixels, audio, timing, and loop continuity intact; cache expensive per-video
analysis; and export lossless video or image sequences. The initial reference set
must be handled without visible tracking, compositing, typography, or timing
defects.

## First validated source class

The first production quality gate is deliberately narrower than the complete
objective. A supported source has:

- one dominant, approximately planar plaque;
- one continuous shot with fixed dimensions and constant frame rate;
- a plaque cavity that is already free of the title being replaced;
- enough visible border or internal structure to establish a stable track;
- motion that can be represented by one quadrilateral per frame;
- smooth motion with bounded blur and rolling-shutter distortion;
- foreground occlusion or temporary disappearance that can be represented by an
  alpha visibility mask;
- stable enough plaque appearance that the same canonical material/style model
  remains meaningful throughout the shot.

Sources with non-planar folding, destructive motion blur, transparent foregrounds
that cannot be separated, large unmodelled reflections, multiple competing
plaques, shot cuts, variable-frame-rate timing, or long intervals with no
recoverable motion evidence require preprocessing, supervised tracking/mattes, or
a later analysis model. They must not silently receive a high automatic confidence
score.

## Decision criteria

Before accepting a material implementation decision, evaluate it against all of
these questions:

1. Does typography remain locked to the plaque at full-size playback?
2. Does the title respect foreground depth and plaque visibility on every frame?
3. Does it look native to the plaque material rather than like a flat overlay?
4. Are unrelated pixels, audio, frame count, timestamps, and loop continuity
   preserved?
5. Can the analysis be cached and reused deterministically for different titles?
6. Is failure explicit and diagnostic instead of producing a plausible but wrong
   render?
7. Is the result covered by machine checks and representative-frame human review?

Numerical verification is necessary but not sufficient for artistic acceptance.
Golden reference frames and human review remain part of the release gate until a
perceptual metric has been validated against those judgments.

## Quality path

Automatic analysis is the fast path. Art-perfect production also needs an escape
hatch: a title-pack may contain corrected quadrilateral keyframes, plaque
visibility, foreground mattes, and a material/style profile approved once and
reused for every subsequent title. When upstream production can provide camera
transforms, depth, or object-ID mattes, those should be ingested instead of
re-inferred from flattened video.
