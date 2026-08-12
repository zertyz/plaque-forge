# Glossary

Plaque Forge uses a few computer-vision terms that are easy to misread without context.

**Plaque**  
The planar surface in the source video that will receive new text. A plaque can move, rotate in perspective, and be partly covered by foreground objects.

**Tracker / tracking**  
The analysis subsystem that estimates where the plaque is on every video frame. Its output is a frame-by-frame geometric transform from a stable plaque coordinate system to source-video pixels.

**Reference frame**  
A source-video frame used as the geometric anchor for the plaque definition.

**Canonical plaque**  
A rectified, stable view of the plaque. Perspective and per-frame motion are removed so analysis and typography can work in one coordinate system.

**Writing surface / content mask**  
The region of the canonical plaque where text is allowed. The mask can be irregular, so text fitting is not limited to a rectangle.

**Structural mask / template**  
Stable plaque features used to measure and correct tracking. These are scene features, not typography.

**Occluder**  
An object that passes between the camera and the plaque, such as a chain, plant, or character. Its source pixels must be restored over the newly rendered text.

**Alpha mask**  
A grayscale image describing coverage. Black means absent, white means fully present, and gray values preserve soft or partially transparent edges.

**Layer**  
A declared scene component used during analysis or compositing. Examples include a writing surface, foreground object, or shadow.

**Segmentation**  
The process of separating a requested object from the rest of a frame. Plaque Forge can delegate this optional task to Python-based ML models through a narrow worker protocol.

**Refinement**  
Reviewed input that corrects or supplements automatic analysis. `refinement.toml` is the human entry point. Large motion tracks and mask sequences are artifacts that can be generated, reviewed, and reused rather than typed by hand.

**Analysis cache**  
Reusable generated scene data under `assets/analysis/<name>/`. It contains motion, masks, templates, diagnostics, and provenance needed to render titles without repeating expensive analysis.

**Locked motion keyframe**  
A human-approved plaque position that analysis must honor exactly.

**Guide motion keyframe**  
A suggested plaque position that guides automatic tracking but does not fully override it.

**Source-pixel coordinates**  
Coordinates measured directly in the original video frame.

**Plaque-canonical coordinates**  
Coordinates measured in the rectified plaque coordinate system.


**Text style**  
The paint/material/effect description applied after the title has been shaped and laid out. Current styles can contain fill, stroke, glow, and shadow. A style does not change plaque analysis.

**Artistic fit**  
An optional title-layout mode that tries several explicit word-boundary line arrangements, measures them using the real shaped font output, and chooses a visually balanced layout that still fits the writable mask.

**Resolved line layout**  
The exact text after renderer-selected line breaks. It is recorded in render provenance so automatic composition decisions can be reviewed.

**Canonical title layer**  
The title rendered in canonical plaque coordinates before the plaque motion transform is applied to each video frame.
