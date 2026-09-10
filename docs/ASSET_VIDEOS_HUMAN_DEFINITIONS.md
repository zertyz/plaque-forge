# Independent assessment of the human definitions for the asset videos

Assessment date: **2026-09-10**. Repository revision examined: `0b8f2ed7e7057f7c946001f290df58618b59b32f`.

This assessment examines the actual videos, scene definitions, associated masks and trajectories, and regression contracts under `assets/`, together with the code that consumes them. The similarly named document under `docs/` was not used as an authority or as the basis for this report.

The central finding is that these videos form a **substantially curated rendering collection**. Every video already has a selected title surface. Some definitions provide a placement and leave motion or depth estimation to the software; others supply a complete motion solution, detailed foreground evidence, or an explicit disappearance schedule. Successful processing of this collection therefore demonstrates different mixtures of automation and supplied knowledge, depending on the video.

The checked data is internally consistent in several important respects: source references resolve correctly, supplied trajectories cover the actual decoded videos, and the checked source, artwork, trajectory, and homologation-witness hashes match. The more interesting concerns are about **what the definitions promise, how the implementation interprets them, and how much independent acceptance evidence exists**.

Reading guide: [scope and inventory](#scope-and-meaning-of-human-definitions) · [meaning in the code](#how-the-definitions-become-visible-behavior) · [shared artwork and assumptions](#shared-artwork-and-global-assumptions) · [all 19 videos](#video-by-video-assessment) · [homologation evidence](#what-the-homologation-data-actually-certifies) · [verification limits](#verification-performed-and-limits) · [final considerations](#final-considerations).

## Scope and meaning of “human definitions”

Here, “human definitions” means project-authoritative instructions or accepted evidence, rather than a claim that a person manually painted every pixel or typed every coordinate. Several scene artifacts explicitly record an algorithm or ML model as their generator. Their role as supplied scene evidence comes from how the project stores and references them.

The scene and artifact records do not provide enough information to reconstruct who reviewed every artifact, when they reviewed it, or which exact workflow produced every accepted mask. Comments saying “human-reviewed” are evidence of the project's stated status; they are not independently verifiable review records.

| Location | What it means in this assessment |
|---|---|
| `assets/*.mp4` | The source imagery and its timeline. Filenames are descriptive identifiers, not executable descriptions of depth or motion. |
| `assets/scenes/<video>/scene.toml` | The selected surface, geometry, depth policy, layer identities, prompts, and references to supplied artifacts. |
| Trajectories and masks referenced from `assets/scenes/` | Supplied motion or image-space evidence. These are inputs to analysis even when originally produced with software. |
| `assets/analysis/<video>/` | Generated analysis, including motion samples, estimated masks, summaries, and worker provenance. Useful for checking interpretation, but not an independent human judgment of correctness. |
| `assets/homologation/` | Accepted behavioral contracts and selected visual witnesses. These describe what specified example renders must preserve. |
| `assets/plaques/catalog.toml` and its PNGs | Reusable artwork and its intended writable margins. Four scenes explicitly choose one of these images. |
| `assets/segmentation/policy.toml` | Project-authored acceptance thresholds for generated segmentation candidates. This is global policy, not an annotation of a particular video. |

The supplied scene collection contains:

- **19 videos and 19 matching scene manifests**, with one surface named `main` in each.
- **13 physical scene planes** and **6 screen canvases**. Nine of the physical planes leave motion estimation to the implementation; four have complete locked trajectories.
- **10 explicit writable shapes**: six rounded rectangles, two ellipses, and two image masks. Four further scenes derive a writable rectangle from injected-artwork insets. Five observed surfaces omit `writable_region`; one of those five also supplies a separate writing-surface layer that constrains layout.
- **15 declared layers**: ten foreground layers, three writing-surface layers, and two shadow layers. Seven layers use prompts, containing 60 prompt records altogether; eight reference existing artifacts.
- **607 PNGs under `assets/scenes/`**: two writable-region images, five static layer images, and 600 images belonging to three foreground sequences. Many sequence images are intentionally empty.
- **8 homologation contracts**, with 55 source-preservation witnesses and six complementary title-visibility witnesses. Five representative renders are selected for the continuous homologation script.

These counts describe the checked snapshot. In particular, a directory containing hundreds of PNGs is not evidence of hundreds of independent human annotations.

The sources comprise ten landscape videos at `1280 × 720` and nine portrait videos at `720 × 1280`, including `moving-holographic-plaque`. Most clips have ten seconds of decoded imagery; shorter clips and misleading container frame counts are explained in their individual entries.

## How the definitions become visible behavior

### Scene identity and selection

All manifests use `format = "plaque-forge.scene/1"`. This is the schema identifier. `source` identifies the video relative to the manifest; `default_surface = "main"` selects the only declared surface unless a caller explicitly requests one.

The normal workflow finds a scene by the video's stem. It then checks that the manifest's source is the actual input file. Changing the spelling of an asset stem can therefore break automatic discovery even though words such as `plaque`, `static`, or `foreground` have no special meaning to the renderer. See [scene loading](src/scene.rs), [workspace paths](src/workspace.rs), and [analysis input resolution](src/analyze/mod.rs).

**All 19 scenes already choose the surface location.** Although the schema describes `bounds` as a tracking hint, the current [candidate detector](src/analyze/candidate.rs), in `detect`, accepts supplied bounds directly and bypasses automatic proposal ranking. The normal bundled workflow consequently does not independently discover which object should receive the title.

Programmatic or direct CLI hints can take precedence over the scene's geometry. This report describes the normal workflow using the checked scene files, without such overrides.

### Coordinates, reference frames, and writable space

| Field or concept | Practical meaning |
|---|---|
| `reference_frame` | The frame associated with the supplied placement. It is a geometry reference, not a title start time or a requirement to ignore earlier frames. |
| `bounds = [x, y, width, height]` | An enclosing rectangle in source-video pixels. The last two values are dimensions, not bottom-right coordinates. |
| `writable_region` | Where typography may be laid out inside that enclosing surface. It is distinct from the outer object used for tracking. |
| `rounded-rect` | A deliberately inset writing area with excluded rounded corners; its radius is expressed in source pixels. |
| `ellipse` | A title silhouette defined by center and radii. It need not represent a physical elliptical object. |
| `shape = "mask"` | The image's grayscale values define the writable area within the supplied source-space bounds. The bitmap can be the size of the inner cavity rather than the full video. |
| `plaque-canonical` layer coordinates | Coordinates on the rectified surface before it is warped into a video frame. Static attached material can follow the plaque using the same motion. |
| `source-pixels` layer coordinates | Coordinates in the full video image at a particular frame. These allow an object to move independently of the plaque. |

For example, the landscape dungeon's outer rectangle begins at `(190, 100)` and is `935 × 151` pixels. Its right edge is therefore at `1125`, not `935`. The smaller writing rectangle leaves the metal frame outside the typography. This distinction is much more meaningful than treating both rectangles as competing estimates of the same boundary.

Frames are zero-based. All source videos use 24 frames per second, so frame 120 is approximately five seconds into a clip. Coordinates refer to the original video dimensions, not a preview thumbnail. The prompts all explicitly use source pixels; none uses normalized coordinates. Resizing, cropping, or retiming a source requires revisiting these definitions.

An explicit writable shape replaces the automatically recovered content mask before applicable layout layers are applied. An observed surface with no explicit shape instead depends on automatic cavity reconstruction. Thus, omitting `writable_region` does **not** mean that every pixel of `bounds` is safe for text. See [writable-region rasterization](src/writable_region.rs), `apply_surface_intent` in [analysis](src/analyze/mod.rs), and [surface extraction](src/analyze/extraction.rs).

### Motion space and depth are separate choices

| Setting | What it instructs the system to do |
|---|---|
| `space = "scene-plane"` | Attach the title to a planar surface in the scene. A quadrilateral transformation supplies translation, rotation, scale, and perspective. This remains the model even when the photographed material itself is irregular or changes appearance. |
| `space = "screen-canvas"` | Keep the title placement fixed in the video image. Camera movement and background animation do not move it. |
| `depth = "automatic"` | Allow automatic foreground discovery in addition to any supplied layers. This is a compositing policy, not a measured 3D distance. |
| `depth = "declared-only"` | Suppress general automatic foreground discovery and rely on the declared layers. Source-conditioned processing of supplied opaque foreground can still occur. |
| `depth = "flat"` | Treat the placement as having no scene depth. All six screen canvases use this setting and have no foreground layers. |

The seven `declared-only` surfaces are not necessarily motionless. In particular, the moving holographic plaque is a physical plane with an explicit trajectory, despite having no declared occluders. Conversely, the injected plaques remain fixed to the screen even when the underlying video moves dramatically.

None of the scenes requests a deformable mesh. The implementation currently rejects `scene-mesh`; the cloud scene uses a planar title with supporting masks, not a mesh that bends each glyph with the cloud. See [schema validation](src/scene.rs) and [tracking selection](src/analyze/mod.rs).

### Supplied trajectories and visibility

The four `trajectory.toml` files use `plaque-forge.trajectory/1`. Every keyframe supplies four corners in **top-left, top-right, bottom-right, bottom-left** order, a locked status, and a visibility value. All four files identify the exact source video through a matching SHA-256 hash.

In this collection, these are not sparse motion hints: each trajectory contains one locked quad for every decoded frame. Their respective lengths are 234, 206, 240, and 240 frames. The [analysis workflow](src/analyze/mod.rs) selects the dense-trajectory loader instead of estimating their initial motion automatically.

`locked = true` means the supplied geometry is authoritative. The schema also supports guide keyframes and sparse anchors, but none of the checked scenes uses those alternatives. `locked` defaults to true when omitted.

`visibility` controls the opacity of the inserted title, and of injected artwork when present. It does not erase the original plaque from the source video, and it is not merely a confidence score. A value of zero hides the added title even if its quad is still inside the frame. A partly offscreen quad also does not inherently require fading all remaining visible text: clipping and visibility are separate concepts. See [dense motion loading](src/analyze/tracking/mod.rs), [visibility constraints](src/analyze/tracking/constraints.rs), and [compositing](src/render/mod.rs).

There is a related reporting caution: the dense loader assigns values such as high confidence and zero reprojection error to its supplied measurements. Likewise, the candidate detector assigns confidence values to human-selected bounds. These are implementation conventions for accepted input, not fresh measurements proving pixel-perfect visual correctness.

### Layers describe different responsibilities

| Layer role used here | Meaning |
|---|---|
| `writing-surface` | Positive evidence of the surface material. Depending on its coordinate system and flags, it restricts layout or supports selecting material points for tracking. It does not paint a replacement object into the source. |
| `foreground` | Material that should cover inserted typography. Rendering restores the corresponding source pixels after adding the title. |
| `shadow` | Existing source shading that should remain over the added title. The current rendering path restores source pixels through this mask; it does not calculate a new physically simulated shadow. |

`surface = "main"` associates a layer with the selected title surface. All foreground layers also declare `in_front_of = "main"`, making the intended relationship readable. The implementation validates that this target exists, but compositing is driven by the selected surface and layer role; these files are not a general multi-object 3D ordering graph.

Two flags distinguish layout from motion:

- **`affects_layout = false`** means the object can cover letters without reserving permanent empty space in the title composition. Fourteen of the fifteen layers declare this. The one layout-affecting layer is the static writing-surface mask for the landscape vines-and-lizard scene.
- **`affects_tracking = false`** keeps that declared layer out of the tracking-exclusion machinery. It does not disable its visible occlusion, and it does not globally disable automatic tracking or depth analysis. Both explicit spider layers, the chains, and the instrument housing use this setting.

Both flags default to true. A true tracking flag also needs to be interpreted with the artifact coordinates: the tracking-exclusion path consumes source-pixel layers, whereas the static canonical moss and attached-material masks primarily follow the already established plaque motion.

Writing-surface support is not an unconditional alternative pose estimate. The [layer implementation](src/layers.rs), in `build_tracking_exclusions`, checks its overlap and plausibility against an existing tracked plane before using it to exclude other material. This matters for the cloudy and dark-iron surfaces, where segmentation can otherwise drift to unrelated material.

### Prompts are guidance, not complete outlines

Each prompt names a frame and an object identity. Positive points say “include material here”; negative points say “exclude material here.” A box narrows the search, while a polygon can supply a much stronger initial shape. The polygon prompts on the portrait iron plaque are therefore more specific than the point-and-box prompts used during later occlusion.

The `object` strings also keep multiple identities separate within a layer. For example, `moss`, `left-vine`, `right-vine`, and `lizard` are separate identities even though the resulting layer is a combined foreground mask. In the [SAM2 worker](tools/segmentation_worker.py), these strings become object IDs. Naming an object `spider` does not by itself supply a natural-language spider detector. No checked prompt uses the optional open-vocabulary `concept` field.

Repeated prompts are temporal corrections for the same identity. The landscape spider has 21 prompts, mostly half a second apart. The woodland lizards generally have four temporal observations. Between those observations, the system still has to propagate and refine the masks; the annotations do not directly supply all intermediate silhouettes.

Prompted layers are generated under `assets/analysis/<video>/layers/<layer>/`. Their scene definitions are the inputs; the resulting PNG sequences and worker reports are generated evidence. The normal [analysis script](scripts/analyze_assets.sh) asks the worker to materialize needed layers. A pure-Rust run can reuse valid generated layers or skip unavailable ones, so the same scene file does not guarantee identical foreground support under every execution mode.

### Alpha, opacity, and mask timing

An alpha mask represents a degree of membership or restoration. Black means no contribution; white means full contribution; intermediate values represent a partial contribution. For layer images with an alpha channel, the loader uses that channel; otherwise it uses luminance. The separate writable-region image path explicitly reads luminance.

The default matte mode is **`optical`**: intermediate values remain intermediate. The moss, attached vegetation, chains, and shadow layers use this default. The shadow images are deliberately faint: their maxima are only about one fifth of full alpha. They should not be interpreted as failed attempts to create an opaque mask.

Four foreground layers explicitly use **`opaque`**. In this mode, mask values are treated as confidence about opaque material and remapped to a solid occluder with a soft transition. The landscape spider, late cloud foreground, and instrument housing reach full occlusion at a relatively low confidence of 0.20, discarding support below 0.03. The portrait spider uses a much more selective 0.35-to-0.70 transition. This difference materially affects fine legs, halos, and whether uncertain surrounding material hides text. It is not a declaration that the portrait spider is naturally more transparent. See `apply_matte_policy` in [layers](src/layers.rs).

The imported layer manifests use `format = "plaque-forge.layer/1"`. `kind = "alpha-image"` supplies one static image; `alpha-sequence` supplies a changing full-frame mask. `first_frame`, `last_frame`, and a pattern such as `masks/%06d.png` describe the stored sequence. Endpoints are inclusive; `%06d` is a zero-padded frame number, not a timestamp.

Two implementation details deserve particular attention:

1. **An imported artifact's `affects_layout` is the value packaged for layout.** A scene layer has a field with the same name, but `layers::package` copies the artifact value. The scene value is passed to the worker for prompted layers. All fifteen effective scene/artifact pairs currently agree, so there is no observed contradiction; editing only one side of an imported pair could still be misleading.
2. **Imported sequences do not receive a separate render-time `active_frames` cutoff.** The artifact's frame range and actual pixels govern consumption. `active_frames` is used for prompted-worker requests and their validation, but is not retained as a distinct range in the packaged render layer. The current imported masks avoid a visible conflict by being empty outside their declared windows, or by storing only that window. Changing the scene range alone would not trim a nonempty imported mask.

These are conclusions from [scene resolution](src/analyze/mod.rs), [layer packaging and reading](src/layers.rs), and [worker validation](src/segmentation.rs), not assumptions based on field names.

## Shared artwork and global assumptions

### Injected plaque artwork

The [plaque catalog](assets/plaques/catalog.toml) offers two artwork families in two intended video compositions:

| Artwork family | Intended composition and actual bitmap sizes | Meaning of its margins |
|---|---|---|
| Aetherglass Aurora | Landscape: `1500 × 420`; portrait: `1000 × 280` | Keep 8% of each side and 12% of the top and bottom outside the writable rectangle. This leaves 84% of the width and 76% of the height. |
| Prismwraith Reliquary | Landscape: `1500 × 680`; portrait: `1000 × 420` | Keep 10% of each side and 16% of the top and bottom outside the writable rectangle. |

The aspect label describes the intended **video composition**, not the bitmap's own aspect ratio. In particular, the portrait artwork is still a horizontal plaque. That is consistent with placing a short title banner near the top of a tall video.

All four plaque-less scenes select Aetherglass Aurora. None selects Prismwraith Reliquary. The four artwork hashes and declared image dimensions match the checked PNGs.

The scene's image reference and inset drive analysis. The catalog is descriptive metadata used for inventory; it is not a live link that automatically updates a scene's copied inset. A future artwork change should therefore check both locations. See [catalog model](src/media/plaques.rs) and `apply_surface_intent` in [analysis](src/analyze/mod.rs).

When `appearance` is omitted, its default is `observed`: the source supplies the visual background and no new plaque image is injected. This applies both to existing physical plaques and to the two empty background openings used as screen canvases. The four image-injection scenes explicitly select `kind = "image"` instead.

The injected PNG's alpha controls the visibility of the artwork itself. Its translucent center does **not** multiply away the title's writable area or make the title equally translucent. The code deliberately keeps image transparency and typography geometry separate.

### Text-free source assumption

The high-level analysis script explicitly supplies `--source-is-text-free`, and the checked analysis manifests record that assertion. It is not an OCR result or a per-video human annotation in `scene.toml`.

For practical use, this asserts that no pre-existing title needs to be erased before adding the new one. The sampled videos contain decorative displays, symbols, and some background screen text. “Text-free” should not be read as proof that every pixel in the entire source is free of writing. This matters especially for transparent holographic surfaces through which background displays remain visible. The [analysis entry point](src/analyze/mod.rs) requires the assertion because the renderer does not remove or inpaint an old title.

### Segmentation acceptance policy

[The policy file](assets/segmentation/policy.toml) defines `preview`, `balanced`, and `canonical` acceptance thresholds. The current balanced and canonical thresholds are identical; preview is more permissive about positive and negative prompt agreement. These acceptance thresholds do not imply that the profiles use identical model execution strategies.

The values express several understandable checks:

- Positive cues should receive appreciable alpha and negative cues should not. Canonical uses approximately half-scale as the dividing point; the positive-evidence calculation allows a small neighborhood around a supplied point.
- A candidate should not label almost the entire frame, or simply fill an authored foreground box as a rectangle.
- Where the persistent boxed-foreground checks apply, the mask should not collapse between prompt frames or change implausibly between adjacent frames. The area comparison uses interpolation between prompt-frame areas; adjacent-mask IoU measures overlap between successive masks.

These are broad rejection rules, not precise anatomical or material ground truth. The policy allows zero as the generic minimum fraction of nonempty frames. Temporal checks are also conditional: `persistent_prompt_track` applies to eligible foreground prompts with boxes, not every writing-surface or point-only layer. It would be incorrect to infer universal per-frame continuity from this policy alone. See [strategy policy](src/segmentation_strategy.rs) and [evidence evaluation](src/segmentation.rs).

## Video-by-video assessment

Each entry below describes the source composition, the actual supplied decisions, and their implications. Frame times are approximate at 24 fps. A “contract” means an existing accepted behavioral specification; it does not mean that this assessment rerendered and certified the current output.

### 1. `16_9_background_digifall`

[Scene definition](assets/scenes/16_9_background_digifall/scene.toml) · [Source video](assets/16_9_background_digifall.mp4)

The source is a digital waterfall composition with a large dark circular opening toward the left and animated imagery and a magnifying glass toward the right. The definition reserves a large ellipse inside that opening. Its placement is intentionally left of the video's center; it is not an accidentally off-center physical plaque.

The frame-10 rectangle covers most of the picture height, while the ellipse rounds away its corners. The center is around 39% of the image width. That combination permits a large, vertically arranged title inside the opening without filling the busy right side.

This is a `screen-canvas` with `flat` depth. There are no layers, prompts, injected artwork, or authored trajectory. The title stays fixed while the background moves, and the moving magnifying glass is not declared to cover it. An [existing contract](assets/homologation/16_9_background_digifall/contract.toml) checks the ellipse's bounds and the three-line composition “DIGITAL / MATRIX / FALL”; it contains no foreground-preservation witnesses.

### 2. `16_9_dungeon_spider_iron_plaque`

[Scene definition](assets/scenes/16_9_dungeon_spider_iron_plaque/scene.toml) · [Trajectory](assets/scenes/16_9_dungeon_spider_iron_plaque/trajectory.toml) · [Source video](assets/16_9_dungeon_spider_iron_plaque.mp4)

The intended title belongs to the long, shallow iron plaque across the upper dungeon. The frame-10 outer rectangle identifies that object; an inset rounded rectangle protects its metal border. The writable region is wide enough for a long two-line title but has very limited height.

The supplied trajectory locks the plaque on all 234 decoded frames. It moves upward and changes perspective as the shot develops. Calling this case automatically tracked would obscure the fact that the motion solution is already supplied.

The `spider` layer supplies 21 temporal prompts from the first through the last decoded frame. Its boxes and point pattern describe a spider walking from left to right across the plaque. The box size and relative point arrangement are the same in every prompt, translated along the walk. This is regular guidance to a segmentation model, not 21 individually traced anatomical silhouettes.

The spider is explicitly opaque, covers letters, and affects neither layout nor tracking. Depth remains `automatic`, which is especially relevant to the separate crossing web: there is no explicit human `web` layer. The [contract](assets/homologation/16_9_dungeon_spider_iron_plaque/contract.toml) is unusually useful here because it checks both preserved spider/web material and visible title through genuine web gaps. It therefore distinguishes a porous web from a solid sheet that hides everything behind it.

### 3. `16_9_holographic_datacenter_static_plaque`

[Scene definition](assets/scenes/16_9_holographic_datacenter_static_plaque/scene.toml) · [Source video](assets/16_9_holographic_datacenter_static_plaque.mp4)

The selected object is the broad cyan holographic banner above the character. The supplied frame-41 rectangle occupies roughly the middle half of the image width near the top. There is no explicit inner writing shape, so cavity reconstruction determines how far typography stays from the luminous frame.

The surface is `scene-plane`, and its motion is estimated rather than supplied. The word `static` in the filename does not force an identity transform. `declared-only` depth with no layers means the scene supplies no foreground objects that should cover the title and suppresses general automatic foreground discovery.

This is a relatively small human definition: “use this banner and reference view, estimate its motion and writing cavity, and do not infer foreground from the surrounding animated graphics.” There is no dedicated homologation contract in the checked tree. Its visual stability should not be inferred solely from the filename.

### 4. `16_9_mountain_top_day_hummingbird_cloudy_plaque`

[Scene definition](assets/scenes/16_9_mountain_top_day_hummingbird_cloudy_plaque/scene.toml) · [Late-foreground artifact](assets/scenes/16_9_mountain_top_day_hummingbird_cloudy_plaque/late-foreground/artifact.toml) · [Source video](assets/16_9_mountain_top_day_hummingbird_cloudy_plaque.mp4)

The writing surface is the elongated cloud above the mountain character. Its irregular, breathing silhouette is reduced to a stable planar title region: a frame-10 outer rectangle and a rounded inner rectangle with more generous corner rounding than the iron and wooden plaques.

Five `cloud-writing-surface` prompts describe cloud material over the clip, including its late movement toward and beyond the left edge. Later negative points deliberately exclude the approaching hummingbird and nearby non-cloud material. This is positive evidence about the cloud's material, not a request to stretch every letter with the changing cloud outline.

The writing-surface layer has `affects_layout = false`, preserving the authored typography shape. Its source-pixel masks can support tracking subject to the implementation's plausibility checks. General automatic depth discovery is disabled by `declared-only`.

Foreground is supplied separately through 120 masks covering frames 120–239, approximately the second half of the clip. The comments identify the late hummingbird and the ogre's hat as relevant foreground. `late-foreground` is deliberately a compositing category containing multiple objects, rather than one semantic animal identity. This opaque layer may influence tracking but does not reserve empty layout space.

The [contract](assets/homologation/16_9_mountain_top_day_hummingbird_cloudy_plaque/contract.toml) has five foreground witnesses, including between-prompt frames. Earlier foreground events would need separate evidence if they became important: a later sequence and `declared-only` do not automatically protect every bird throughout the entire video.

### 5. `16_9_plaqueless_mountain_top_night`

[Scene definition](assets/scenes/16_9_plaqueless_mountain_top_night/scene.toml) · [Source video](assets/16_9_plaqueless_mountain_top_night.mp4)

This source begins with a character on a night mountain and later moves into a very different astronomical view. There is no existing title plaque selected for tracking. Instead, Aetherglass Aurora is injected as a wide banner in the upper central area, using frame 0 as the placement reference.

The `screen-canvas` and `flat` settings are consequential: the banner remains in the same screen position through the changing shot. It does not recede with the mountains or become part of the star field. The common artwork inset keeps the title away from decorative edges, leaving roughly two thirds of the placed rectangle's area writable before typography padding.

There are no foreground layers or dedicated homologation contract. The static overlay is a declared compositional choice, not evidence that the system has solved physical placement in this changing environment.

### 6. `16_9_plaqueless_swamp`

[Scene definition](assets/scenes/16_9_plaqueless_swamp/scene.toml) · [Source video](assets/16_9_plaqueless_swamp.mp4)

This swamp scene has a character, vegetation, and animated luminous objects, but the chosen title treatment is another injected Aetherglass Aurora banner. It spans about 70% of the frame width above the character, with a placement very similar to the night-mountain banner.

Frame 0, screen-fixed positioning, flat depth, and the standard inset make this a reusable graphic overlay specification. The title and artwork do not follow tree or camera motion, and there are no declarations that vegetation, props, or the character should cover them.

Its similarity to the other landscape injection is an intentional reuse of a visual treatment. The source imagery differs, but the rendering responsibility supplied by the definition is nearly the same. No dedicated homologation contract is present.

### 7. `16_9_scrapyard_iron_plaque_foreground_chains`

[Scene definition](assets/scenes/16_9_scrapyard_iron_plaque_foreground_chains/scene.toml) · [Chains artifact](assets/scenes/16_9_scrapyard_iron_plaque_foreground_chains/chains/artifact.toml) · [Writable mask](assets/scenes/16_9_scrapyard_iron_plaque_foreground_chains/writable-mask.png) · [Source video](assets/16_9_scrapyard_iron_plaque_foreground_chains.mp4)

The selected surface is the rusted iron plaque near the top of the scrapyard. Large chains pass close to the camera and across the title region while the plaque's perspective also changes. Frame 113 supplies the outer tracking rectangle; there is no authored trajectory, so the plaque motion is estimated.

The title cavity is a frozen mask inside the metal border. The mask is mostly rectangular with small corner cuts and softened edge values, not a highly convoluted outline. Its importance is that the exact accepted cavity is preserved, rather than that its shape is visually exotic.

Depth is `declared-only`. The supplied chain sequence is therefore essential to the intended crossing. It stores 240 full-frame masks, but only frames 60–116 contain nonzero pixels. This matches the scene's active interval, approximately 2.5–4.83 seconds. The remaining stored images are intentionally empty, not failed segmentations.

The chain layer uses optical alpha, does not change layout, and cannot influence tracking through this declared layer. Its metadata records a SAM2/Cutie/ViTMatte origin, but the scene imports it as fixed evidence rather than providing prompts to recreate it. The [contract](assets/homologation/16_9_scrapyard_iron_plaque_foreground_chains/contract.toml) pins the writable mask and checks four source-preservation frames. This case is included in the continuous homologation script.

### 8. `16_9_swamp_iron_plaque`

[Scene definition](assets/scenes/16_9_swamp_iron_plaque/scene.toml) · [Moss artifact](assets/scenes/16_9_swamp_iron_plaque/moss-soft.toml) · [Shadow artifact](assets/scenes/16_9_swamp_iron_plaque/moss-shadow.toml) · [Source video](assets/16_9_swamp_iron_plaque.mp4)

The selected iron plaque is narrow and near the top center, with moss hanging over its edges. Frame 83 identifies the tracking rectangle. Motion and the writing cavity are estimated; there is no supplied trajectory or explicit writable-region shape.

Two static canonical masks describe attached moss and its existing shadow. Their `640 × 125` size matches the selected plaque canvas, not the full landscape video. They move with the plaque transform and cannot describe independently moving vegetation elsewhere in the source.

Both layers leave layout unchanged. The moss restores existing foreground detail over letters, while the faint shadow mask restores a smaller amount of source appearance. `declared-only` excludes additional automatic foreground discovery. There is no dedicated homologation contract, so the presence of the masks alone does not establish that every moss edge and shadow has been independently accepted in a final render.

### 9. `16_9_swamp_wooden_plaque`

[Scene definition](assets/scenes/16_9_swamp_wooden_plaque/scene.toml) · [Source video](assets/16_9_swamp_wooden_plaque.mp4)

The title belongs to the wooden sign above the character. Its frame-10 outer rectangle includes decorative material around the sign, while a considerably smaller rounded rectangle reserves the central wood for text. The inner rectangle's bounding area is less than half the outer rectangle's area; that is meaningful protection for the decorated border, not merely a tiny generic margin.

One foreground layer groups moss, two vines, and the moving lizard. The first three identities are seeded at frame 10. The lizard is supplied with four observations, at frames 10, 80, 160, and 230, describing its changing position along the lower part of the plaque.

These objects do not change the title composition but can influence tracking through their source-pixel foreground masks. No opaque matte override is supplied, so the combined mask retains optical alpha. General automatic depth analysis remains enabled alongside that prompted layer.

The [contract](assets/homologation/16_9_swamp_wooden_plaque/contract.toml) accepts a two-line Portuguese title with accented characters in the gold-shine style, and this case is a continuous sentinel. However, its contract has **no source-preservation witnesses**. It directly protects geometry and typography; it should not be described as equally strong independent acceptance evidence for its lizard, vines, and moss.

### 10. `16_9_swamp_wooden_plaque_foreground_vines_and_lizard`

[Scene definition](assets/scenes/16_9_swamp_wooden_plaque_foreground_vines_and_lizard/scene.toml) · [Writing-surface artifact](assets/scenes/16_9_swamp_wooden_plaque_foreground_vines_and_lizard/writing-surface.toml) · [Attached-material artifact](assets/scenes/16_9_swamp_wooden_plaque_foreground_vines_and_lizard/attached.toml) · [Shadow artifact](assets/scenes/16_9_swamp_wooden_plaque_foreground_vines_and_lizard/attached-shadow.toml) · [Source video](assets/16_9_swamp_wooden_plaque_foreground_vines_and_lizard.mp4)

This related woodland composition uses a more elaborate definition than the preceding video. The frame-72 outer rectangle is taller and includes substantial surrounding material. It does not itself identify the entire safe text area.

A static writing-surface mask supplies a broad, nearly rectangular interior with sloped lower corners. It affects layout and intersects the automatically recovered content area. This is the only declared layer in the collection that changes layout; there is no separate `writable_region` entry.

Attached foreground and attached shadow are separate canonical images. They distinguish vegetation fixed to the plaque from a lizard that moves independently. The lizard has four source-pixel box-and-point prompts at frames 0, 72, 144, and 239. Its declared horizontal movement is not monotonic: it moves rightward, partly back left, and then right again. Treating the four prompts as an assumed constant-speed walk would lose supplied information.

All three canonical images are `641 × 285`, matching the enclosing plaque canvas. The foreground and shadow do not reserve empty title space. Automatic depth is enabled. The [capability ledger](assets/homologation/capabilities.toml) explicitly uses this asset to represent multiple foreground/shadow layers, but leaves its homologation contract absent. That is a significant coverage gap for one of the richest definitions in the collection.

### 11. `9_16_background_ogre_dear`

[Scene definition](assets/scenes/9_16_background_ogre_dear/scene.toml) · [Source video](assets/9_16_background_ogre_dear.mp4)

The portrait composition places an antlered character and digital imagery on the right, leaving a dark circular opening toward the upper left. The supplied ellipse puts the title into that opening. Its location left of center is consistent with the imagery; it is not intended to cover the full portrait width.

This is the portrait counterpart of the digital-background treatment in concept, but not a simple scaling of its coordinates. The ellipse occupies only the upper portion of a much taller image. Frame 10 is a placement reference, with screen-fixed motion and flat depth.

There are no semantic prompts for the character, no foreground masks, and no injected plaque. The filename's spelling is simply the asset identifier. The clip has 192 decoded frames, or eight seconds, and no dedicated homologation contract. Animated material entering the ellipse would not automatically move in front of the title under this definition.

### 12. `9_16_dungeon_spider_iron_plaque`

[Scene definition](assets/scenes/9_16_dungeon_spider_iron_plaque/scene.toml) · [Trajectory](assets/scenes/9_16_dungeon_spider_iron_plaque/trajectory.toml) · [Source video](assets/9_16_dungeon_spider_iron_plaque.mp4)

The portrait iron plaque is a deeper rectangular sign above the character, with a notch along its upper edge. The definition uses a frame-10 outer rectangle and a smaller rounded writing area. A dense trajectory fixes all 206 decoded poses, including the plaque's rightward displacement and later return.

There are two different kinds of supplied segmentation guidance. The opaque `spider` foreground has ten detailed box-and-point prompts, mostly describing the later walk. Its active window is frames 104–205, approximately 4.33 seconds through the end. It covers letters but affects neither layout nor tracking.

The `iron-writing-surface` layer identifies the iron plate itself. Its first two prompts are twelve-vertex polygons at frames 10 and 80, preserving the notched shape and the plate's displaced position. Four later prompts use positive iron points and negative evidence on the web, spider, and surrounding material. These are material-support instructions, not an independent deformable outline for typography. The layer leaves layout unchanged; the dense trajectory still supplies the motion solution.

The spider's confidence-to-opacity thresholds are considerably stricter than those on the landscape spider. Meanwhile, automatic depth remains responsible for the separate translucent or porous web, for which no explicit scene layer is supplied.

The [contract](assets/homologation/9_16_dungeon_spider_iron_plaque/contract.toml) pins the supplied trajectory, checks spider and web preservation, and requires title visibility through web gaps. It is also a continuous sentinel. This provides stronger independent foreground evidence than merely checking whether a segmentation sequence exists or is nonempty.

### 13. `9_16_dungeon_spider_iron_temporary_plaque`

[Scene definition](assets/scenes/9_16_dungeon_spider_iron_temporary_plaque/scene.toml) · [Trajectory](assets/scenes/9_16_dungeon_spider_iron_temporary_plaque/trajectory.toml) · [Housing artifact](assets/scenes/9_16_dungeon_spider_iron_temporary_plaque/instrument-housing/artifact.toml) · [Source video](assets/9_16_dungeon_spider_iron_temporary_plaque.mp4)

The source has an initial iron writing plate that retracts behind a mechanical instrument housing. The title belongs to the early plate, not to the instrument display that remains afterward. That distinction is explicitly authored rather than left to object discovery.

The supplied 240-frame trajectory keeps the plate at a constant size while translating it upward. Its rectangular dimensions remain fixed; the definition deliberately avoids interpreting retraction as the plaque shrinking or shearing. The rounded writing area is inset from this constant-sized plane.

Visibility is fully on through frame 60, decreases over frames 61–67, and is zero from frame 68 through the end. Thus the added title disappears by approximately **2.83 seconds**, even though the source video lasts ten seconds and the stored quad remains meaningful. Most of the trajectory describes the location of a now-hidden title surface.

The `instrument-housing` layer supplies opaque foreground that can hide the retracting title. Its scene window is 32–71; the artifact stores all 240 frames, with nonzero masks only on frames 36–71. The initial empty part of the declared window is internally consistent: an active interval permits an object to contribute but does not require it to overlap immediately.

This housing is explicitly excluded from tracking and layout. Automatic depth is still enabled for other material; despite the filename, there is no human-prompted spider layer in this scene. The [contract](assets/homologation/9_16_dungeon_spider_iron_temporary_plaque/contract.toml) pins the trajectory and includes two housing-preservation witnesses. It is a continuous sentinel.

The short title lifetime is a practical constraint on styles: a long introductory text animation could consume most of the readable interval. A style change cannot override the authored disappearance simply by continuing its own animation.

### 14. `9_16_lonely_ogre_holographic_static_plaque`

[Scene definition](assets/scenes/9_16_lonely_ogre_holographic_static_plaque/scene.toml) · [Source video](assets/9_16_lonely_ogre_holographic_static_plaque.mp4)

The chosen object is a large holographic panel in the upper part of the portrait image, with the character and animated digital material below it. Its rectangle occupies about three quarters of the source width, giving this scene a substantially deeper title area than the narrow injected portrait banners.

Frame 207, late in the clip, is the supplied reference view. This does not postpone the title until that frame. It tells analysis which view the rectangle describes. The source samples show the panel earlier as well.

There is no explicit writable shape, no trajectory, and no layer. Motion and cavity reconstruction remain automatic, while `declared-only` suppresses general depth inference from animated graphics. The capability ledger names this asset for background movement that must not pull the plaque off position, but has no accepted contract for it. That specifically leaves the background-versus-plaque distinction without a corresponding final-render contract in the ledger.

### 15. `9_16_plaqueless_datacenter_lab`

[Scene definition](assets/scenes/9_16_plaqueless_datacenter_lab/scene.toml) · [Source video](assets/9_16_plaqueless_datacenter_lab.mp4)

The scene adds the portrait Aetherglass Aurora artwork near the top of the lab image. Its placement spans about 81% of the video width but only 13% of its height. It is a horizontal heading above the character, rather than a portrait-shaped replacement panel.

The standard inset reduces the writing area to a relatively shallow central rectangle. Frame 0, screen-fixed positioning, and flat depth make the banner independent of animated monitors, particles, and character motion. There are no foreground layers and no dedicated homologation contract.

The source decodes to 236 frames, although the container advertises 240. The generated analysis uses 236, consistent with the actual decoded clip. This is a source-metadata issue, not four missing human annotations.

### 16. `9_16_plaqueless_neon_datacenter_ground_hole`

[Scene definition](assets/scenes/9_16_plaqueless_neon_datacenter_ground_hole/scene.toml) · [Source video](assets/9_16_plaqueless_neon_datacenter_ground_hole.mp4)

This source shows the character emerging into a neon datacenter. Its title treatment is almost the same as the portrait lab's: the same Aetherglass artwork, width, height, side placement, and inset, placed slightly higher in the image.

The difference in vertical position is a composition choice, not a different tracking model. It remains a flat screen canvas throughout the character's movement. There is no declaration that the rising character or other close objects should cover the banner, and the name `ground_hole` does not add any depth behavior.

This clip has the full 240 decoded frames. No dedicated homologation contract is present. Its definition is useful as a second placement example, but does not independently demonstrate scene-attached injection or foreground interaction with injected artwork.

### 17. `9_16_scrappy_datacenter_holographic_plaque`

[Scene definition](assets/scenes/9_16_scrappy_datacenter_holographic_plaque/scene.toml) · [Source video](assets/9_16_scrappy_datacenter_holographic_plaque.mp4)

The intended writing object is an existing holographic sign above the character in a cluttered industrial datacenter. Frame 0 identifies a rectangle covering about 60% of the video width. The sampled source shows its apparent scale and placement changing during the clip.

The scene therefore requests a physical plane with automatically estimated motion, rather than a fixed overlay. There is no explicit writable-region shape; the implementation has to infer the interior despite the luminous border and background material visible through the panel.

`declared-only` with no layers means no explicit scene foreground covers the title. There are also no prompts or supplied trajectory to disambiguate tracking. This is a comparatively light definition for a visually complicated surface, and a useful candidate for focused human review of interior fitting and motion. No dedicated homologation contract is present.

### 18. `9_16_swamp_wooden_plaque`

[Scene definition](assets/scenes/9_16_swamp_wooden_plaque/scene.toml) · [Source video](assets/9_16_swamp_wooden_plaque.mp4)

The portrait wooden sign stretches across the entire source width. The outer rectangle touching both image edges is intentional and valid; it is not itself evidence that the coordinates escaped the frame. The rounded writing region retreats from those edges and the plaque's upper and lower decoration.

The foreground structure resembles the simpler landscape wooden scene: one layer groups moss, left vine, right vine, and lizard. Vegetation is seeded at frame 10; four lizard observations describe its later movement across the lower writing area. The actual pixel locations and the proportions of the writing cavity are separately supplied for this portrait source.

The combined layer has optical alpha, affects tracking by default, and does not change layout. Automatic depth remains enabled. The common motif does not justify scaling or copying the landscape prompts: these are different source videos with different geometry and staging. Unlike the landscape wooden plaque, this portrait scene has no dedicated homologation contract.

### 19. `moving-holographic-plaque`

[Scene definition](assets/scenes/moving-holographic-plaque/scene.toml) · [Trajectory](assets/scenes/moving-holographic-plaque/trajectory.toml) · [Writable mask](assets/scenes/moving-holographic-plaque/writable-mask.png) · [Source video](assets/moving-holographic-plaque.mp4)

Despite lacking an aspect-ratio prefix, this is a portrait video. The selected holographic panel grows and moves upward, becomes partly clipped by the top edge, and returns toward its earlier placement. Background monitor detail remains visible through the transparent panel.

Frame 72 defines the outer placement. A frozen writable mask supplies a mostly rectangular cavity with clipped corners and a soft edge. It is distinct from the luminous frame and from the larger tracking rectangle.

All 240 poses are supplied as locked quads. Some corner coordinates become negative when the plaque moves above the viewport; that is a legitimate representation of partial offscreen motion. There are no foreground layers, and depth is `declared-only`.

Visibility is one on every frame except **124 and 125**, where it drops to approximately **8.84%**. Inspection of source frames 122–127 shows a real hologram flicker: the border becomes very faint or absent around this interval. The low values therefore have a plausible source-based purpose. The precise two-frame timing and abrupt return remain a review point, particularly since neighboring frames also show changing border intensity. They should not be “fixed” to one merely because the numbers look unusual.

The [contract](assets/homologation/moving-holographic-plaque/contract.toml) pins both the writable-mask bytes and the trajectory bytes and checks a two-line classic-glow title. It is a continuous sentinel, but has no dedicated visual witness of the flicker itself. The trajectory hash freezes that choice; it does not explain or independently demonstrate the correctness of the opacity timing.

## What the homologation data actually certifies

Homologation is a specification for a particular accepted example render. It is not a substitute for the scene instructions and does not supply the default title for every subsequent use of the video.

In each [contract](assets/homologation/), the source hash identifies the source bytes; geometry identifies the accepted tracking and writable bounds; the render section identifies a text, style, font filename, and fitting mode; and typography specifies the expected line composition and numerical ceilings. All eight contracts use `NotoSerif-Regular.ttf` and artistic fitting. The continuous script supplies the repository-pinned font file explicitly.

`render.text` is the input title. `render.resolved_text` records the accepted line arrangement after fitting. These can differ: the portrait dungeon takes “Seeing what others cannot see!” and expects three lines with “others” on its own line. That is a specific artistic acceptance decision, not a universal rule about optimal wrapping.

`maximum_font_size` is a ceiling, not an exact required size or a minimum readability guarantee. Zero permitted clipped, missing, or fallback glyphs expresses a stricter requirement: the accepted title must fit and use the expected glyph coverage without such problems.

| Contract | Accepted title treatment | Independent selected-pixel evidence | In the continuous render script? |
|---|---|---|---|
| Landscape digital background | “DIGITAL MATRIX FALL,” three lines, gold-shine | None | No |
| Landscape dungeon spider | “WITH THE BIGGER POTENTIAL OF SEEING FURTHER,” two lines, gold-shine | 25 spider and three web-preservation witnesses; three web-gap title witnesses | No |
| Cloudy mountain plaque | “Codex gave me the bigger / potential of seeing / what others cannot see!”, three lines, classic-glow | Five foreground-preservation witnesses | No |
| Scrapyard chains | “SEEING / FURTHER,” two lines, classic-glow | Four chain-preservation witnesses | Yes |
| Landscape wooden swamp plaque | “Nós que aqui estamos, por vós esperamos!”, two lines, gold-shine | None | Yes |
| Portrait dungeon spider | “Seeing what others cannot see!”, three lines, gold-shine | Thirteen spider and three web-preservation witnesses; three web-gap title witnesses | Yes |
| Temporary portrait plaque | “Seeing what / others cannot / see!”, three lines, classic-glow | Two housing-preservation witnesses | Yes |
| Moving holographic plaque | “SEEING FURTHER / THAN BEFORE,” two lines, classic-glow | None | Yes |

Slashes in this table represent line breaks. The contract files retain the exact text and case.

A **source-preservation witness** selects pixels where the rendered result must remain close to the source, allowing bounded encoding or compositing differences. It asks whether foreground material remains visibly in front of the title. A minimum selected-pixel count prevents an accidentally empty or trivial mask from being accepted.

A **title-visibility witness** asks the complementary question: selected pixels must visibly differ from the source because the title should show there. This is essential for web holes. Preserving all source pixels everywhere would satisfy a preservation-only check while displaying no title at all; the complementary witnesses reject that failure at their selected locations.

These are sparse visual requirements, not full object segmentation labels. Their error tolerances describe acceptable color differences, not percentages of segmentation accuracy. The checked 61 mask hashes match, and their nonzero selections meet the declared minima. That verifies the stored witness inputs; it does not evaluate a new render against them. See [homologation implementation](src/homologation.rs).

The [main capability ledger](assets/homologation/capabilities.toml) lists ten behavioral capabilities, eight with contracts and five marked for continuous rendering. The two explicitly uncontracted capabilities are background motion independent of the plaque and multiple foreground/shadow layers. Eleven videos have no individual contract, but that is not equivalent to eleven wholly untested capabilities: the project deliberately uses representative behavior coverage, and other automated tests also exist.

The separate [segmentation ledger](assets/homologation/segmentation-capabilities.toml) has six categories, four with representative assets, but only one explicit `final_homologation` link. Human fine-detail matting and generic open-vocabulary segmentation have no representative asset there. The segmentation parallax entry does not link the existing chains contract. This is a documentation/coverage-ledger discrepancy worth reconciling; an available render contract should not automatically be assumed to accept every segmentation-specific capability.

## Verification performed and limits

The assessment parsed the scene definitions and referenced trajectory/artifact metadata, sequentially decoded all 19 source videos with OpenCV to establish actual frame counts, inspected representative source frames for every video, and made targeted inspections around the spider prompts, temporary retraction, and holographic flicker. Static writable and material masks were also inspected visually. Every supplied layer-mask image was examined for dimensions and nonempty coverage, including images outside declared active windows.

Additional checks compared all four trajectory source hashes, all 19 cached source hashes and frame counts, the cached raw scene/trajectory identities, all eight contract source/style identities and geometry bounds, all five optional contract geometry hashes, all 61 witness hashes and minimum selections, and all four catalog artwork hashes and dimensions. No inconsistency was found in those checks. The current prompted masks also agreed with the checked positive/negative point samples at their authored prompt frames; this is a consistency observation, not independent segmentation truth.

The primary code references are [scene schemas and validation](src/scene.rs), [analysis selection and precedence](src/analyze/mod.rs), [candidate selection](src/analyze/candidate.rs), [layer interpretation](src/layers.rs), [writable geometry](src/writable_region.rs), [tracking and dense inputs](src/analyze/tracking/mod.rs), [visibility constraints](src/analyze/tracking/constraints.rs), [segmentation generation and acceptance](src/segmentation.rs), [the worker](tools/segmentation_worker.py), [rendering](src/render/mod.rs), [video probing](src/video.rs), and [homologation](src/homologation.rs).

This was an input-definition assessment with sampled visual inspection, not a frame-by-frame human acceptance of every source or rendered output. It did not regenerate ML analysis, run a complete cache-freshness audit against current worker/runtime/build identities, rerender the videos, or execute the full homologation gate. Existing generated confidence values and reports were not treated as proof of present visual correctness.

## Final considerations

### Common patterns and strengths

**The collection consistently distinguishes surface extent from writing space.** Outer bounds often include framing, decoration, or tracking material; smaller writable regions prevent text from filling those features. The background ellipses are a different composition choice, making broad empty visual areas into title silhouettes.

**The dominant composition is a title above a character.** Most supplied placements sit in the upper portion of the image. The two background canvases use left-biased empty openings, and the four injected canvases use broad top banners. This is a coherent art direction, but provides limited evidence about lower-third titles, arbitrary side walls, or many simultaneous surfaces.

**Layout generally stays stable while foreground passes over it.** Almost every layer opts out of layout changes. That avoids a title permanently dodging the entire path of a moving lizard or chain. Foreground preservation, rather than continuous reflow, is the intended effect.

**Attached and independent motion are distinguished where necessary.** Canonical moss and shadow images follow the plaque; source-pixel lizard and chain sequences can move separately. The more elaborate woodland scene is a particularly clear example of this distinction.

**The web contracts test an important two-sided requirement.** Preserving web threads and showing title through gaps is much stronger evidence than a nonempty-mask check. More complex foreground acceptance could benefit from similarly complementary visible outcomes.

**The checked identities and dimensions are coherent.** No supplied source reference, reviewed trajectory source hash, checked homologation hash, or catalog artwork identity was inconsistent in this snapshot. Reference frames and prompt positions were within their sources, and imported layer images had the dimensions required by their coordinate systems.

### Confirmed implementation limitations relevant to editing the data

**Imported `active_frames` is not an independent runtime limit.** This is the clearest difference between an intuitive reading of the scene file and the current consumer. The chains and housing artifacts span the entire clip. Their nonzero pixels currently respect the narrower scene windows, so no out-of-window contribution was found. A future editor shortening the window must also change the artifact's effective data or ensure the consumer enforces the intended range. The cloud sequence already stores only its declared late interval.

**Layout authority is duplicated between a scene layer and its artifact.** Current values agree, but imported artifacts supply the actual packaged flag. A scene-only edit can appear meaningful while leaving layout behavior unchanged. This deserves explicit documentation or a future behavior change backed by a focused regression test; it is not evidence that the present layouts are wrong.

**Some expressive field names have narrower effects than they suggest.** `in_front_of` is not a general depth graph, canonical `affects_tracking` does not create independent source tracking evidence, and a writing-surface mask does not automatically deform typography. Future definitions should be checked against these consumers instead of relying on names alone.

### Suspicious or review-worthy choices, with the evidence qualified

**The moving hologram's visibility is unusually abrupt.** The source confirms a real flicker, so a low-opacity interval is plausible. The exact 8.84% value on only two frames has no accompanying explanation of how it was chosen. Review the neighboring frames and a representative rendered title before deciding whether its timing is satisfactory. The trajectory is hash-pinned by homologation, so changing this intentionally also changes accepted behavior.

**Dense “reviewed” trajectories encode substantial prior work.** Four difficult motion cases already contain complete locked solutions. The comments assert review, but do not identify an acceptance session or distinguish measured, interpolated, exported, and manually corrected coordinates. Fine decimal precision should not be confused with comparable human measurement accuracy. These files are legitimate supplied inputs; they are weak evidence that the current tracker can independently solve the same cases from pixels.

**Some authoritative scene masks retain incomplete historical provenance.** The chains and attached-material metadata name earlier ML pipelines, while moss and shadow metadata name refinements. They lack the complete source/prompt/worker/runtime identities required of live generated caches. The code intentionally permits this for imported artifacts, so they are not invalid on that basis. However, their exact reconstruction and review history cannot be recovered from those short generator records alone.

**The landscape spider cues are highly templated.** All 21 prompts use the same box dimensions and relative positive/negative arrangement. Source samples support the intended walking path, and the checked generated masks agree with the supplied positive and negative sample locations. That is encouraging, but it does not establish that every point always selects the intended anatomy or that all between-prompt silhouettes are correct. The independent between-prompt render witnesses are more persuasive evidence for the final visible result.

**Acceptance coverage is uneven where scene definitions are richest.** The multi-layer woodland scene and the background-motion sentinel are explicitly awaiting contracts. The landscape wooden scene has a contract but no selected foreground pixels. Conversely, the cloud and landscape-spider contracts contain useful visual witnesses but are not in the five-case continuous render script. These distinctions matter when describing what is protected against regression; contract existence, CI selection, and visual-depth coverage are separate facts.

**A material change in source geometry can invalidate seemingly portable files.** Paths are portable, but coordinates are tied to the video's pixels and frames. Trajectories and contracts bind particular source bytes; not every static scene artifact independently records its source hash. Replacing a video at the same path should trigger a review of its full definition, even if the filename and image dimensions remain unchanged.

### Unusual details that are explained by the evidence

The following should not be reported as defects merely because they look odd in a numerical inventory:

- **Shorter dungeon trajectories match real decoded clips.** The landscape dungeon decodes to 234 frames and the portrait dungeon to 206, despite both advertising 240 in container metadata. The portrait lab similarly decodes to 236. The [video probing code](src/video.rs) explicitly handles overstated frame counts, and the checked analysis lengths agree with decoding.
- **Late reference frames are not delayed title starts.** Frame 207 for the lonely holographic plaque and frame 113 for the chains are selected geometry references. They do not themselves limit rendering to the end of a clip.
- **Empty sequence images often express absence.** Chains are nonempty only on frames 60–116; housing only on 36–71. Their blank tails should be preserved when interpreting the imported full-length sequences.
- **The temporary plaque intentionally hides most of its title timeline.** Zero visibility after frame 67 describes the source plate's disappearance behind the housing. It is not missing motion data.
- **Negative coordinates on the moving plaque describe offscreen corners.** They do not necessarily indicate a broken transform.
- **A portrait artwork asset can be wide and shallow.** The catalog's aspect label is about composition in a portrait video, not a requirement that the PNG itself be portrait-shaped.
- **The frozen “irregular” writable masks are mostly rectangular.** Their small corner cuts and edge treatment still preserve specific accepted geometry. Their use does not imply a dramatically nonrectangular cavity.
- **`declared-only` with no layers can be deliberate.** It is a choice to keep animated background or holographic detail from becoming foreground occlusion. It would need changing only if the intended visible ordering requires actual foreground crossings.

### Suggested follow-up priorities

1. Make the effective authority of imported mask timing and layout flags clear to anyone editing scene files. The current data is consistent, but these are easy places for a future edit to appear effective when it is not.
2. Preserve review provenance when promoting generated masks or trajectories into authoritative scene inputs: source identity, the reason for the correction, and the accepted visual evidence are more useful than additional decimal precision.
3. Review the moving hologram's short visibility transition in context, and retain or change it deliberately with its accepted evidence.
4. Address the explicitly uncontracted multi-layer and background-motion capabilities, and reconcile the segmentation ledger with existing relevant contracts.
5. When describing automation quality, separate surface selection, motion estimation, depth estimation, and final rendering. This collection supplies different amounts of each answer in advance.

These are assessment recommendations. The source videos, scene instructions, masks, trajectories, contracts, and implementation were not changed for this report.
