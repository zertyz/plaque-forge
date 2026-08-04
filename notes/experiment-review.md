# Review of `video-title-replacer.tar.xz`

Eight source variants were inspected. Useful ideas and recurrent failure modes are summarized here so they remain design inputs rather than fossilized code paths.

## Ideas retained

- HSV/color segmentation as one plaque-candidate signal.
- ORB feature extraction outside the title region.
- RANSAC-style motion estimation from multiple features.
- moving the cleanup mask with the plaque.
- explicit foreground restoration after plaque compositing.
- word wrapping and automatic size fitting as required behavior.

## Replaced

### Three synthetic corner points + Lucas-Kanade

The active version tracks rectangle coordinates that are not guaranteed to be image features. It also does not robustly reject failed tracks. The new tracker estimates camera motion from hundreds of stable background points and registers frames directly to a common reference.

### Hard-coded cyan identity

Cyan is now a vote in an ensemble. Geometry, text density, persistence, and motion coherence must corroborate it.

### Per-frame Telea inpainting

Independent inpainting invents a different background each frame and causes crawling artifacts. Plaque extraction must happen in stabilized canonical space using several frames.

### OpenCV Hershey typography

It cannot honor arbitrary font files or provide modern Unicode shaping. Typography moves to `cosmic-text`.

### OpenCV `mp4v` output

Analysis and rendering are separated from encoding. FFmpeg produces lossless or user-selected output while preserving audio and timestamps.

### Color-threshold claw restoration

Foreground status is inferred from motion disagreement and parallax relative to the plaque plane, not from object-specific BGR thresholds.
