# Changelog

## Unreleased

- Added detector-populated TOML sidecars with commented alternatives, portable prompts, and layer declarations.
- Added mixed guided and locked human motion constraints, dense authoritative track import, and guided-by-default title-pack track export.
- Added human-input provenance and semantic cache invalidation in title-pack format 4.
- Included explicit plaque bounds and legacy CSV contents in cache identity.
- Preserved authored visibility after occlusion analysis and honored loop-closure overrides for human tracks.
- Made track export inherit the analyzed plaque id by default.

## 0.3.0

- Added a documented project objective, first validated source class, and quality decision contract.
- Measured every source frame and replaced sparse automatic trajectory reconstruction with all-frame zero-phase smoothing.
- Removed full-frame ECC retries from the SIFT tracker, reducing the moving reference from minutes to seconds.
- Ranked dominant nested plaque geometry above internal panels and rejected structureless false targets before rendering.
- Made verification remedies report current controls, exact worst-frame diagnostics, and explicit adjustment directions.
- Bounded dense structural registration without changing the static reference's perfect verification score.
- Tracked plaque-border features instead of using background motion as a proxy for plaque motion.
- Made automatic moving-plaque detection pass the production verifier without `--plaque-hint`.
- Added plaque-outline corroboration to constrain feature drift through parallax and foreground crossings.
- Added deterministic build fingerprints to binaries, title-packs, and render metadata.
- Invalidated stale or wrong-source analysis automatically in `replace`, and rejected mismatched analyzer builds in direct rendering.
- Added supervised sparse quadrilateral tracks through `--track-csv`.
- Added an explicit analysis confidence gate with a diagnostic override.
- Separated per-frame plaque visibility from spatial foreground occlusion.
- Made frame counts packet-based and preserved negative audio priming timestamps.
- Made text, stroke, glow colors, and glow radius art-directable.
- Made maximum fitting search the final irregular-mask composition.
- Added render contact sheets and a complete reference-validation script.
- Kept supervised tracks authoritative and added post-refinement/circular motion regularization.
- Prevented structural plaque edges from being classified as foreground occluders.
- Fixed alpha preservation while rectifying masks.
- Reworked tracking, temporal, and loop verification around geometric registration and trajectory continuity instead of animated RGB equality.
- Retained diagnostic `motion.json` when an analysis is rejected by the quality gate.
- Rejected variable-frame-rate sources explicitly and preferred packet counts for exact frame accounting.
- Made plaque visibility depend on structural edge presence rather than animated plaque color.
- Made reviewed supervised geometry an explicit verifier trust boundary while retaining trajectory and structural diagnostics.
- Allowed the reference-validation script to consume `REFERENCE_TRACK` for the supervised production gate.

- Adopted a strict text-free source-plaque contract.
- Removed blank-plaque synthesis and whole-cavity repainting.
- Rendering now composites only the new text alpha layer.
- Added root-anchored plus adaptive-keyframe SIFT tracking.
- Added four-corner zero-phase temporal inertia.
- Added subpixel local plaque translation and scale refinement.
- Added maximum-safe, balanced and fixed typography modes.
- Added explicit failure for empty, overflowing or unsupported text.
- Added optional alignment, line height, padding, stroke and supersampling controls.
- Added progress/ETA output for analysis, rendering and verification.
- Added transactional title-pack creation and source fingerprint validation.
- Added aggressive verification subscores with concrete remedies and worst-frame reporting.
- Added the worst trajectory frame and explicit temporal/loop score bases to verification reports.
