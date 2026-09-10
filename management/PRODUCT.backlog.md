# Planned



# Started



# "In Code Review"


# "Integrated"


# QA


# Merged


# Rolled Out

## PR.VisFid.01-001 -- Refine specular highlight dynamics on moving metallic plaques
1. Evaluate anisotropic highlight sweep behavior on `16_9_swamp_wooden_plaque` and `moving-holographic-plaque`.
2. Ensure highlight reflection angles correlate realistically with estimated camera motion.
3. Verify title contrast remains within contract specifications during peak specular flash.
   ==> Team. Planned: 2026-08-08; Started: 2026-09-09; Merged: 2026-09-10; Rolled Out: 2026-09-10;

## PN.AstNrg.02-001 -- Ensure complete asset isolation during scene-specific parameter tuning
1. Establish scene-isolated parameter profiles so tuning prompts or detection thresholds for a new video does not alter default behaviors for existing videos.
2. Require that modifications to global algorithms or default model checkpoints pass the full multi-asset visual regression suite before adoption.
   ==> Team. Planned: 2026-09-09; Started: 2026-09-10; Merged: 2026-09-10; Rolled Out: 2026-09-10;

## PN.AstNrg.01-001 -- Establish frozen visual baseline contracts for all 19 video assets
1. Audit the 11 video assets currently lacking `contract.toml` in `assets/homologation/`.
2. Generate candidate renders and conduct human visual reviews for each uncontracted asset.
3. Author formal `contract.toml` specifications capturing source preservation bounds, title visibility thresholds, and keyframe bounds.
4. Add all accepted contracts into the project regression suite so future model updates cannot regress them.
   ==> Team. Planned: 2026-09-09; Started: 2026-09-09; Merged: 2026-09-10; Rolled Out: 2026-09-10;

## PN.OccPrs.02-001 -- Eliminate edge chatter and boundary flicker on thin foreground occluders
1. Address user-reported edge noise on delicate occluders (e.g. spider legs, swinging chain links, vine tendrils).
2. Incorporate temporal alpha boundary smoothing that suppresses single-frame pixel chatter while maintaining sharp silhouette definition.
3. Verify that the fix improves the target scene without altering or regressing any previously approved occluder masks.
   ==> Team. Planned: 2026-09-09; Started: 2026-09-09; Merged: 2026-09-10; Rolled Out: 2026-09-10;

## PN.AugMsn.01-001 -- Core video plaque insertion pipeline
1. Provide command-line workflows for scene creation, surface placement, trajectory export, rendering, and verification.
2. Integrate OpenCV-based video decoding/encoding and tracking into a unified executable.
   ==> Team. Planned: 2026-05-01; Started: 2026-05-05; Merged: 2026-05-20; Rolled Out: 2026-05-20;

## PN.VisFid.01-001 -- Material style shaders: gold-shine and classic-glow
1. Implement physical `gold-shine` metallic plaque shader with moving specular sheen.
2. Implement `classic-glow` luminous holographic shader with alpha projection.
   ==> Team. Planned: 2026-05-10; Started: 2026-05-15; Merged: 2026-05-25; Rolled Out: 2026-05-25;

## PN.OccPrs.01-001 -- Foreground occlusion compositing for dungeon spider and swamp lizard
1. Support multi-layer depth ordering where foreground creatures pass in front of inserted titles.
2. Integrate mask analysis into linear-light compositing pipeline.
   ==> Team. Planned: 2026-06-01; Started: 2026-06-05; Merged: 2026-06-18; Rolled Out: 2026-06-18;

## PN.TypoLay.01-001 -- Multi-line typography fitting with Noto Serif
1. Implement automatic word wrapping, margin calculation, and font size fitting.
2. Support curated `NotoSerif-Regular.ttf` with UTF-8 unicode script handling.
   ==> Team. Planned: 2026-06-01; Started: 2026-06-08; Merged: 2026-06-15; Rolled Out: 2026-06-15;

## PN.VidFmt.01-001 -- Aspect ratio independence for 16:9 landscape and 9:16 portrait
1. Remove hardcoded 16:9 resolution assumptions across rendering and tracking pipelines.
2. Provide verified sample scenes in both landscape and vertical formats.
   ==> Team. Planned: 2026-06-10; Started: 2026-06-15; Merged: 2026-06-28; Rolled Out: 2026-06-28;

## PN.AstNrg.03-001 -- Dual-witness automated acceptance verification
1. Implement automated checks for source preservation (`source_preservation`) outside the plaque boundary.
2. Implement automated title visibility checks (`title_visibility`) verifying legible text pixels.
   ==> Team. Planned: 2026-07-01; Started: 2026-07-05; Merged: 2026-07-15; Rolled Out: 2026-07-15;
