# Video Augmentation Mission "AugMsn"

Plaque Forge transforms natural, historical, and cinematic video footage into augmented commemorative scenes by tracking planar surfaces, rendering physically stylized commemorative plaques with custom typography, and compositing them behind moving foreground occluders.


## 01) Automated Plaque Insertion

The engine must take raw video footage ([`assets/videos/`](file:///root/opencode/plaque-forge/assets/videos)), discover or receive designated surface anchors, track camera motion and perspective changes across all frames, and composite a photo-realistic or styled title plaque into the scene.


## 02) Supported Target Use Cases

Plaque Forge serves three primary production workflows:
1. **Documentary & Historical Preservation**: Placing commemorative brass, stone, or wooden plaques onto monuments, natural landscapes, and heritage sites with archival realism.
2. **Cinematic & Narrative Worldbuilding**: Placing fantasy inscriptions, cyberpunk holographic plaques, and industrial markers into atmospheric environments.
3. **Short-Form & Vertical Video**: Placing spatially stabilized titles and badges onto portrait footage for mobile and social publication.



# Visual Realism & Surface Fidelity "VisFid"

Inserted plaques must appear organically embedded into the original scene rather than looking like an artificial static 2D subtitle overlay.


## 01) Physical Material Styles

The system must support distinct, physically grounded material styles:
- `gold-shine`: Polished gold or brushed brass with anisotropic reflections and moving specular highlight sweeps reacting to camera motion.
- `classic-glow`: Luminous holographic or neon cybernetic projection with soft glow bloom and alpha transparency.
- Additional physical textures: Weathered bog wood, mossy ancient bronze, engraved dungeon iron, and industrial sheet metal.


## 02) Subpixel Spatial & Perspective Lock

Plaque geometry must remain rigidly locked to the target surface in 3D projective space across camera pans, tilts, dolly motions, and zooms. Frame-to-frame position jitter must not exceed subpixel thresholds.


## 03) Dynamic Scene Illumination

When ambient lighting in the video shifts (e.g. shadows passing over the surface, day-to-night transitions, flickering torches), the rendered plaque's diffuse and ambient terms must adapt smoothly without visual popping.



# Occlusion & Foreground Preservation "OccPrs"

The core visual signature of Plaque Forge is depth correctness: physical objects existing in front of the target surface must naturally occlude the plaque and its typography.


## 01) Depth-Ordered Foreground Compositing

When foreground subjects (e.g., spiders, lizards, swinging chains, foliage, moving characters) cross the region where the plaque is placed, the foreground subject must remain visually in front. Plaque pixels must never paint over authentic foreground matter.


## 02) Boundary Integrity & Temporal Continuity

Foreground masks must preserve fine geometric structures (e.g., spindly spider legs, chain links, lizard claws). Foreground occluders must maintain temporal continuity without mask flickering, dropout frames, or edge halo chatter across consecutive video frames.


## 03) Shadow & Multi-Layer Interaction

The engine must support multi-layered compositing where foreground occluders cast shadows onto the plaque surface or where independent foreground elements operate at distinct depth planes.



# Typography & Layout "TypoLay"

Typography must be legible, aesthetically balanced, and robust against variable text inputs.


## 01) Multi-Line Fitting & Wrapping

The typography subsystem must break lines, calculate margins, and scale font sizes to fit arbitrary human-supplied titles into the designated writable area (rectangular or non-rectangular) without visual clipping or illegible compression.


## 02) Curated Typographic Hierarchy

The engine must support curated font families, explicit weights, and unicode character sets (including accented characters and multi-language scripts), maintaining crisp anti-aliased glyph contours under projective warping.



# Video Delivery & Formats "VidFmt"

Outputs must meet broadcast and modern digital streaming standards.


## 01) Aspect Ratio Independence

The system must natively support standard landscape formats (`16:9` widescreen) and vertical mobile formats (`9:16` portrait) without aspect-ratio distortion or hardcoded dimension assumptions.


## 02) Production Encoding Standards

Final rendered outputs must be generated in high-efficiency video codecs (HEVC / H.265 in Matroska containers for archival master preservation, and H.264 MP4 for web and mobile previews), balancing visual fidelity with streaming performance.



# Asset Protection & Non-Regression "AstNrg"

Plaque Forge operates with a library of real video assets. Because visual quality in video compositing is subjective and sensitive to subtle mathematical drift, human-accepted visual assets require absolute protection.


## 01) Frozen Visual Baseline Protection

Once a video scene's rendered output has been reviewed, accepted, and homologated by a human, its visual quality represents an immutable baseline. No subsequent code change, dependency bump, or machine learning model change may degrade the visual quality or cause contract violations on an existing homologated asset.


## 02) Independent Scene Tuning Isolation

Tuning tracking algorithms, prompt strategies, or segmentation models to resolve difficulties in one video scene must be strictly isolated. Improvements to scene A must NEVER alter or degrade the visual output of scene B.


## 03) Dual-Witness Acceptance Standard

Every rendered video asset must pass automated dual-witness verification before delivery:
1. **Source Preservation**: Pixels outside the plaque bounding polygon must remain identical to the original source footage within strict encoder tolerances.
2. **Title Visibility**: The inserted plaque text must maintain continuous optical contrast, geometric integrity, and legibility throughout its active display duration.
