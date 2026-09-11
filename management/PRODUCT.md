# Product requirements

**Support new videos through general capabilities. Preserve accepted visual quality while reducing correction effort.** The asset list is evidence, not the product boundary.

## P-001 Generalization

Support unfamiliar videos containing the capabilities below, including their combinations. A repair addresses the failure mechanism beyond the triggering source. Do not gate production behavior on supplied asset identities.

Acceptance: independent examples and valid controlled variations, with existing accepted behavior preserved.

## P-002 Human assistance

Analyze every new video automatically first. Let the human trial-render and review it, then refine the scene when needed. Either an adequate automatic result or an assisted result can be homologated.

State what the software inferred and what was supplied. Evaluate current analysis with the existing approved human inputs before proposing remapped inputs. Preserve quality with comparable or lower correction effort.

Acceptance: the replay, reconstruction, and remapping protocol in [Acceptance](ACCEPTANCE.md). Complete supplied trajectories demonstrate assisted tracking, not automatic discovery.

## P-003 Scene meaning

Define coordinate systems, timing, references, visibility, depth, and layout effects. Valid declarations have their specified effect; otherwise fail with a useful error.

Conflicting effective declarations between a scene and imported artifact fail. Do not silently prefer either source. Distinguish an artifact's storage extent from its active content: zero-padded frames outside a declared interval are not inherently a timing conflict.

Acceptance: observable tests for imported/generated layers, frame boundaries, nonzero content outside active windows, conflicting layout flags, and canonical/source-pixel geometry.

## P-004 Surface and writing region

Distinguish the tracked surface from the region available for typography. Support rectangular, curved, polygonal, masked, injected, and deforming surfaces, including multiple independently moving title surfaces.

Represent each surface's identity, writable region, deformation, and relationship to other surfaces/objects. A writable mask alone does not establish deforming text geometry.

Acceptance: approved content remains within its intended region, follows the appropriate geometry, and preserves depth during surface interactions.

## P-005 Motion and visibility

Keep titles attached through camera/surface motion, deformation, occlusion, offscreen intervals, and shot changes. Independent foreground/background motion must not displace them. Respect approved appearance and disappearance.

Do not pre-forbid a degree of deformation or number of cuts. Establish accuracy for the input; reject insufficient evidence under P-011.

Acceptance: temporal evidence throughout the video, including between anchors, occlusion entry/exit, shot boundaries, and recovery. Supplied motion remains labeled assistance.

## P-006 Foreground and material

Preserve visible ordering among titles, surfaces, foreground objects, attached material, and shadows. Preserve porous gaps, fine detail, and soft/translucent boundaries. Foreground motion changes layout only when explicitly requested.

Acceptance: complementary foreground-preservation and title-visibility evidence, layer interactions, and temporal continuity. A nonempty mask is insufficient.

## P-007 Typography and effects

Render requested text, font, layout, style, and animation with readable glyphs, correct fitting, and intended surface interaction. Title/style changes preserve scene motion and depth.

Acceptance: varied title lengths, explicit line breaks, representative glyphs, styles, and aspect ratios. Missing glyphs, unintended clipping, or tracking shake that makes approved text hard to read fail. [Current style implementation](../docs/TEXT_EFFECTS.md).

## P-008 Reusable analysis

Analyze a source once and reuse compatible results across titles, fonts, and styles. Rendering does not rewrite reusable scene analysis or human intent.

Acceptance: render-only input changes preserve analysis identity. Frozen homologation replay works without rerunning analysis under [A-002](ACCEPTANCE.md#a-002-frozen-replay).

## P-009 Media fidelity and scope

Preserve source content outside the requested effect, including timing, orientation, audio, and color meaning. Support SDR, HDR, wide-gamut, varying frame timing, multiple surfaces, and shot changes.

Impose no arbitrary duration, resolution, frame-rate, or scene-count ceiling. Finite resource failures and unsupported current behavior are explicit failures, not successful approximations.

Acceptance: qualified media profiles, timeline/audio/color evidence, and honest resource diagnostics. The current text-free, 8-bit SDR implementation is a gap, not a permanent scope restriction.

## P-010 Accepted quality

Preserve human-approved visual results across fixes, features, refactors, models, and performance work. Prefer known visual perfection; retain usable imperfect homologations with their documented limitations.

Later renderings must be visually faithful to approved data: no drift. A gain elsewhere cannot compensate for a failed accepted constraint. Improvements to a frozen result require a human-approved successor.

Acceptance: [A-004](ACCEPTANCE.md#a-004-quality-and-fidelity), with separate fidelity and intrinsic-quality evidence.

## P-011 Honest confidence and failure

Distinguish preview, automatic checks, human acceptance, unsupported capability, resource failure, insufficient analysis evidence, and visual regression. A validator's pass is evidence only for what it measures.

Reject when accurate processing cannot be established, including unstable tracking that destroys readability. Do not silently degrade quality or claim human acceptance from a score.

Acceptance: actionable stage-specific failures and measured rejection behavior. Rejecting a previously accepted input is a regression; reject-all cannot masquerade as improved accuracy.

## P-012 Usable delivery

Provide programmatic and CLI operations, documented setup, repeatable analysis/render/review, and portable/bundled workflows. User-visible behavior remains consistent across entry points.

Acceptance: clean-location workflows with declared dependencies on the environments required by [O-006](OPERATIONS.md#o-006-reproducible-execution).

## P-013 Existing-title removal

Support replacing existing titles as well as adding titles to text-free sources. Remove/inpaint the existing title while preserving the intended underlying scene, motion, material, and foreground relationships.

Acceptance: independent examples with temporal evidence for residual lettering, damaged background, flicker, and new-title composition. Unsupported or uncertain reconstruction fails explicitly.
