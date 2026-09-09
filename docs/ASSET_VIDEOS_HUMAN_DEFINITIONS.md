# Assessment of Human-Given Information for Input Videos in Plaque Forge

## Executive Summary & Architectural Philosophy

Plaque Forge is an automated and artistic video typography compositor that places, animates, and tracks titles on surfaces embedded in moving video scenes. Its core operational lifecycle is:

$$\\text{Setup Once} \\longrightarrow \\text{Analyze Once} \\longrightarrow \\text{Render Many} \\longrightarrow \\text{Review}$$

According to `docs/engineering-policy.md` and `docs/SCENES.md`, the repository enforces a strict, normative division of responsibilities:

1. **Human Intent (`assets/scenes/`, `assets/homologation/`, `assets/plaques/`, `assets/segmentation/`)**:
   - Stores human artistic declarations, geometric hypotheses, tracking constraints, and verified acceptance witnesses.
   - Files are sparse, version-controlled, human-readable (TOML/PNG), and immutable during automated re-analysis.
   - **Never** contains dense generated trajectories, machine confidence scores, temporal MAD maps, or self-certifying analysis caches.

2. **Machine Analysis & Caches (`assets/analysis/`, `output/`)**:
   - Fully reproducible, dense outputs produced by the Rust tracking pipeline and Python ML workers (SAM 2, Cutie, ViTMatte).
   - Designed to be safely erased at any moment via `./scripts/reset_analysis.sh --yes` without losing any human artistic intent.
   - Under the engineering policy, generated caches **can never serve as their own acceptance oracle**.

This assessment provides an in-depth, code-level analysis of all human-given data for the 19 input videos located under `./assets`, explaining what every field, number, coordinate, and mask means mechanically within the codebase, how they guide the pipeline, and concluding with critical observations on common patterns, container quirks, and review debt.

---

## 1. Structure & Semantics of Human-Authored Data

Human intent in Plaque Forge is expressed across five primary formats:

```
assets/
├── scenes/<asset>/scene.toml                   # plaque-forge.scene/1 (Primary Scene Intent)
│   ├── trajectory.toml                        # plaque-forge.trajectory/1 (Reviewed Dense Trajectory)
│   └── <layer-artifact>.toml / *.png          # plaque-forge.layer/1 (Static Reviewed Masks)
├── homologation/<asset>/contract.toml          # plaque-forge.homologation/1 (Acceptance Contracts)
│   └── <witness>/*.png                        # Sparse Reviewed Visual Witnesses
├── homologation/capabilities.toml              # plaque-forge.homologation-capabilities/1
├── homologation/segmentation-capabilities.toml # plaque-forge.segmentation-capabilities/1
├── plaques/catalog.toml                        # Reusable Injected Plaque Specifications
└── segmentation/policy.toml                    # plaque-forge.segmentation-policy/1
```

### 1.1 Scene Intent: `scene.toml` (`plaque-forge.scene/1`)

Parsed by `src/scene.rs` into `TitleSurface` and `SceneLayer` structures.

#### Surface Declarations (`[[surfaces]]`)

- **`id`** (`String`): Identifier for the surface (e.g., `"main"`). Referenced by layers (`surface = "main"`, `in_front_of = "main"`), render commands, and typography fitting.
- **`space`** (`SurfaceSpace`):
  - `scene-plane`: A physical, rigid planar surface in the 3D world undergoing camera motion, perspective, rotation, and scaling. The Rust solver computes an 8-DOF projective homography matrix ($H_t \in \mathbb{R}^{3 \times 3}$) for each frame, mapping reference-frame plane coordinates to source pixels.
  - `screen-canvas`: A fixed 2D graphic canvas anchored to camera viewport coordinates. Camera or background motion is deliberately ignored. Depth must be `flat`.
  - `scene-mesh`: Reserved for non-planar deformable surfaces. Currently rejected by `src/scene.rs` until a deformable mesh solver and independent verifier are implemented.
- **`depth`** (`DepthMode`):
  - `automatic`: Activates the full occluder discovery pipeline (`src/analyze/occlusion.rs`). Computes temporal MAD (Median Absolute Deviation) to discover transient objects crossing the plaque and dispatches Python ML workers (SAM 2 / Cutie).
  - `declared-only`: Suppresses automatic occluder discovery. The compositor only uses layers explicitly declared in `[[layers]]`. Used when background animation, illumination changes, or complex shadows would confuse automated change detection.
  - `flat`: Required for `screen-canvas`. Specifies zero depth complexity.
- **`reference_frame`** (`usize`):
  - The golden anchor frame index (0-indexed) where human coordinates and bounding boxes are specified.
  - In `src/analyze/candidate.rs` and `src/analyze/extraction.rs`, this frame provides the registration template and patch descriptors that initialize the forward/backward feature tracker.
- **`bounds`** (`[f64; 4]`):
  - Planar tracking bounding box `[x, y, width, height]` in source pixels at `reference_frame`.
  - **Mechanical Meaning**: Features (SIFT/ORB keypoints, edge gradients) are extracted within this rectangle to estimate the homography. It is intentionally kept larger than the writing faceplate so that high-contrast borders, mounting bolts, and bevels can stabilize tracking.
- **`writable_region`** (`WritableRegion`):
  - The precise geometric boundary where title typography may be rendered, completely decoupled from `bounds`. Validated by `src/writable_region.rs`:
    - `shape = "rect"`: Simple sub-rectangle `[x, y, w, h]`.
    - `shape = "rounded-rect"`: Rectangle with corner radius (`radius = r`).
    - `shape = "ellipse"`: Ellipse with `center = [cx, cy]`, `radii = [rx, ry]`, and `rotation_degrees = rot`.
    - `shape = "polygon"`: Polygon defined by an array of 2D points `[[x, y], ...]`.
    - `shape = "mask"`: Bitmap mask path (`writable-mask.png`) defining an arbitrary irregular faceplate.
  - *Constraint*: `src/scene.rs` enforces that `writable_region` must be strictly contained inside `bounds`.
- **`appearance`** (`SurfaceAppearance`):
  - `kind = "observed"` (default): Text is composited directly onto the natural video pixels.
  - `kind = "image"`: Injected plaque mode. An external transparent graphic asset (e.g. `../../plaques/aetherglass-aurora-16_9.png`) is composited onto the video canvas, with text bounded by `inset: [left, top, right, bottom]`.
- **`trajectory`** (`PathBuf`): Optional path to a reviewed `trajectory.toml`. Bypasses automatic tracking.
- **`anchors`** (`Vec<SurfaceAnchor>`): Sparse 4-corner quad locks (`[TL, TR, BR, BL]`) on specific frames, constraining the non-causal trajectory solver without replacing it entirely.

#### Layer Declarations (`[[layers]]`)

- **`id`** (`String`): Unique layer identifier (e.g., `"spider"`, `"chains"`, `"lizard"`, `"moss"`).
- **`role`** (`LayerRole`):
  - `foreground`: Restores source video pixels **above** rendered title text, creating the physical illusion that the object is in front of the typography.
  - `writing-surface`: Represents the plaque faceplate itself; used to constrain layout or gate surface visibility.
  - `shadow`: Cast shadow; blended via linear-light modulation rather than binary cutout.
  - `background`: Negative depth evidence; cannot occlude title text.
- **`surface`** / **`in_front_of`** (`String`): Topological stacking order relative to declared surfaces.
- **`artifact`** (`PathBuf`): Points to a pre-baked layer artifact manifest (`artifact.toml`, `moss-soft.toml`) containing static or sequential masks. Mutually exclusive with `prompts`.
- **`active_frames`** (`[usize; 2]`): Temporal interval `[first, last]` during which the layer is evaluated and composited. Saves compute and prevents false occlusions when objects exit the frame.
- **`affects_layout`** (`bool`, default: `true`):
  - If `true`, typography layout engines shrink or wrap text to avoid overlapping the layer.
  - If `false`, text flows freely behind the object, allowing the object to physically occlude letters.
- **`affects_tracking`** (`bool`, default: `true`):
  - If `true`, pixels under this layer are masked out during feature extraction so the moving object does not distort surface homography estimation.
  - If `false`, feature tracking ignores this layer (used when tracking is already pre-solved or frozen).
- **`matte`** (`LayerMatte`):
  - `mode = "optical"`: Continuous alpha blending for mist, glass, and soft boundaries.
  - `mode = "opaque"`: Treats ML confidence scores as semantic probabilities, remapping values between `support_threshold` (noise floor) and `solid_threshold` (100% opaque restoration) with a narrow feather.
- **`subject`** (`LayerSubject`): `unspecified` (generic SAM 2 / Cutie) vs `human` (specialized matting like MatAnyone2).
- **`[[layers.prompts]]`** (`SegmentationPrompt`): Seed prompts for Python ML segmentation:
  - `frame`: Target frame index.
  - `coordinates`: `source-pixels` or `normalized`.
  - `box_bounds`: Bounding box `[x, y, w, h]` guiding object localization.
  - `positive_points` / `negative_points`: Foreground/background click coordinates.

---

### 1.2 Reviewed Trajectory Intent: `trajectory.toml` (`plaque-forge.trajectory/1`)

Dense or sparse reviewed planar homography keyframes parsed by `src/scene.rs`:
- **`quad`**: Four ordered corners `[[TL_x, TL_y], [TR_x, TR_y], [BR_x, BR_y], [BL_x, BL_y]]` in source pixels defining the projection of the plaque surface.
- **`locked`**: Boolean flag freezing the quad against solver re-optimization.
- **`visibility`**: Scalar $[0.0, 1.0]$. Modulates title opacity. Crucial for modeling mechanical retraction into housings or fading when surfaces move off-screen.

---

### 1.3 Homologation Regression Contracts: `contract.toml` (`plaque-forge.homologation/1`)

Located under `assets/homologation/<asset>/contract.toml`. Defines human-accepted behavior that future engine refactors must preserve:

- **Identity & Provenance**: Pins `source_sha256`, `scene`, `analysis`, and surface names.
- **Geometry Contract**: Pins exact tracking bounds, writable bounds, and optional SHA-256 hashes for custom writable masks or trajectories.
- **Render & Typography Limits**: Pins text string, effect style file and SHA-256, font file (`NotoSerif-Regular.ttf`), fit mode (`artistic`), resolved line breaks, max font size, and enforces zero glyph clipping and zero missing glyphs.
- **Source Preservation Witnesses (`[[source_preservation]]`)**:
  - Validates that pixels covered by a reviewed mask match the source video within `maximum_mean_absolute_error` and `maximum_p95_absolute_error`.
  - Proves the foreground object remains **in front of** the rendered title.
- **Title Visibility Witnesses (`[[title_visibility]]`)**:
  - Validates that pixels in porous openings (e.g. spiderweb holes) remain visibly altered from the source (`minimum_mean_absolute_error`, `minimum_p50_absolute_error`).
  - Proves title text remains **visible through gaps** rather than being washed out by an over-conservative opaque mask.

---

### 1.4 Catalogs & Matrices

- **`assets/plaques/catalog.toml`**: Catalog of reusable injected plaque PNGs (`aetherglass-aurora` and `prismwraith-reliquary` in 16:9 and 9:16), defining pixel dimensions, writable insets, and SHA-256 hashes.
- **`assets/homologation/capabilities.toml`**: Maps the video set into 10 behavioral capabilities, selecting representative scenes and assigning continuous integration test priority (`ci = true`).
- **`assets/homologation/segmentation-capabilities.toml`**: Ledger tracking 6 ML-specific segmentation capabilities.
- **`assets/segmentation/policy.toml`**: Numerical acceptance thresholds for adaptive ML candidate escalation across `preview`, `balanced`, and `canonical` profiles.

---

## 2. Comprehensive Asset-by-Asset Assessment (All 19 Videos)

| # | Asset Stem | Aspect | Resolution | Decodable Frames | Duration | Surface Space & Depth | Homologation Status | CI Gate |
|---|---|---|---|---|---|---|---|---|
| 1 | `16_9_background_digifall` | 16:9 | 1280×720 | 240 | 10.005s | `screen-canvas` / `flat` | Contract | No |
| 2 | `16_9_dungeon_spider_iron_plaque` | 16:9 | 1280×720 | **234** (240 adv) | **9.795s** | `scene-plane` / `automatic` | Contract | No |
| 3 | `16_9_holographic_datacenter_static_plaque` | 16:9 | 1280×720 | 240 | 10.005s | `scene-plane` / `declared-only` | None | No |
| 4 | `16_9_mountain_top_day_hummingbird_cloudy_plaque` | 16:9 | 1280×720 | 240 | 10.005s | `scene-plane` / `declared-only` | Contract | No |
| 5 | `16_9_plaqueless_mountain_top_night` | 16:9 | 1280×720 | 240 | 10.005s | `screen-canvas` / `flat` | None | No |
| 6 | `16_9_plaqueless_swamp` | 16:9 | 1280×720 | 240 | 10.005s | `screen-canvas` / `flat` | None | No |
| 7 | `16_9_scrapyard_iron_plaque_foreground_chains` | 16:9 | 1280×720 | 240 | 10.005s | `scene-plane` / `declared-only` | Contract | **Yes** |
| 8 | `16_9_swamp_iron_plaque` | 16:9 | 1280×720 | 240 | 10.005s | `scene-plane` / `declared-only` | None | No |
| 9 | `16_9_swamp_wooden_plaque` | 16:9 | 1280×720 | 240 | 10.005s | `scene-plane` / `automatic` | Contract | **Yes** |
| 10 | `16_9_swamp_wooden_plaque_foreground_vines_and_lizard` | 16:9 | 1280×720 | 240 | 10.005s | `scene-plane` / `automatic` | None | No |
| 11 | `9_16_background_ogre_dear` | 9:16 | 720×1280 | **192** | **8.000s** | `screen-canvas` / `flat` | None | No |
| 12 | `9_16_dungeon_spider_iron_plaque` | 9:16 | 720×1280 | **206** (240 adv) | **8.625s** | `scene-plane` / `automatic` | Contract | **Yes** |
| 13 | `9_16_dungeon_spider_iron_temporary_plaque` | 9:16 | 720×1280 | 240 | 10.005s | `scene-plane` / `automatic` | Contract | **Yes** |
| 14 | `9_16_lonely_ogre_holographic_static_plaque` | 9:16 | 720×1280 | 240 | 10.005s | `scene-plane` / `declared-only` | None | No |
| 15 | `9_16_plaqueless_datacenter_lab` | 9:16 | 720×1280 | **236** (240 adv) | **9.875s** | `screen-canvas` / `flat` | None | No |
| 16 | `9_16_plaqueless_neon_datacenter_ground_hole` | 9:16 | 720×1280 | 240 | 10.005s | `screen-canvas` / `flat` | None | No |
| 17 | `9_16_scrappy_datacenter_holographic_plaque` | 9:16 | 720×1280 | 240 | 10.005s | `scene-plane` / `declared-only` | None | No |
| 18 | `9_16_swamp_wooden_plaque` | 9:16 | 720×1280 | 240 | 10.005s | `scene-plane` / `automatic` | None | No |
| 19 | `moving-holographic-plaque` | 9:16 | 720×1280 | 240 | 10.005s | `scene-plane` / `declared-only` | Contract | **Yes** |

---

### 2.1 `16_9_background_digifall`
- **Technical Attributes**: 1280×720, 24.0 fps, 240 frames, 10.005s duration.
- **Physical Scenario**: A text-free cascading digital matrix waterfall. There is no physical sign or planar mounting board.
- **Human Scene Definitions (`scene.toml`)**:
  - `space = "screen-canvas"`, `depth = "flat"`, `reference_frame = 10`.
  - `bounds = [34.0, 4.0, 990.0, 680.0]`: Broad screen bounding box.
  - `writable_region`: `shape = "ellipse"`, `center = [500.0, 344.0]`, `radii = [455.0, 318.0]`, `rotation_degrees = 0.0`.
- **Motion & Occlusion**: Screen-locked canvas (no projective tracking). No occluder layers.
- **Homologation Contract**: Homologated under `assets/homologation/16_9_background_digifall/contract.toml`.
  - Render: `"DIGITAL MATRIX FALL"` formatted as 3 lines with `styles/gold-shine.toml`, max font size 160.0 pt.
- **Capability Sentinel**: Representative asset for `non-rectangular-writable-region` in `capabilities.toml`.
- **Semantic Intent**: Demonstrates text placement directly onto complex animated backgrounds without a physical plaque, using elliptical constraints to keep typography harmoniously clustered in the screen center.

---

### 2.2 `16_9_dungeon_spider_iron_plaque`
- **Technical Attributes**: 1280×720, 24.0 fps, container advertises 240 packets, but **only 234 frames are decodable** (9.795s duration).
- **Physical Scenario**: A stone dungeon wall with an iron plaque. A large spider crawls horizontally from left to right across the plaque faceplate while thin spiderweb strands hang in the foreground.
- **Human Scene Definitions (`scene.toml`)**:
  - `space = "scene-plane"`, `depth = "automatic"`, `reference_frame = 10`.
  - `bounds = [190.0, 100.0, 935.0, 151.0]`: Narrow horizontal plaque tracking box.
  - `writable_region`: `shape = "rounded-rect"`, `bounds = [220.0, 120.0, 875.0, 111.0]`, `radius = 12.0`.
  - `trajectory = "trajectory.toml"`: Dense 234-keyframe reviewed trajectory matching the 234 decodable frames.
  - Layer `spider`: `role = "foreground"`, `affects_layout = false`, `affects_tracking = false`.
    - `matte = { mode = "opaque", support_threshold = 0.03, solid_threshold = 0.2 }`.
    - **21 ML guidance prompts** spaced every 12 frames (frames 0 to 233). Each prompt defines a $135 	imes 130$ bounding box, 5 positive clicks on spider body/legs, and 2 negative clicks on the iron surface.
- **Homologation Contract**: Homologated under `assets/homologation/16_9_dungeon_spider_iron_plaque/contract.toml`.
  - Render: `"WITH THE BIGGER POTENTIAL OF SEEING FURTHER"`, style `gold-shine.toml`.
  - **Dual-Witness Visual Invariants**:
    - 21 `source_preservation` witnesses for the crawling spider (`foreground/*.png`, MAE $\le 12.0$, P95 $\le 25.0$).
    - 3 `source_preservation` witnesses for web threads (`web-foreground/*.png` at frames 104, 112, 120).
    - 3 `title_visibility` witnesses in web gaps (`web-holes/*.png` at frames 104, 112, 120, requiring min MAE $\ge 30.0$, min P50 $\ge 15.0$).
- **Capability Sentinel**: Representative asset for `foreground-crossing-title` in `capabilities.toml` and `generic-opaque-foreground` / `temporal-reappearance` in `segmentation-capabilities.toml`.
- **Semantic Intent**: High-rigor test of moving foreground occlusions. The opaque spider body must occlude the rendered title, while the semi-transparent web threads preserve source pixels without turning web openings into opaque blocks.

---

### 2.3 `16_9_holographic_datacenter_static_plaque`
- **Technical Attributes**: 1280×720, 24.0 fps, 240 frames, 10.005s duration.
- **Physical Scenario**: A high-tech futuristic server room with blinking rack LEDs and a glowing holographic sign mounted between server cabinets.
- **Human Scene Definitions (`scene.toml`)**:
  - `space = "scene-plane"`, `depth = "declared-only"`, `reference_frame = 41`.
  - `bounds = [335.0, 36.0, 611.0, 132.0]`.
  - `writable_region`: Omitted (defaults to full tracking bounds).
- **Motion & Layers**: No trajectory override (uses automatic homography solver). No occluder layers.
- **Homologation Contract**: None.
- **Semantic Intent**: Serves as a baseline for static planar tracking in 16:9 against high-frequency blinking lights and floor reflections without foreground occlusions.

---

### 2.4 `16_9_mountain_top_day_hummingbird_cloudy_plaque`
- **Technical Attributes**: 1280×720, 24.0 fps, 240 frames, 10.005s duration.
- **Physical Scenario**: An alpine summit plaque under bright sunlight with shifting cloud shadows. In the second half of the video (frame 120 onward), a hummingbird swoops across the foreground.
- **Human Scene Definitions (`scene.toml`)**:
  - `space = "scene-plane"`, `depth = "declared-only"`, `reference_frame = 10`.
  - `bounds = [229.0, 36.0, 838.0, 190.0]`.
  - `writable_region`: `shape = "rounded-rect"`, `bounds = [250.0, 55.0, 796.0, 155.0]`, `radius = 48.0`.
  - Layer `cloud-writing-surface`: `role = "writing-surface"`, 5 ML prompts at frames 10, 80, 140, 200, 239.
  - Layer `late-foreground`: `role = "foreground"`, `artifact = "late-foreground/artifact.toml"`, `active_frames = [120, 239]`, `affects_tracking = true`.
- **Reviewed Artifacts**: `late-foreground/artifact.toml` + 120 alpha masks (`masks/000120.png` through `masks/000239.png`).
- **Homologation Contract**: Homologated under `assets/homologation/16_9_mountain_top_day_hummingbird_cloudy_plaque/contract.toml`.
  - Render: `"Codex gave me the bigger
potential of seeing
what others cannot see!"`, style `classic-glow.toml`.
  - 5 `source_preservation` witnesses at frames 120, 140, 157, 170, 196 (MAE $\le 16.0$, P95 $\le 32.0$).
- **Capability Sentinel**: Representative asset for `deforming-writing-surface-with-foreground-crossings` in `capabilities.toml`.
- **Semantic Intent**: Combines shifting ambient lighting/cloud reflections on the writing surface with a high-speed foreground crossing (the hummingbird). Setting `affects_tracking = true` ensures the tracker strips out the hummingbird so rapid wing flapping does not destabilize the plaque homography.

---

### 2.5 `16_9_plaqueless_mountain_top_night`
- **Technical Attributes**: 1280×720, 24.0 fps, 240 frames, 10.005s duration.
- **Physical Scenario**: Nighttime alpine summit under starlight; footage contains no physical sign.
- **Human Scene Definitions (`scene.toml`)**:
  - `space = "screen-canvas"`, `depth = "flat"`, `reference_frame = 0`.
  - `bounds = [180.0, 28.0, 920.0, 258.0]`.
  - `appearance`: `kind = "image"`, `image = "../../plaques/aetherglass-aurora-16_9.png"`, `inset = [0.08, 0.12, 0.08, 0.12]`.
- **Homologation Contract**: None.
- **Semantic Intent**: Demonstrates the injected plaque workflow for 16:9 widescreen. Injects the transparent Aetherglass Aurora plaque PNG and positions text within an 8% horizontal / 12% vertical inset.

---

### 2.6 `16_9_plaqueless_swamp`
- **Technical Attributes**: 1280×720, 24.0 fps, 240 frames, 10.005s duration.
- **Physical Scenario**: Misty mangrove swamp with murky water; contains no physical sign.
- **Human Scene Definitions (`scene.toml`)**:
  - `space = "screen-canvas"`, `depth = "flat"`, `reference_frame = 0`.
  - `bounds = [190.0, 25.0, 900.0, 252.0]`.
  - `appearance`: `kind = "image"`, `image = "../../plaques/aetherglass-aurora-16_9.png"`, `inset = [0.08, 0.12, 0.08, 0.12]`.
- **Homologation Contract**: None.
- **Semantic Intent**: Validates automated quiet-region placement algorithms over complex natural water reflections.

---

### 2.7 `16_9_scrapyard_iron_plaque_foreground_chains`
- **Technical Attributes**: 1280×720, 24.0 fps, 240 frames, 10.005s duration.
- **Physical Scenario**: An industrial scrapyard with an iron plaque on heavy machinery. Rusty steel chains swing across the foreground with independent pendulum motion, creating severe depth parallax.
- **Human Scene Definitions (`scene.toml`)**:
  - `space = "scene-plane"`, `depth = "declared-only"`, `reference_frame = 113`.
  - `bounds = [322.0, 46.0, 634.0, 133.0]`.
  - `writable_region`: `shape = "mask"`, `bounds = [384.0, 61.0, 515.0, 94.0]`, `path = "writable-mask.png"`.
  - Layer `chains`: `role = "foreground"`, `artifact = "chains/artifact.toml"`, `active_frames = [60, 116]`, `affects_layout = false`, `affects_tracking = false`.
- **Reviewed Artifacts**:
  - `writable-mask.png`: $515 	imes 94$ 8-bit grayscale bitmap carving out the exact iron plate face.
  - `chains/artifact.toml` + 240 pre-baked SAM2+Cutie+ViTMatte alpha masks (`masks/000000.png` to `masks/000239.png`).
- **Homologation Contract**: Homologated under `assets/homologation/16_9_scrapyard_iron_plaque_foreground_chains/contract.toml` (**CI Gate Sentinel**).
  - Pinned `writable_mask_sha256 = "bdb858357cb3df98222ed1d110bfc2fae8f7fd1d8ad994eac9ee8a30e21b939b"`.
  - 4 `source_preservation` witnesses on swinging chains at frames 76, 84, 92, 100 (MAE $\le 12.0$, P95 $\le 25.0$).
- **Capability Sentinel**: Representative asset for `foreground-parallax-on-moving-plaque` in `capabilities.toml`.
- **Semantic Intent**: Proves foreground depth ordering survives plaque motion and parallax, verifying bitmap-defined `writable_region` masking.

---

### 2.8 `16_9_swamp_iron_plaque`
- **Technical Attributes**: 1280×720, 24.0 fps, 240 frames, 10.005s duration.
- **Physical Scenario**: An iron plaque affixed to a dead tree in a swamp, overgrown with hanging moss clumps and cast shadows.
- **Human Scene Definitions (`scene.toml`)**:
  - `space = "scene-plane"`, `depth = "declared-only"`, `reference_frame = 83`.
  - `bounds = [323.0, 53.0, 640.0, 125.0]`.
  - Layer `moss`: `role = "foreground"`, `artifact = "moss-soft.toml"` (`moss-soft.png`, canonical plaque space).
  - Layer `moss-shadow`: `role = "shadow"`, `artifact = "moss-shadow.toml"` (`moss-shadow.png`, canonical plaque space).
- **Reviewed Artifacts**: `moss-soft.png` ($640 	imes 125$) and `moss-shadow.png` ($640 	imes 125$).
- **Homologation Contract**: None.
- **Semantic Intent**: Demonstrates canonical plaque-space static layering. Because the plaque is rigid, moss and its shadow do not require per-frame video masks; they are authored once in canonical texture space and projectively warped with the plaque.

---

### 2.9 `16_9_swamp_wooden_plaque`
- **Technical Attributes**: 1280×720, 24.0 fps, 240 frames, 10.005s duration.
- **Physical Scenario**: A weathered wooden plaque attached to swamp pilings with creeping moss and vines. A small lizard crawls across the bottom rim.
- **Human Scene Definitions (`scene.toml`)**:
  - `space = "scene-plane"`, `depth = "automatic"`, `reference_frame = 10`.
  - `bounds = [342.0, 50.0, 606.0, 193.0]`.
  - `writable_region`: `shape = "rounded-rect"`, `bounds = [382.0, 92.0, 486.0, 115.0]`, `radius = 18.0`.
  - Layer `plaque-foreground`: `role = "foreground"`, `affects_layout = false`.
    - 7 ML prompts: frame 10 (moss, left-vine, right-vine, lizard), frame 80 (lizard), frame 160 (lizard), frame 230 (lizard).
- **Homologation Contract**: Homologated under `assets/homologation/16_9_swamp_wooden_plaque/contract.toml` (**CI Gate Sentinel**).
  - Render: `"Nós que aqui estamos, por vós esperamos!"` in `gold-shine.toml`.
  - Invariants: Geometry bounds, 2-line layout, max font size 44.0 pt.
- **Capability Sentinel**: Representative asset for `static-scene-plane-plaque` in `capabilities.toml`.
- **Semantic Intent**: The benchmark for static wooden signage. Proves text fitting and gold-shine shader composition on natural wood grain.

---

### 2.10 `16_9_swamp_wooden_plaque_foreground_vines_and_lizard`
- **Technical Attributes**: 1280×720, 24.0 fps, 240 frames, 10.005s duration. Completely distinct video file from `16_9_swamp_wooden_plaque.mp4` (different SHA-256).
- **Physical Scenario**: An evolved swamp wooden plaque scene featuring heavy vine growth, contact shadows, and a crawling lizard.
- **Human Scene Definitions (`scene.toml`)**:
  - `space = "scene-plane"`, `depth = "automatic"`, `reference_frame = 72`.
  - `bounds = [327.0, 30.0, 641.0, 285.0]`.
  - Layer `writing-surface`: `role = "writing-surface"`, `artifact = "writing-surface.toml"` (`writing-surface.png`, `affects_layout = true`).
  - Layer `attached`: `role = "foreground"`, `artifact = "attached.toml"` (`attached.png`, `affects_layout = false`).
  - Layer `attached-shadow`: `role = "shadow"`, `artifact = "attached-shadow.toml"` (`attached-shadow.png`).
  - Layer `lizard`: `role = "foreground"`, 4 ML prompts at frames 0, 72, 144, 239.
- **Reviewed Artifacts**: `writing-surface.png` ($641 	imes 285$), `attached.png` ($641 	imes 285$), and `attached-shadow.png` ($641 	imes 285$).
- **Homologation Contract**: **None** (currently marked as review debt).
- **Capability Sentinel**: Declared representative for `multi-layer-foreground-and-shadow` in `capabilities.toml` and `soft-or-translucent-boundary` in `segmentation-capabilities.toml`.
- **Semantic Intent**: Decouples static foreground elements (vines, shadows) from moving dynamic foreground elements (the lizard), combining canonical texture masks with ML-tracked dynamic objects.

---

### 2.11 `9_16_background_ogre_dear`
- **Technical Attributes**: 720×1280 (portrait), 24.0 fps, **192 frames**, **8.000s duration** (anomalous frame count).
- **Physical Scenario**: Fantasy forest background showing an ogre and a deer in vertical portrait orientation without a sign.
- **Human Scene Definitions (`scene.toml`)**:
  - `space = "screen-canvas"`, `depth = "flat"`, `reference_frame = 10`.
  - `bounds = [20.0, 104.0, 494.0, 423.0]`.
  - `writable_region`: `shape = "ellipse"`, `center = [267.0, 315.5]`, `radii = [218.0, 178.0]`, `rotation_degrees = 0.0`.
- **Homologation Contract**: None.
- **Semantic Intent**: Portrait counterpart to `16_9_background_digifall`. Tests elliptical line-breaking constraints in vertical video.

---

### 2.12 `9_16_dungeon_spider_iron_plaque`
- **Technical Attributes**: 720×1280, 24.0 fps, 240 packets advertised, but **only 206 frames are decodable** (8.625s duration).
- **Physical Scenario**: Vertical portrait dungeon wall with iron plaque, a climbing spider, and spiderweb threads.
- **Human Scene Definitions (`scene.toml`)**:
  - `space = "scene-plane"`, `depth = "automatic"`, `reference_frame = 10`.
  - `bounds = [128.0, 159.0, 464.0, 280.0]`.
  - `writable_region`: `shape = "rounded-rect"`, `bounds = [160.0, 190.0, 401.0, 215.0]`, `radius = 22.0`.
  - `trajectory = "trajectory.toml"`: Dense 206-keyframe reviewed trajectory.
  - Layer `spider`: `role = "foreground"`, 10 ML prompts from frame 108 to 204.
  - Layer `iron-writing-surface`: `role = "writing-surface"`, 6 prompts from frame 10 to 205.
- **Homologation Contract**: Homologated under `assets/homologation/9_16_dungeon_spider_iron_plaque/contract.toml` (**CI Gate Sentinel**).
  - Pinned `trajectory_sha256 = "f1b3f9f6f4cd16c4de9a26c9dfc09b9080573660bf7dfd06a058246bd2556c50"`.
  - 13 `source_preservation` spider witnesses (`foreground/*.png`, frames 108–204).
  - 3 `source_preservation` web thread witnesses (`web-foreground/*.png`).
  - 3 `title_visibility` web gap witnesses (`web-holes/*.png`).
- **Capability Sentinel**: Representative asset for `portrait-scene-plane-geometry` in `capabilities.toml`.
- **Semantic Intent**: Guarantees that portrait video coordinate handling remains mathematically decoupled from 16:9 widescreen assumptions.

---

### 2.13 `9_16_dungeon_spider_iron_temporary_plaque`
- **Technical Attributes**: 720×1280, 24.0 fps, 240 frames, 10.005s duration.
- **Physical Scenario**: A mechanical dungeon apparatus where an iron plaque slides vertically upward, retracting inside an iron instrument housing.
- **Human Scene Definitions (`scene.toml`)**:
  - `space = "scene-plane"`, `depth = "automatic"`, `reference_frame = 10`.
  - `bounds = [143.0, 173.0, 435.0, 253.0]`.
  - `writable_region`: `shape = "rounded-rect"`, `bounds = [161.0, 192.0, 399.0, 215.0]`, `radius = 22.0`.
  - `trajectory = "trajectory.toml"`: 240-keyframe trajectory where plaque quad top y shifts from 173 to 23, and `visibility` drops from 1.0 (frames 0–60) to 0.0 (frames 99–239).
  - Layer `instrument-housing`: `role = "foreground"`, `artifact = "instrument-housing/artifact.toml"`, `active_frames = [32, 71]`, `matte = { mode = "opaque", support_threshold = 0.03, solid_threshold = 0.2 }`.
- **Reviewed Artifacts**: `instrument-housing/artifact.toml` + 240 sequence masks (`masks/000000.png` through `masks/000239.png`).
- **Homologation Contract**: Homologated under `assets/homologation/9_16_dungeon_spider_iron_temporary_plaque/contract.toml` (**CI Gate Sentinel**).
  - 2 `source_preservation` witnesses on the housing lip at frames 52 and 64 (requiring 70,000 selected pixels, MAE $\le 12.0$).
- **Capability Sentinel**: Representative asset for `temporary-retracting-plaque-occlusion` in `capabilities.toml`.
- **Semantic Intent**: Proves that a retracting plaque disappears behind its physical enclosure without title pixels leaking or ghosting over the housing lip.

---

### 2.14 `9_16_lonely_ogre_holographic_static_plaque`
- **Technical Attributes**: 720×1280, 24.0 fps, 240 frames, 10.005s duration.
- **Physical Scenario**: Vertical portrait scene showing an animated ogre moving in the background behind a static holographic sign.
- **Human Scene Definitions (`scene.toml`)**:
  - `space = "scene-plane"`, `depth = "declared-only"`, `reference_frame = 207` (anomalous late anchor).
  - `bounds = [87.0, 208.0, 548.0, 391.0]`.
  - `writable_region`: Omitted.
- **Homologation Contract**: **None** (review debt).
- **Capability Sentinel**: Declared representative for `background-motion-independent-of-plaque` in `capabilities.toml`.
- **Semantic Intent**: Validates that strong motion in the background (the walking ogre) does not hijack the feature tracker or pull the static plaque off lock.

---

### 2.15 `9_16_plaqueless_datacenter_lab`
- **Technical Attributes**: 720×1280, 24.0 fps, 240 packets advertised, **236 playable frames** (9.875s duration).
- **Physical Scenario**: Vertical server corridor without a plaque.
- **Human Scene Definitions (`scene.toml`)**:
  - `space = "screen-canvas"`, `depth = "flat"`, `reference_frame = 0`.
  - `bounds = [70.0, 85.0, 580.0, 162.0]`.
  - `appearance`: `kind = "image"`, `image = "../../plaques/aetherglass-aurora-9_16.png"`, `inset = [0.08, 0.12, 0.08, 0.12]`.
- **Homologation Contract**: None.
- **Semantic Intent**: Injected plaque baseline for 9:16 vertical video using the portrait Aetherglass Aurora plaque.

---

### 2.16 `9_16_plaqueless_neon_datacenter_ground_hole`
- **Technical Attributes**: 720×1280, 24.0 fps, 240 frames, 10.005s duration.
- **Physical Scenario**: Vertical futuristic server room with glowing neon floor apertures.
- **Human Scene Definitions (`scene.toml`)**:
  - `space = "screen-canvas"`, `depth = "flat"`, `reference_frame = 0`.
  - `bounds = [70.0, 70.0, 580.0, 162.0]`.
  - `appearance`: `kind = "image"`, `image = "../../plaques/aetherglass-aurora-9_16.png"`, `inset = [0.08, 0.12, 0.08, 0.12]`.
- **Homologation Contract**: None.
- **Semantic Intent**: Injected plaque variant testing quiet placement near glowing neon fixtures.

---

### 2.17 `9_16_scrappy_datacenter_holographic_plaque`
- **Technical Attributes**: 720×1280, 24.0 fps, 240 frames, 10.005s duration.
- **Physical Scenario**: Cluttered vertical datacenter with a holographic terminal plaque.
- **Human Scene Definitions (`scene.toml`)**:
  - `space = "scene-plane"`, `depth = "declared-only"`, `reference_frame = 0`.
  - `bounds = [144.0, 176.0, 433.0, 250.0]`.
  - `writable_region`: Omitted.
- **Homologation Contract**: None.
- **Semantic Intent**: Static planar plaque baseline in vertical orientation amidst visual clutter.

---

### 2.18 `9_16_swamp_wooden_plaque`
- **Technical Attributes**: 720×1280, 24.0 fps, 240 frames, 10.005s duration.
- **Physical Scenario**: Vertical portrait wooden plaque in a swamp environment with climbing vines, moss, and crawling lizard.
- **Human Scene Definitions (`scene.toml`)**:
  - `space = "scene-plane"`, `depth = "automatic"`, `reference_frame = 10`.
  - `bounds = [0.0, 64.0, 720.0, 380.0]`: Plaque spans entire horizontal width ($x=0$, $w=720$).
  - `writable_region`: `shape = "rounded-rect"`, `bounds = [55.0, 105.0, 610.0, 290.0]`, `radius = 30.0`.
  - Layer `plaque-foreground`: 7 ML prompts identical in structure to the 16:9 counterpart (moss, left-vine, right-vine, lizard).
- **Homologation Contract**: None.
- **Semantic Intent**: Portrait counterpart to `16_9_swamp_wooden_plaque`, testing full-bleed horizontal bounds where the sign edges touch the frame boundaries.

---

### 2.19 `moving-holographic-plaque`
- **Technical Attributes**: 720×1280 (portrait), 24.0 fps, 240 frames, 10.005s duration. Note the missing `9_16_` prefix.
- **Physical Scenario**: A floating holographic glass plaque undergoing rapid 3D camera sweeps, rotation, and partial off-screen excursion.
- **Human Scene Definitions (`scene.toml`)**:
  - `space = "scene-plane"`, `depth = "declared-only"`, `reference_frame = 72`.
  - `bounds = [84.0, 55.0, 557.0, 321.0]`.
  - `trajectory = "trajectory.toml"`: 240-keyframe dense trajectory.
  - `writable_region`: `shape = "mask"`, `bounds = [113.0, 93.0, 499.0, 260.0]`, `path = "writable-mask.png"`.
- **Reviewed Artifacts**: `writable-mask.png` ($499 	imes 260$).
- **Homologation Contract**: Homologated under `assets/homologation/moving-holographic-plaque/contract.toml` (**CI Gate Sentinel**).
  - Pinned `writable_mask_sha256 = "9531fa143f031661d392c756c44ccb49d486d6c40525a4092aec4a475c0c63fa"`.
  - Pinned `trajectory_sha256 = "1f1389f6eed79884213aea78e877f69f927bd5cf1942a2957ddc789fd39f7e9a"`.
- **Capability Sentinel**: Representative asset for `projective-moving-plaque` in `capabilities.toml`.
- **Semantic Intent**: Benchmark for extreme projective homography tracking and partial off-screen camera clipping.

---

## 3. Final Considerations: Patterns, Anomalies, and Review Debt

Cross-referencing the human definitions, raw media containers, and Rust codebase reveals key architectural patterns and several notable anomalies:

### 3.1 Architectural & Design Patterns

1. **Strict Decoupling of Tracking from Typography**:
   In every physical sign, `bounds` is broader than `writable_region`. In `16_9_dungeon_spider_iron_plaque`, `bounds` is $935 	imes 151$ px while `writable_region` is $875 	imes 111$ px with a 12px corner radius. Outer bolts and bevels provide high-contrast tracking features without allowing title text to bleed onto them.

2. **Compositing Decoupled from Tracking**:
   - `affects_layout = false` allows typography to flow under moving occluders (spiders, chains, lizards) rather than awkwardly shrinking the font.
   - `affects_tracking = false` prevents moving occluders from corrupting previously accepted homographies.
   - `affects_tracking = true` in `16_9_mountain_top_day_hummingbird_cloudy_plaque` explicitly masks the fast-flapping hummingbird so the automatic tracker ignores it.

3. **Dual-Witness Verification of Porous Material**:
   For porous foregrounds like the dungeon spiderweb, verifying source preservation alone is insufficient. Pairing `source_preservation` witnesses on web threads with `title_visibility` witnesses in web holes enforces that text is truly visible through the gaps.

---

### 3.2 Suspicious Findings, Container Quirks & Anomalies

#### 1. The Truncated Stream & Phantom Tail Anomaly
Three video assets exhibit a severe discrepancy between container packet metadata and actual decodable frames:
- `16_9_dungeon_spider_iron_plaque.mp4`: Advertises 240 packets, but stream duration is 9.790039s $\rightarrow$ **exactly 234 decodable frames**.
- `9_16_dungeon_spider_iron_plaque.mp4`: Advertises 240 packets, but stream duration is 8.620036s $\rightarrow$ **exactly 206 decodable frames**.
- `9_16_plaqueless_datacenter_lab.mp4`: Advertises 240 packets, but stream duration is 9.870036s $\rightarrow$ **exactly 236 decodable frames**.

**Codebase Confirmation**:
This is a known media flaw handled explicitly in `src/video.rs`:
```rust
// Lines 246-251 of src/video.rs:
// Preserve the decoder's actual frame sequence. FFmpeg's default
// output synchronization may duplicate frames to fill a nominal
// container duration (for example 234 decodable frames advertised
// as 240), which makes a verifier compare synthetic tail frames.
"-fps_mode", "passthrough"

// Lines 412-424 of src/video.rs (select_playable_frame_count):
(Some(metadata), duration) if duration > 0 && metadata > duration + 1 => duration,
```
The test suite in `src/video.rs` (lines 471–482) explicitly unit tests the numbers `234`, `206`, and `236`. The human scene definitions reflect this: `16_9_dungeon_spider_iron_plaque/trajectory.toml` has exactly 234 keyframes, and `9_16_dungeon_spider_iron_plaque/trajectory.toml` has exactly 206 keyframes.

#### 2. Filename Inconsistency: `moving-holographic-plaque.mp4`
18 of the 19 videos start with their aspect ratio prefix (`16_9_...` or `9_16_...`). `moving-holographic-plaque.mp4` is the sole exception, lacking the `9_16_` prefix despite being a $720 	imes 1280$ video.

#### 3. Duration Outlier: `9_16_background_ogre_dear.mp4`
While 18 videos have a nominal length of 240 frames (10 seconds), `9_16_background_ogre_dear.mp4` contains only **192 frames (8.0 seconds)**. Any script assuming 10-second clips will fail on this asset.

#### 4. Anomalous Late Reference Frame in `9_16_lonely_ogre_holographic_static_plaque`
Most scenes set `reference_frame` to frame 0, 10, 41, 72, or 83. However, `9_16_lonely_ogre_holographic_static_plaque` chooses `reference_frame = 207` (near the end of a 240-frame video). This was selected because the ogre's animated motion creates severe occlusion earlier in the clip, leaving frame 207 as the only clean window for plaque identification.

#### 5. Prompt Bounding Box Clamping at $x = 0.0$
In `16_9_mountain_top_day_hummingbird_cloudy_plaque`, prompts at frames 200 and 239 set `box_bounds = [0.0, 12.0, 970.0, 245.0]`. The bounding box starts exactly at the left video boundary ($x=0.0$) and spans almost the entire width ($970$ px), indicating the cloud writing surface drifts completely off the left edge of the screen.

#### 6. Temporal Activation Clamping vs. Mask Artifact Length
- In `9_16_dungeon_spider_iron_temporary_plaque`, `instrument-housing/artifact.toml` provides 240 masks (frames 0–239). However, `scene.toml` clamps `active_frames = [32, 71]`.
- In `16_9_scrapyard_iron_plaque_foreground_chains`, `chains/artifact.toml` contains 240 masks, but `scene.toml` clamps `active_frames = [60, 116]`.
This is an intentional optimization: the masks were generated for the whole video, but human review determined the crossing only intersects text pixels during the clamped window.

#### 7. Off-Screen Trajectory Negative Coordinates
In `moving-holographic-plaque/trajectory.toml`, frames 124 and 125 exhibit negative coordinates (`quad` top y = $-32.9$ and $-36.5$) as the plaque dips above the top edge of the video frame. During these two frames, `visibility` drops abruptly from $1.0$ down to $0.088369915$, testing the renderer's ability to clip and restore surfaces exiting the viewport.

---

### 3.3 Homologation & Capability Review Debt

1. **Unprotected Scenes**:
   Only **8 of the 19 videos** have executable homologation contracts (`contract.toml`). The remaining 11 videos (`16_9_holographic_datacenter_static_plaque`, `16_9_plaqueless_*`, `16_9_swamp_iron_plaque`, `16_9_swamp_wooden_plaque_foreground_vines_and_lizard`, `9_16_background_ogre_dear`, `9_16_lonely_ogre_holographic_static_plaque`, `9_16_plaqueless_*`, `9_16_scrappy_datacenter_holographic_plaque`, `9_16_swamp_wooden_plaque`) have no regression contracts.

2. **Capability Coverage Gap (`capabilities.toml`)**:
   `plaque-forge homologation-coverage` reports:
   ```json
   {
     "capabilities": 10,
     "homologated": 8,
     "ci_protected": 5,
     "complete": false
   }
   ```
   Two capabilities lack human contracts:
   - `background-motion-independent-of-plaque` (Representative: `9_16_lonely_ogre_holographic_static_plaque`)
   - `multi-layer-foreground-and-shadow` (Representative: `16_9_swamp_wooden_plaque_foreground_vines_and_lizard`)

3. **Segmentation Capability Debt (`segmentation-capabilities.toml`)**:
   `python3 tools/check_segmentation_capabilities.py` reports:
   ```json
   {
     "capabilities": 6,
     "represented": 4,
     "final_homologated": 1,
     "complete": false
   }
   ```
   Two capabilities (`human-fine-detail` and `generic-open-vocabulary`) have **empty string representative assets** and zero contract coverage, representing open review debt for future model integrations (e.g. SAM 3.1 and MatAnyone2).

---

## Conclusion

The human-authored information in Plaque Forge is engineered with strict mathematical discipline and architectural separation of concerns. Rather than relying on fragile whole-video pixel comparisons or monolithic configuration files, the system splits intent into:
1. Spatial planar constraints (`bounds` vs `writable_region`),
2. Compositing vs tracking decoupling (`affects_layout` vs `affects_tracking`),
3. Temporal windowing (`active_frames`),
4. Categorical vs optical alpha remapping (`LayerMatte`), and
5. Invariant regression sentinels (`contract.toml`).

The anomalies identified—such as container stream truncations (234, 206, and 236 frames), offscreen quad excursions, and layer activation clamping—demonstrate that the codebase has been explicitly hardened to handle the real-world idiosyncrasies of video decoding and complex visual tracking.
