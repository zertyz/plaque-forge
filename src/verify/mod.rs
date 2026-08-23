//! Render verification against the source video and analysis cache.
//!
//! Verification measures scene preservation, tracking, typography validity, temporal
//! stability, occlusion restoration, and loop continuity.

use std::{collections::VecDeque, fs, path::Path};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
    analysis::{
        Analysis, OCCLUDER_DIR, REGISTRATION_MASK_FILE, REGISTRATION_TEMPLATE_FILE,
        STRUCTURAL_MASK_FILE, STRUCTURAL_TEMPLATE_FILE,
    },
    analyze::{
        extraction::{StructuralMatcher, StructuralRegistration, rectify, transformed_rect},
        tracking::trajectory_dynamics,
    },
    application::VerifyRequest,
    color::Rgba,
    image_io::{load_luma, load_rgba},
    layers::{ForegroundReader, merge_mask},
    progress::ProgressReporter,
    render::{RENDER_MANIFEST_SCHEMA_VERSION, RenderManifest},
    scene::SurfaceSpace,
    surface::Surface,
    video::{self, Decoder},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct VerificationThresholds {
    pub overall: f64,
    pub tracking_lock: f64,
    pub rendered_title_plane_lock: f64,
    pub rendered_title_maximum_drift_pixels: f64,
    pub scene_integrity: f64,
    pub typography_fit: f64,
    pub typography_validity: f64,
    pub temporal_stability: f64,
    pub maximum_trajectory_residual_pixels: f64,
    pub occlusion_restore: f64,
    pub loop_seam: f64,
}

pub const VERIFICATION_REPORT_SCHEMA_VERSION: u32 = 12;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationReport {
    pub schema_version: u32,
    pub program_version: String,
    pub source_sha256: String,
    pub analysis_manifest_sha256: String,
    pub analysis_inputs_sha256: String,
    pub renderer_source_sha256: String,
    pub render_manifest_sha256: String,
    pub rendered_sha256: String,
    pub passed: bool,
    pub overall: f64,
    pub tracking_lock: f64,
    pub tracking_lock_basis: String,
    pub rendered_title_plane_lock: f64,
    pub rendered_title_plane_lock_basis: String,
    pub rendered_title_observed_frames: usize,
    pub rendered_title_maximum_drift_pixels: f64,
    pub rendered_title_worst_frame: usize,
    pub scene_integrity: f64,
    pub typography_fit: f64,
    pub typography_validity: f64,
    pub temporal_stability: f64,
    pub temporal_stability_basis: String,
    /// Raw second-difference smoothness of the stored four-corner trajectory.
    /// This remains diagnostic because real, independently observed plaque
    /// acceleration must not be classified as tracker jitter.
    pub trajectory_curvature_stability: f64,
    pub occlusion_restore: f64,
    pub loop_seam: f64,
    pub loop_seam_basis: String,
    pub loop_seam_mean_error: f64,
    pub title_effect_frame_mean_error: f64,
    pub structural_edge_alignment: f64,
    pub source_flow_observed_pairs: usize,
    pub source_flow_median_error_pixels: f64,
    pub source_flow_p95_error_pixels: f64,
    pub source_flow_p99_error_pixels: f64,
    /// Independent flow errors split by temporal baseline. Lag 1 is the hard
    /// single-frame-slip signal; longer lags expose slow screen-fixed drift but can
    /// become unobservable during large perspective changes or foreground crossings.
    pub source_flow_lag_1_observed_pairs: usize,
    pub source_flow_lag_1_p95_error_pixels: f64,
    pub source_flow_lag_1_p99_error_pixels: f64,
    pub source_flow_lag_6_observed_pairs: usize,
    pub source_flow_lag_6_p95_error_pixels: f64,
    pub source_flow_lag_6_p99_error_pixels: f64,
    pub source_flow_lag_12_observed_pairs: usize,
    pub source_flow_lag_12_p95_error_pixels: f64,
    pub source_flow_lag_12_p99_error_pixels: f64,
    pub source_flow_median_inlier_fraction: f64,
    pub source_flow_median_spatial_coverage: f64,
    pub source_flow_worst_frame: usize,
    /// True when source-flow feature selection was intersected with a complete
    /// source-pixel writing-surface sequence. The matte supplies membership/depth;
    /// material points inside it supply the independently measured rigid pose.
    pub source_flow_uses_writing_surface_support: bool,
    pub source_flow_writing_surface_supported_frames: usize,
    pub source_flow_writing_surface_fallback_frames: usize,
    pub tracking_measurement_valid: bool,
    pub tracking_observable_frames: usize,
    pub tracking_evidence_fraction: f64,
    pub tracking_median_spatial_coverage: f64,
    pub tracking_p95_uncertainty_pixels: f64,
    /// Diagnostic from appearance/template registration only. This is not an
    /// acceptance metric; animated lighting can make that optimizer suggest a
    /// spurious correction even when independent material flow is subpixel.
    pub template_registration_maximum_suggested_correction_pixels: Option<f64>,
    pub maximum_trajectory_residual_pixels: f64,
    pub worst_trajectory_frame: usize,
    pub loop_trajectory_residual_pixels: f64,
    pub untouched_region_mean_error: f64,
    pub structural_mean_error: f64,
    pub worst_tracking_frame: usize,
    pub worst_tracking_diagnostic: Option<String>,
    pub worst_scene_frame: usize,
    pub thresholds: VerificationThresholds,
    pub failures: Vec<String>,
    pub remedies: Vec<String>,
}

pub fn run(
    args: VerifyRequest,
    commands: &dyn crate::infrastructure::CommandExecutor,
) -> Result<()> {
    let mut progress = ProgressReporter::new(args.progress, args.progress_interval_ms);
    progress.start(1, 2, "Open verification inputs", None);
    let pack = Analysis::open(&args.analysis)?;
    let original = args.original.clone().unwrap_or_else(|| pack.source_path());
    let original_info = video::probe_with(commands, &args.ffprobe, &original)?;
    let rendered_info = video::probe_with(commands, &args.ffprobe, &args.rendered)?;
    original_info.ensure_supported_compositing_color()?;
    if !original_info.constant_frame_rate || !rendered_info.constant_frame_rate {
        bail!("verification requires constant-frame-rate source and rendered video");
    }
    if original_info.width != rendered_info.width || original_info.height != rendered_info.height {
        bail!(
            "render dimensions {}x{} differ from source {}x{}",
            rendered_info.width,
            rendered_info.height,
            original_info.width,
            original_info.height
        );
    }
    if original_info.frames != rendered_info.frames
        || (original_info.fps - rendered_info.fps).abs() > 1.0e-6
        || (original_info.start_time_seconds - rendered_info.start_time_seconds).abs() > 0.001
    {
        bail!(
            "render timing differs from source: source {} frames at {:.6} fps from {:.6}s, render {} frames at {:.6} fps from {:.6}s",
            original_info.frames,
            original_info.fps,
            original_info.start_time_seconds,
            rendered_info.frames,
            rendered_info.fps,
            rendered_info.start_time_seconds
        );
    }
    verify_color_metadata(&original_info, &rendered_info)?;
    let manifest_path = args.rendered.with_extension("render-manifest.json");
    let manifest_bytes = fs::read(&manifest_path)
        .with_context(|| format!("failed to read render manifest {}", manifest_path.display()))?;
    let manifest: RenderManifest = serde_json::from_slice(&manifest_bytes)?;
    if manifest.schema_version != RENDER_MANIFEST_SCHEMA_VERSION {
        bail!(
            "unsupported render manifest schema {}; expected {}",
            manifest.schema_version,
            RENDER_MANIFEST_SCHEMA_VERSION
        );
    }
    let source_sha256 = crate::digest::file_sha256(&original)?;
    let rendered_sha256 = crate::digest::file_sha256(&args.rendered)?;
    let analysis_manifest_sha256 =
        crate::digest::file_sha256(&pack.root.join(crate::analysis::MANIFEST_FILE))?;
    let analysis_inputs_sha256 =
        pack.render_inputs_sha256(manifest.used_analysis_occluder_masks)?;
    let render_manifest_sha256 = crate::digest::bytes_sha256(&manifest_bytes);
    if source_sha256 != pack.manifest.source.sha256
        || manifest.source_sha256 != source_sha256
        || manifest.analysis_manifest_sha256 != analysis_manifest_sha256
        || manifest.analysis_inputs_sha256 != analysis_inputs_sha256
        || manifest.rendered_sha256 != rendered_sha256
        || manifest.frames != rendered_info.frames
        || manifest.analyzer_build != pack.manifest.analyzer_build
        || manifest.renderer_build != crate::build_info::RENDERER_BUILD_VERSION
        || manifest.renderer_source_sha256 != crate::build_info::RENDERER_SOURCE_SHA256
        || manifest.used_analysis_occluder_masks
            != (crate::render::should_use_analysis_occluders(&pack)
                && pack.root.join(crate::analysis::OCCLUDER_DIR).is_dir())
        || manifest.used_injected_surface != pack.manifest.injected_surface.is_some()
    {
        bail!("render manifest provenance does not match the source, analysis, or rendered video");
    }
    let decision_trace = crate::render::load_decision_trace(&manifest_path, &manifest)?;
    let text_mask_path = manifest.canonical_text_mask.resolve_from(&manifest_path);
    let text_mask_image = image::open(&text_mask_path)
        .with_context(|| {
            format!(
                "failed to load canonical text mask {}",
                text_mask_path.display()
            )
        })?
        .to_luma8();
    if crate::digest::file_sha256(&text_mask_path)? != manifest.canonical_text_mask_sha256 {
        bail!("canonical text mask differs from its render-manifest identity");
    }
    match (
        &manifest.render_contact_sheet,
        &manifest.render_contact_sheet_sha256,
    ) {
        (Some(path), Some(expected_sha256)) => {
            let contact_sheet = path.resolve_from(&manifest_path);
            if crate::digest::file_sha256(&contact_sheet)? != *expected_sha256 {
                bail!("render contact sheet differs from its render-manifest identity");
            }
        }
        (None, None) => {}
        _ => bail!("render manifest has incomplete contact-sheet provenance"),
    }
    anyhow::ensure!(
        text_mask_image.width() == pack.manifest.canonical_width
            && text_mask_image.height() == pack.manifest.canonical_height,
        "canonical text mask dimensions do not match analysis"
    );
    let canonical_text_mask = text_mask_image.into_raw();
    let title_plane_support = dilate_binary_mask(
        &canonical_text_mask,
        pack.manifest.canonical_width as usize,
        pack.manifest.canonical_height as usize,
        6,
    );
    let mut canonical_allowed_mask = canonical_text_mask.clone();
    if let Some(surface) = &pack.manifest.injected_surface {
        let path = pack.require_asset_path(surface.path.as_path())?;
        let image = image::open(&path)
            .with_context(|| format!("failed to load injected plaque {}", path.display()))?
            .to_rgba8();
        anyhow::ensure!(
            image.width() == pack.manifest.canonical_width
                && image.height() == pack.manifest.canonical_height,
            "injected plaque dimensions do not match analysis"
        );
        for (allowed, pixel) in canonical_allowed_mask.iter_mut().zip(image.pixels()) {
            *allowed = source_over_alpha(*allowed, pixel.0[3]);
        }
    }
    let canonical_allowed_surface = Surface::from_alpha_mask(
        pack.manifest.canonical_width,
        pack.manifest.canonical_height,
        &canonical_allowed_mask,
        Rgba::new(255, 255, 255, 255),
    )?;
    let screen_canvas = pack.manifest.surface_space == SurfaceSpace::ScreenCanvas;
    let structural_mask = load_luma(
        &pack.require_asset(STRUCTURAL_MASK_FILE)?,
        pack.manifest.canonical_width,
        pack.manifest.canonical_height,
    )?;
    let structural_template = load_rgba(&pack.require_asset(STRUCTURAL_TEMPLATE_FILE)?)?;
    let registration_mask = load_luma(
        &pack.require_asset(REGISTRATION_MASK_FILE)?,
        pack.manifest.canonical_width,
        pack.manifest.canonical_height,
    )?;
    let registration_template = load_rgba(&pack.require_asset(REGISTRATION_TEMPLATE_FILE)?)?;
    let structural_matcher = StructuralMatcher::new(&registration_template, &registration_mask);
    let foregrounds = ForegroundReader::open(&pack, manifest.used_analysis_occluder_masks)?;
    let has_any_occluder = pack.manifest.has_occluder || !foregrounds.is_empty();
    let writing_surface_layer = pack.manifest.layers.iter().find(|layer| {
        layer.role == crate::scene::LayerRole::WritingSurface
            && layer.coordinates == crate::scene::LayerCoordinates::SourcePixels
            && layer.kind == crate::scene::LayerArtifactKind::AlphaSequence
            && layer.first_frame == Some(0)
            && layer.last_frame == Some(original_info.frames.saturating_sub(1))
    });
    // Verification never registers the rendered title: that would let a title
    // certify its own screen-fixed error. Low-texture physical surfaces are an
    // explicit unmeasurable/failing result until independent source evidence exists.
    progress.finish("dimensions, timing, manifest and masks are valid");

    progress.start(2, 2, "Verify every frame", Some(original_info.frames));
    let mut original_decoder = Decoder::spawn(&args.ffmpeg, &original, &original_info)?;
    let mut rendered_decoder = Decoder::spawn(&args.ffmpeg, &args.rendered, &rendered_info)?;
    let mut outside_error = 0_u64;
    let mut outside_count = 0_u64;
    let mut structural_error = 0_u64;
    let mut structural_count = 0_u64;
    let mut structural_alignment_sum = 0.0_f64;
    let mut structural_alignment_count = 0_u64;
    let mut template_registration_maximum_suggested_correction = 0.0_f64;
    let mut source_flow_errors = Vec::new();
    let mut source_flow_errors_by_lag = [Vec::new(), Vec::new(), Vec::new()];
    let mut source_flow_inliers = Vec::new();
    let mut source_flow_coverages = Vec::new();
    let mut source_flow_history: VecDeque<(usize, Surface, Vec<u8>)> = VecDeque::new();
    let mut source_flow_writing_surface_supported_frames = 0usize;
    let mut source_flow_writing_surface_fallback_frames = 0usize;
    let mut occlusion_error = 0_u64;
    let mut occlusion_count = 0_u64;
    let mut temporal_error = 0_u64;
    let mut temporal_count = 0_u64;
    let mut title_plane_lock_sum = 0.0_f64;
    let mut title_plane_lock_count = 0usize;
    let mut title_plane_maximum_drift = 0.0_f64;
    let mut title_plane_worst_frame = 0usize;
    let mut title_plane_matcher: Option<StructuralMatcher> = None;
    let mut previous_delta: Option<Vec<i16>> = None;
    let mut previous_occluder: Option<Vec<u8>> = None;
    let mut first_delta: Option<Vec<i16>> = None;
    let mut last_delta = Vec::new();
    let mut first_seam_occluder: Option<Vec<u8>> = None;
    let mut last_seam_occluder = Vec::new();
    let mut worst_tracking = (0usize, 1.0f64);
    let mut worst_tracking_preview = None;
    let mut worst_scene = (0usize, 0.0f64);
    let mut frame_index = 0usize;

    loop {
        let Some(original_frame) = original_decoder.next_frame()? else {
            break;
        };
        let rendered_frame = rendered_decoder
            .next_frame()?
            .context("render ended before source")?;
        let sample = pack
            .motion
            .get(frame_index)
            .with_context(|| format!("motion sample missing for frame {frame_index}"))?;

        let mut allowed = Surface::new(original_info.width, original_info.height);
        allowed.warp_blend(
            &canonical_allowed_surface,
            transformed_rect(pack.manifest.source_plaque_rect, sample.transform),
            1.0,
        )?;
        let allowed_mask = allowed.alpha_mask();
        let mut frame_outside_error = 0u64;
        let mut frame_outside_count = 0u64;
        for (&allowed_alpha, (source, rendered)) in allowed_mask.iter().zip(
            original_frame
                .pixels()
                .as_chunks::<4>()
                .0
                .iter()
                .zip(rendered_frame.pixels().as_chunks::<4>().0.iter()),
        ) {
            if allowed_alpha < 255 {
                let difference = (0..3)
                    .map(|channel| {
                        scene_channel_error(source[channel], rendered[channel], allowed_alpha)
                    })
                    .sum::<u64>();
                outside_error += difference;
                outside_count += 3;
                frame_outside_error += difference;
                frame_outside_count += 3;
            }
        }
        let frame_scene_mean = frame_outside_error as f64 / frame_outside_count.max(1) as f64;
        if frame_scene_mean > worst_scene.1 {
            worst_scene = (frame_index, frame_scene_mean);
        }

        let original_canonical = rectify(
            &original_frame,
            pack.manifest.source_plaque_rect,
            sample.transform,
        )?;
        for (&mask, (observed, template)) in structural_mask.iter().zip(
            original_canonical
                .pixels()
                .as_chunks::<4>()
                .0
                .iter()
                .zip(structural_template.pixels().as_chunks::<4>().0.iter()),
        ) {
            if mask > 64 {
                let difference = (0..3)
                    .map(|channel| observed[channel].abs_diff(template[channel]) as u64)
                    .sum::<u64>();
                structural_error += difference;
                structural_count += 3;
            }
        }
        let mut source_occluder = foregrounds
            .frame_mask(frame_index, sample.transform)?
            .unwrap_or_default();
        if manifest.used_analysis_occluder_masks {
            let path = pack
                .root
                .join(OCCLUDER_DIR)
                .join(format!("{frame_index:06}.png"));
            if path.exists() {
                let mask = image::open(&path)?.to_luma8();
                anyhow::ensure!(
                    mask.width() == original_info.width && mask.height() == original_info.height,
                    "occluder mask dimensions differ from source at frame {frame_index}"
                );
                merge_mask(&mut source_occluder, mask.as_raw());
            }
        }
        let source_flow_exclusion = if let Some(layer) = writing_surface_layer {
            let path = pack.require_asset_path(&crate::analysis::sequence_path(
                layer.path.as_path(),
                frame_index,
            ))?;
            let support = image::open(&path)
                .with_context(|| {
                    format!("failed to load writing-surface support {}", path.display())
                })?
                .to_luma8();
            anyhow::ensure!(
                support.width() == original_info.width && support.height() == original_info.height,
                "writing-surface support dimensions differ from source at frame {frame_index}"
            );
            let mut exclusion = source_occluder.clone();
            if exclusion.is_empty() {
                exclusion.resize(
                    original_info.width as usize * original_info.height as usize,
                    0,
                );
            }
            if crate::layers::writing_surface_support_is_plausible(
                support.as_raw(),
                original_info.width,
                original_info.height,
                pack.manifest.source_plaque_rect,
                sample.transform,
            ) {
                crate::layers::exclude_outside_surface_support(&mut exclusion, support.as_raw());
                source_flow_writing_surface_supported_frames += 1;
            } else {
                source_flow_writing_surface_fallback_frames += 1;
            }
            exclusion
        } else {
            source_occluder.clone()
        };
        for ((&alpha, source), rendered) in source_occluder
            .iter()
            .zip(original_frame.pixels().as_chunks::<4>().0.iter())
            .zip(rendered_frame.pixels().as_chunks::<4>().0.iter())
        {
            if alpha > 0 {
                occlusion_error += (0..3)
                    .map(|channel| {
                        restoration_channel_error(source[channel], rendered[channel], alpha)
                    })
                    .sum::<u64>();
                occlusion_count += 3;
            }
        }
        let canonical_occluder = if source_occluder.is_empty() {
            None
        } else {
            let full_mask = Surface::from_alpha_mask(
                original_info.width,
                original_info.height,
                &source_occluder,
                Rgba::new(255, 255, 255, 255),
            )?;
            Some(
                rectify(
                    &full_mask,
                    pack.manifest.source_plaque_rect,
                    sample.transform,
                )?
                .alpha_mask(),
            )
        };

        let frame_tracking_score = if sample.plaque_visibility >= 0.5
            && crate::analyze::tracking::surface_visible_fraction(
                pack.manifest.source_plaque_rect,
                sample.transform,
                original_info.width,
                original_info.height,
            ) >= 0.60
        {
            let visible_registration_mask =
                mask_excluding_foreground(&registration_mask, canonical_occluder.as_deref(), 16);
            let alignment = structural_edge_alignment(
                &original_canonical,
                &registration_template,
                &visible_registration_mask,
            );
            let usable_fraction = visible_registration_mask
                .iter()
                .filter(|&&value| value > 64)
                .count() as f64
                / registration_mask
                    .iter()
                    .filter(|&&value| value > 64)
                    .count()
                    .max(1) as f64;
            if usable_fraction < 0.30 {
                1.0
            } else {
                structural_alignment_sum += alignment;
                structural_alignment_count += 1;
                let structural_correction = match &structural_matcher {
                    Some(matcher) => matcher
                        .measure_excluding(&original_canonical, 4, canonical_occluder.as_deref())
                        .map(|registration| {
                            let relative_improvement = (registration.before - registration.after)
                                / registration.before.max(1.0);
                            // A moving light or fine foreground line can produce a
                            // slightly cheaper transform even when the surface is
                            // already aligned. Demand a material improvement before
                            // interpreting the optimizer's displacement as geometry.
                            if registration.after + 1.0 < registration.before
                                && relative_improvement >= 0.12
                                && registration.ecc.unwrap_or(1.0) >= 0.72
                            {
                                registration_correction_pixels(
                                    &registration,
                                    original_canonical.width(),
                                    original_canonical.height(),
                                )
                            } else {
                                0.0
                            }
                        })
                        .unwrap_or(f64::INFINITY),
                    None => f64::INFINITY,
                };
                let correction = structural_correction;
                template_registration_maximum_suggested_correction =
                    template_registration_maximum_suggested_correction.max(correction);
                tracking_lock_score(correction, alignment)
            }
        } else {
            1.0
        };

        if !screen_canvas {
            for (lag_index, lag) in [1usize, 6, 12].into_iter().enumerate() {
                // Consecutive evidence catches a single-frame slip. Longer baselines
                // make slow screen-fixed drift observable without tripling runtime.
                if lag > 1 && !frame_index.is_multiple_of(3) {
                    continue;
                }
                let Some(previous_index) = frame_index.checked_sub(lag) else {
                    continue;
                };
                let Some((_, previous_frame, previous_source_flow_exclusion)) = source_flow_history
                    .iter()
                    .find(|(index, _, _)| *index == previous_index)
                else {
                    continue;
                };
                let previous_sample = &pack.motion[previous_index];
                if sample.plaque_visibility < 0.5
                    || previous_sample.plaque_visibility < 0.5
                    || crate::analyze::tracking::surface_visible_fraction(
                        pack.manifest.source_plaque_rect,
                        sample.transform,
                        original_info.width,
                        original_info.height,
                    ) < 0.35
                    || crate::analyze::tracking::surface_visible_fraction(
                        pack.manifest.source_plaque_rect,
                        previous_sample.transform,
                        original_info.width,
                        original_info.height,
                    ) < 0.35
                {
                    continue;
                }
                let observation = crate::analyze::tracking::measure_source_flow_consistency(
                    previous_frame,
                    &original_frame,
                    pack.manifest.source_plaque_rect,
                    previous_sample.transform,
                    sample.transform,
                    Some(previous_source_flow_exclusion),
                    Some(&source_flow_exclusion),
                )?;
                let Some(observation) = observation.filter(|observation| {
                    observation.tracked_points >= 24
                        && observation.spatial_coverage >= 0.42
                        && observation.flow_model_inlier_fraction >= 0.68
                        && observation.flow_model_error_pixels <= 1.5
                }) else {
                    continue;
                };
                source_flow_errors.push(observation.median_error_pixels);
                source_flow_errors_by_lag[lag_index].push(observation.median_error_pixels);
                source_flow_inliers.push(observation.inlier_fraction);
                source_flow_coverages.push(observation.spatial_coverage);
                let score = source_flow_observation_score(observation.median_error_pixels);
                if score < worst_tracking.1 {
                    worst_tracking = (frame_index, score);
                    worst_tracking_preview = Some((
                        original_frame.clone(),
                        transformed_rect(pack.manifest.source_plaque_rect, sample.transform),
                    ));
                }
            }
        }

        let rendered_canonical = rectify(
            &rendered_frame,
            pack.manifest.source_plaque_rect,
            sample.transform,
        )?;
        let delta: Vec<i16> = rendered_canonical
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .zip(original_canonical.pixels().as_chunks::<4>().0.iter())
            .flat_map(|(rendered, source)| {
                (0..3).map(move |channel| rendered[channel] as i16 - source[channel] as i16)
            })
            .collect();
        if sample.plaque_visibility >= 0.85 && sample.occluder_coverage < 0.04 {
            let signature = title_difference_signature(
                &delta,
                &title_plane_support,
                canonical_occluder.as_deref(),
                pack.manifest.canonical_width,
                pack.manifest.canonical_height,
            )?;
            if let Some(matcher) = &title_plane_matcher {
                title_plane_lock_count += 1;
                if let Some(registration) = matcher.measure(&signature, 6) {
                    let drift = registration_correction_pixels(
                        &registration,
                        signature.width(),
                        signature.height(),
                    );
                    let score = (-drift / 1.75).exp().clamp(0.0, 1.0);
                    title_plane_lock_sum += score;
                    if drift > title_plane_maximum_drift {
                        title_plane_maximum_drift = drift;
                        title_plane_worst_frame = frame_index;
                    }
                } else {
                    title_plane_maximum_drift = title_plane_maximum_drift.max(12.0);
                    title_plane_worst_frame = frame_index;
                }
            } else {
                title_plane_matcher = StructuralMatcher::new(&signature, &title_plane_support);
            }
        }
        if let Some(previous) = &previous_delta {
            for (pixel_index, (&text_alpha, (current, prior))) in canonical_text_mask
                .iter()
                .zip(
                    delta
                        .as_chunks::<3>()
                        .0
                        .iter()
                        .zip(previous.as_chunks::<3>().0.iter()),
                )
                .enumerate()
            {
                let current_occluded = canonical_occluder
                    .as_ref()
                    .is_some_and(|mask| mask[pixel_index] >= 250);
                let previous_occluded = previous_occluder
                    .as_ref()
                    .is_some_and(|mask| mask[pixel_index] >= 250);
                if text_alpha > 32 && !current_occluded && !previous_occluded {
                    temporal_error += (0..3)
                        .map(|channel| current[channel].abs_diff(prior[channel]) as u64)
                        .sum::<u64>();
                    temporal_count += 3;
                }
            }
        }
        if first_delta.is_none() {
            first_delta = Some(delta.clone());
            first_seam_occluder = canonical_occluder.clone();
        }
        last_delta.clone_from(&delta);
        last_seam_occluder = canonical_occluder.clone().unwrap_or_default();
        previous_delta = Some(delta);
        previous_occluder = canonical_occluder;
        source_flow_history.push_back((frame_index, original_frame.clone(), source_flow_exclusion));
        while source_flow_history
            .front()
            .is_some_and(|(index, _, _)| frame_index.saturating_sub(*index) >= 12)
        {
            source_flow_history.pop_front();
        }
        frame_index += 1;
        progress.update(
            frame_index,
            format!(
                "structural lock {:.2}, outside error {:.4}",
                frame_tracking_score, frame_scene_mean
            ),
        );
    }
    original_decoder.finish()?;
    if rendered_decoder.next_frame()?.is_some() {
        bail!("render contains frames after the source ended");
    }
    rendered_decoder.finish()?;
    progress.finish(format!("{} frames", frame_index));

    let untouched_error = outside_error as f64 / outside_count.max(1) as f64;
    let structure_mean = structural_error as f64 / structural_count.max(1) as f64;
    let temporal_mean = temporal_error as f64 / temporal_count.max(1) as f64;
    let structural_edge_alignment = if structural_alignment_count == 0 {
        0.0
    } else {
        (structural_alignment_sum / structural_alignment_count as f64).clamp(0.0, 1.0)
    };
    let source_flow_observed_pairs = source_flow_errors.len();
    let mut sorted_source_flow_errors = source_flow_errors;
    let source_flow_median_error_pixels = percentile(&mut sorted_source_flow_errors, 0.50);
    let source_flow_p95_error_pixels = percentile(&mut sorted_source_flow_errors, 0.95);
    let source_flow_p99_error_pixels = percentile(&mut sorted_source_flow_errors, 0.99);
    let source_flow_lag_observed_pairs = source_flow_errors_by_lag.each_ref().map(Vec::len);
    let source_flow_lag_p95_error_pixels = source_flow_errors_by_lag
        .each_mut()
        .map(|values| percentile(values, 0.95));
    let source_flow_lag_p99_error_pixels = source_flow_errors_by_lag
        .each_mut()
        .map(|values| percentile(values, 0.99));
    let source_flow_median_inlier_fraction = percentile(&mut source_flow_inliers, 0.50);
    let source_flow_median_spatial_coverage = percentile(&mut source_flow_coverages, 0.50);
    let (
        source_flow_median_error_pixels,
        source_flow_p95_error_pixels,
        source_flow_p99_error_pixels,
        source_flow_median_inlier_fraction,
        source_flow_median_spatial_coverage,
    ) = if screen_canvas {
        (0.0, 0.0, 0.0, 1.0, 1.0)
    } else {
        (
            source_flow_median_error_pixels,
            source_flow_p95_error_pixels,
            source_flow_p99_error_pixels,
            source_flow_median_inlier_fraction,
            source_flow_median_spatial_coverage,
        )
    };
    let source_flow_uses_writing_surface_support = source_flow_writing_surface_supported_frames > 0;
    let measured_tracking_lock = source_flow_lock_score(
        source_flow_lag_p95_error_pixels,
        source_flow_lag_p99_error_pixels,
    );
    let tracking_observable_frames = pack
        .motion
        .iter()
        .filter(|sample| {
            crate::analyze::tracking::surface_visible_fraction(
                pack.manifest.source_plaque_rect,
                sample.transform,
                original_info.width,
                original_info.height,
            ) >= 0.15
        })
        .count();
    let evidence_frames = pack
        .motion
        .iter()
        .filter(|sample| {
            sample.measurement_valid
                && crate::analyze::tracking::surface_visible_fraction(
                    pack.manifest.source_plaque_rect,
                    sample.transform,
                    original_info.width,
                    original_info.height,
                ) >= 0.15
        })
        .count();
    let evidence_fraction = evidence_frames as f64 / tracking_observable_frames.max(1) as f64;
    let mut evidence_coverages = pack
        .motion
        .iter()
        .filter(|sample| {
            sample.measurement_valid
                && crate::analyze::tracking::surface_visible_fraction(
                    pack.manifest.source_plaque_rect,
                    sample.transform,
                    original_info.width,
                    original_info.height,
                ) >= 0.15
        })
        .map(|sample| sample.spatial_coverage)
        .collect::<Vec<_>>();
    let tracking_median_spatial_coverage = percentile(&mut evidence_coverages, 0.50);
    let mut evidence_uncertainties = pack
        .motion
        .iter()
        .filter(|sample| {
            sample.measurement_valid
                && sample.uncertainty_px.is_finite()
                && crate::analyze::tracking::surface_visible_fraction(
                    pack.manifest.source_plaque_rect,
                    sample.transform,
                    original_info.width,
                    original_info.height,
                ) >= 0.15
        })
        .map(|sample| sample.uncertainty_px)
        .collect::<Vec<_>>();
    let tracking_p95_uncertainty_pixels = percentile(&mut evidence_uncertainties, 0.95);
    let (evidence_fraction, tracking_median_spatial_coverage, tracking_p95_uncertainty_pixels) =
        if screen_canvas {
            (1.0, 1.0, 0.0)
        } else {
            (
                evidence_fraction,
                tracking_median_spatial_coverage,
                tracking_p95_uncertainty_pixels,
            )
        };
    let tracking_measurement_valid = screen_canvas
        || (source_flow_observed_pairs >= tracking_observable_frames.max(1) / 2
            && evidence_fraction >= 0.60
            && tracking_median_spatial_coverage >= 0.42
            && source_flow_median_spatial_coverage >= 0.42
            && source_flow_median_inlier_fraction >= 0.65
            && source_flow_p99_error_pixels.is_finite());
    let (rendered_title_plane_lock, rendered_title_plane_lock_basis) = if screen_canvas {
        (1.0, "not-applicable-screen-canvas")
    } else if title_plane_lock_count < 12 {
        (0.0, "insufficient-rendered-title-difference-evidence")
    } else {
        (
            (title_plane_lock_sum / title_plane_lock_count as f64).clamp(0.0, 1.0),
            "source-subtracted-title-registration-in-expected-surface-coordinates",
        )
    };
    let fully_reviewed_trajectory = is_fully_reviewed_trajectory(
        &decision_trace.tracking.trajectory_model,
        decision_trace.tracking.locked_keyframes,
        decision_trace.tracking.guide_keyframes,
        rendered_info.frames,
        &pack.motion,
    );
    let reviewed_title_evidence = fully_reviewed_trajectory && title_plane_lock_count >= 12;
    let (tracking_lock, tracking_lock_basis) = authoritative_tracking_lock(
        screen_canvas,
        reviewed_title_evidence,
        rendered_title_plane_lock,
        tracking_measurement_valid,
        measured_tracking_lock,
        source_flow_uses_writing_surface_support,
    );
    let scene_integrity = (-untouched_error / 1.5).exp().clamp(0.0, 1.0);
    let trajectory = trajectory_dynamics(
        &pack.motion,
        pack.manifest.source_plaque_rect,
        pack.manifest.loop_closed,
    );
    let temporal_stability = temporal_stability_score(
        trajectory.temporal_score,
        tracking_lock,
        tracking_measurement_valid || reviewed_title_evidence,
    );
    let temporal_stability_basis = if screen_canvas {
        "not-applicable-screen-canvas"
    } else if reviewed_title_evidence && tracking_lock > trajectory.temporal_score {
        "fully-reviewed-dense-trajectory-corroborated-by-direct-rendered-title-plane-lock"
    } else if tracking_measurement_valid && tracking_lock > trajectory.temporal_score {
        "trajectory-curvature-corroborated-by-independent-source-material-flow"
    } else {
        "quad-and-visibility-trajectory-curvature"
    };
    let occlusion_restore = if occlusion_count == 0 {
        if has_any_occluder { 0.40 } else { 1.0 }
    } else {
        (-(occlusion_error as f64 / occlusion_count as f64) / 1.5)
            .exp()
            .clamp(0.0, 1.0)
    };
    let seam_error = if pack.manifest.loop_closed {
        canonical_seam_error(
            first_delta.as_deref(),
            &last_delta,
            &canonical_text_mask,
            first_seam_occluder.as_deref(),
            &last_seam_occluder,
        )
    } else {
        0.0
    };
    let loop_seam = if pack.manifest.loop_closed {
        trajectory.loop_score
    } else {
        1.0
    };
    let (typography_fit, typography_validity) = typography_scores(&manifest);

    let overall = weighted_geometric_mean(&[
        (tracking_lock, 0.16),
        (rendered_title_plane_lock, 0.14),
        (scene_integrity, 0.20),
        (typography_fit, 0.14),
        (typography_validity, 0.10),
        (temporal_stability, 0.12),
        (occlusion_restore, 0.10),
        (loop_seam, 0.08),
    ]);
    let thresholds = VerificationThresholds {
        overall: args.minimum_score,
        tracking_lock: 0.95,
        rendered_title_plane_lock: 0.96,
        rendered_title_maximum_drift_pixels: 1.5,
        scene_integrity: 0.995,
        typography_fit: 0.98,
        typography_validity: 1.0,
        temporal_stability: 0.95,
        maximum_trajectory_residual_pixels: 4.0,
        occlusion_restore: 0.95,
        loop_seam: 0.98,
    };
    let mut failures = Vec::new();
    let mut remedies = Vec::new();
    let worst_tracking_diagnostic = if screen_canvas {
        None
    } else if let (Some(directory), Some((mut frame, quad))) =
        (&args.diagnostics, worst_tracking_preview)
    {
        fs::create_dir_all(directory).with_context(|| {
            format!(
                "failed to create verification diagnostics {}",
                directory.display()
            )
        })?;
        draw_quad(&mut frame, quad, Rgba::new(255, 220, 0, 255));
        let path = directory.join("verification-worst-tracking-frame.png");
        save_surface(&frame, &path)?;
        Some(
            path.file_name()
                .context("verification diagnostic has no file name")?
                .to_string_lossy()
                .into_owned(),
        )
    } else {
        None
    };
    let rect = pack.manifest.source_plaque_rect;
    let frame_seconds = worst_tracking.0 as f64 / original_info.fps;
    let tracking_remedy = if screen_canvas {
        "tracking is not applicable to an intentional screen-canvas surface".to_string()
    } else if tracking_measurement_valid {
        format!(
            "independent material flow{} disagrees with the four-corner trajectory most at frame {} ({frame_seconds:.3}s){}; p95 error {:.2}px and p99 error {:.2}px. The analyzed rectangle is {:.0},{:.0},{:.0},{:.0}. Correct scene bounds, surface support, or tracking before reanalysis",
            if source_flow_uses_writing_surface_support {
                " inside the writing-surface matte"
            } else {
                ""
            },
            worst_tracking.0,
            worst_tracking_diagnostic
                .as_ref()
                .map(|path| format!(", saved as {path}"))
                .unwrap_or_default(),
            source_flow_p95_error_pixels,
            source_flow_p99_error_pixels,
            rect.x,
            rect.y,
            rect.width,
            rect.height
        )
    } else {
        format!(
            "tracking could not be independently measured: {} source-flow pairs, {:.1}% median flow inliers, {:.1}% median flow coverage, and {:.1}% analyzed-frame evidence. Inspect the tracked rectangle {:.0},{:.0},{:.0},{:.0}, its foreground masks, and tracking-contact-sheet.png",
            source_flow_observed_pairs,
            source_flow_median_inlier_fraction * 100.0,
            source_flow_median_spatial_coverage * 100.0,
            evidence_fraction * 100.0,
            rect.x,
            rect.y,
            rect.width,
            rect.height
        )
    };
    check_score(
        "tracking_lock",
        tracking_lock,
        thresholds.tracking_lock,
        &mut failures,
        &mut remedies,
        tracking_remedy,
    );
    check_score(
        "rendered_title_plane_lock",
        rendered_title_plane_lock,
        thresholds.rendered_title_plane_lock,
        &mut failures,
        &mut remedies,
        format!(
            "the rendered title drifts by up to {:.2}px in expected plaque coordinates at frame {} ({:.3}s); inspect the trajectory, compositing transform, and foreground mask timing",
            title_plane_maximum_drift,
            title_plane_worst_frame,
            title_plane_worst_frame as f64 / original_info.fps
        ),
    );
    if !screen_canvas
        && title_plane_maximum_drift > thresholds.rendered_title_maximum_drift_pixels + f64::EPSILON
    {
        failures.push(format!(
            "rendered_title_maximum_drift_pixels {:.4} exceeds {:.4}",
            title_plane_maximum_drift, thresholds.rendered_title_maximum_drift_pixels
        ));
        remedies.push(format!(
            "inspect frame {} ({:.3}s); even a one-frame title/plaque slip is an acceptance failure",
            title_plane_worst_frame,
            title_plane_worst_frame as f64 / original_info.fps
        ));
    }
    check_score(
        "scene_integrity",
        scene_integrity,
        thresholds.scene_integrity,
        &mut failures,
        &mut remedies,
        if pack.manifest.injected_surface.is_some() {
            "use the default lossless FFV1 output and confirm changes outside the injected plaque/title/foreground masks are unintended".into()
        } else {
            "use the default lossless FFV1 output and confirm the source plaque is text-free".into()
        },
    );
    check_score(
        "typography_fit",
        typography_fit,
        thresholds.typography_fit,
        &mut failures,
        &mut remedies,
        "use --fit maximize; if it still fails, reduce --padding, increase --max-lines, or choose a narrower font".into(),
    );
    check_score(
        "typography_validity",
        typography_validity,
        thresholds.typography_validity,
        &mut failures,
        &mut remedies,
        "choose a font containing every requested glyph; no fallback font is allowed".into(),
    );
    check_score(
        "temporal_stability",
        temporal_stability,
        thresholds.temporal_stability,
        &mut failures,
        &mut remedies,
        format!(
            "trajectory changes abruptly at frame {} ({:.3}s), with analysis inertia {:.2}; export the motion and lock incorrect frames before reanalysis",
            trajectory.worst_frame,
            trajectory.worst_frame as f64 / original_info.fps,
            tracking_inertia(&pack.manifest.trajectory_model).unwrap_or(0.35)
        ),
    );
    if trajectory.maximum_residual > thresholds.maximum_trajectory_residual_pixels + f64::EPSILON {
        failures.push(format!(
            "maximum_trajectory_residual_pixels {:.4} exceeds {:.4}",
            trajectory.maximum_residual, thresholds.maximum_trajectory_residual_pixels
        ));
        remedies.push(format!(
            "trajectory has a localized four-corner motion impulse at frame {} ({:.3}s); repair that pose even if the clip-average temporal score passes",
            trajectory.worst_frame,
            trajectory.worst_frame as f64 / original_info.fps,
        ));
    }
    check_score(
        "occlusion_restore",
        occlusion_restore,
        thresholds.occlusion_restore,
        &mut failures,
        &mut remedies,
        "add or correct a foreground scene where automatic separation is wrong".into(),
    );
    check_score(
        "loop_seam",
        loop_seam,
        thresholds.loop_seam,
        &mut failures,
        &mut remedies,
        "lock the first and last motion frames when the source is intended to loop".into(),
    );
    if overall < thresholds.overall {
        failures.push(format!(
            "overall score {overall:.4} is below {:.4}",
            thresholds.overall
        ));
    }
    let passed = failures.is_empty();
    let report = VerificationReport {
        schema_version: VERIFICATION_REPORT_SCHEMA_VERSION,
        program_version: env!("CARGO_PKG_VERSION").to_string(),
        source_sha256,
        analysis_manifest_sha256,
        analysis_inputs_sha256,
        renderer_source_sha256: crate::build_info::RENDERER_SOURCE_SHA256.to_string(),
        render_manifest_sha256,
        rendered_sha256,
        passed,
        overall,
        tracking_lock,
        tracking_lock_basis: tracking_lock_basis.to_string(),
        rendered_title_plane_lock,
        rendered_title_plane_lock_basis: rendered_title_plane_lock_basis.to_string(),
        rendered_title_observed_frames: title_plane_lock_count,
        rendered_title_maximum_drift_pixels: title_plane_maximum_drift,
        rendered_title_worst_frame: title_plane_worst_frame,
        scene_integrity,
        typography_fit,
        typography_validity,
        temporal_stability,
        temporal_stability_basis: temporal_stability_basis.to_string(),
        trajectory_curvature_stability: trajectory.temporal_score,
        occlusion_restore,
        loop_seam,
        loop_seam_basis: "circular-trajectory-curvature".to_string(),
        loop_seam_mean_error: seam_error,
        title_effect_frame_mean_error: temporal_mean,
        structural_edge_alignment,
        source_flow_observed_pairs,
        source_flow_median_error_pixels,
        source_flow_p95_error_pixels,
        source_flow_p99_error_pixels,
        source_flow_lag_1_observed_pairs: source_flow_lag_observed_pairs[0],
        source_flow_lag_1_p95_error_pixels: source_flow_lag_p95_error_pixels[0],
        source_flow_lag_1_p99_error_pixels: source_flow_lag_p99_error_pixels[0],
        source_flow_lag_6_observed_pairs: source_flow_lag_observed_pairs[1],
        source_flow_lag_6_p95_error_pixels: source_flow_lag_p95_error_pixels[1],
        source_flow_lag_6_p99_error_pixels: source_flow_lag_p99_error_pixels[1],
        source_flow_lag_12_observed_pairs: source_flow_lag_observed_pairs[2],
        source_flow_lag_12_p95_error_pixels: source_flow_lag_p95_error_pixels[2],
        source_flow_lag_12_p99_error_pixels: source_flow_lag_p99_error_pixels[2],
        source_flow_median_inlier_fraction,
        source_flow_median_spatial_coverage,
        source_flow_worst_frame: if screen_canvas { 0 } else { worst_tracking.0 },
        source_flow_uses_writing_surface_support,
        source_flow_writing_surface_supported_frames,
        source_flow_writing_surface_fallback_frames,
        tracking_measurement_valid,
        tracking_observable_frames,
        tracking_evidence_fraction: evidence_fraction,
        tracking_median_spatial_coverage,
        tracking_p95_uncertainty_pixels,
        template_registration_maximum_suggested_correction_pixels: structural_matcher
            .is_some()
            .then_some(template_registration_maximum_suggested_correction),
        maximum_trajectory_residual_pixels: trajectory.maximum_residual,
        worst_trajectory_frame: trajectory.worst_frame,
        loop_trajectory_residual_pixels: trajectory.loop_residual,
        untouched_region_mean_error: untouched_error,
        structural_mean_error: structure_mean,
        worst_tracking_frame: if screen_canvas { 0 } else { worst_tracking.0 },
        worst_tracking_diagnostic,
        worst_scene_frame: worst_scene.0,
        thresholds,
        failures,
        remedies,
    };
    let json = serde_json::to_string_pretty(&report)?;
    if let Some(path) = args.report {
        crate::staged_output::write_file(&path, json.as_bytes(), true)
            .with_context(|| format!("failed to write verification report {}", path.display()))?;
    }
    println!("{json}");
    if !report.passed {
        bail!(
            "verification failed with score {:.3}; follow the remedies in the verification report",
            report.overall
        );
    }
    Ok(())
}

fn typography_scores(manifest: &RenderManifest) -> (f64, f64) {
    let metrics = &manifest.typography;
    let fit = if metrics.fit_mode == "maximize" {
        (metrics.font_size as f64 / metrics.maximum_safe_font_size.max(0.001) as f64)
            .clamp(0.0, 1.0)
    } else {
        1.0
    };
    let validity = if metrics.missing_glyphs == 0
        && metrics.fallback_glyphs == 0
        && metrics.clipped_pixels == 0
    {
        1.0
    } else {
        0.0
    };
    (fit, validity)
}

fn verify_color_metadata(source: &video::VideoInfo, rendered: &video::VideoInfo) -> Result<()> {
    for (name, expected, actual) in [
        (
            "range",
            source.color_range.as_deref(),
            rendered.color_range.as_deref(),
        ),
        (
            "space",
            source.color_space.as_deref(),
            rendered.color_space.as_deref(),
        ),
        (
            "transfer",
            source.color_transfer.as_deref(),
            rendered.color_transfer.as_deref(),
        ),
        (
            "primaries",
            source.color_primaries.as_deref(),
            rendered.color_primaries.as_deref(),
        ),
    ] {
        if let Some(expected) = expected
            && actual != Some(expected)
        {
            bail!(
                "render did not preserve source color {name}: expected {expected}, found {}",
                actual.unwrap_or("unspecified")
            );
        }
    }
    let normalized_rotation = |degrees: i32| degrees.rem_euclid(360);
    if normalized_rotation(source.rotation_degrees)
        != normalized_rotation(rendered.rotation_degrees)
    {
        bail!(
            "render did not preserve source rotation metadata: expected {} degrees, found {}",
            source.rotation_degrees,
            rendered.rotation_degrees
        );
    }
    Ok(())
}

/// Error that cannot be explained by source-over compositing at `allowed_alpha`.
/// A one-level allowance covers integer rounding in the compositor.
fn scene_channel_error(source: u8, rendered: u8, allowed_alpha: u8) -> u64 {
    crate::surface::constrained_linear_mixture_error(source, rendered, 255 - allowed_alpha)
}

/// Combined coverage of two independently composited layers. Using `max` here
/// underestimates the legal change where translucent title and plaque pixels overlap.
fn source_over_alpha(bottom: u8, top: u8) -> u8 {
    let remaining = (u16::from(255 - bottom) * u16::from(255 - top) + 127) / 255;
    255 - remaining as u8
}

fn title_difference_signature(
    delta: &[i16],
    support: &[u8],
    occluder: Option<&[u8]>,
    width: u32,
    height: u32,
) -> Result<Surface> {
    anyhow::ensure!(
        delta.len() == support.len() * 3,
        "title-difference dimensions are inconsistent"
    );
    let mut pixels = vec![0u8; support.len() * 4];
    for (pixel, (&allowed, channels)) in support
        .iter()
        .zip(delta.as_chunks::<3>().0.iter())
        .enumerate()
    {
        let hidden = occluder.is_some_and(|mask| mask.get(pixel).is_some_and(|&alpha| alpha >= 32));
        if allowed <= 16 || hidden {
            continue;
        }
        let magnitude = channels
            .iter()
            .map(|value| value.unsigned_abs() as u32)
            .sum::<u32>()
            / 3;
        // The verifier measures the geometry of the rendered mark, not its paint.
        // A binary difference signature is invariant to a moving shine, gradient,
        // pulse, or other color-only title animation.
        let value = if magnitude >= 6 { 255 } else { 0 };
        let base = pixel * 4;
        pixels[base..base + 4].copy_from_slice(&[value, value, value, 255]);
    }
    Surface::from_rgba(width, height, pixels)
}

fn dilate_binary_mask(source: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    if source.len() != width * height || width == 0 || height == 0 {
        return Vec::new();
    }
    let mut horizontal = vec![0u8; source.len()];
    for y in 0..height {
        for x in 0..width {
            let start = x.saturating_sub(radius);
            let end = (x + radius).min(width - 1);
            if source[y * width + start..=y * width + end]
                .iter()
                .any(|&alpha| alpha > 16)
            {
                horizontal[y * width + x] = 255;
            }
        }
    }
    let mut output = vec![0u8; source.len()];
    for y in 0..height {
        for x in 0..width {
            let start = y.saturating_sub(radius);
            let end = (y + radius).min(height - 1);
            if (start..=end).any(|yy| horizontal[yy * width + x] > 0) {
                output[y * width + x] = 255;
            }
        }
    }
    output
}

fn mask_excluding_foreground(stable: &[u8], foreground: Option<&[u8]>, threshold: u8) -> Vec<u8> {
    let Some(foreground) = foreground.filter(|mask| mask.len() == stable.len()) else {
        return stable.to_vec();
    };
    stable
        .iter()
        .zip(foreground)
        .map(|(&evidence, &occlusion)| if occlusion >= threshold { 0 } else { evidence })
        .collect()
}

/// Error beyond the maximum residual left by restoring the source at `restore_alpha`.
/// A one-level allowance covers integer rounding in the compositor.
fn restoration_channel_error(source: u8, rendered: u8, restore_alpha: u8) -> u64 {
    crate::surface::constrained_linear_mixture_error(source, rendered, restore_alpha)
}

fn tracking_inertia(model: &str) -> Option<f64> {
    model
        .rsplit_once("regularization-")?
        .1
        .split(|character: char| !(character.is_ascii_digit() || character == '.'))
        .next()?
        .parse()
        .ok()
}

fn percentile(values: &mut [f64], quantile: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    if values.is_empty() {
        return f64::INFINITY;
    }
    let index = ((values.len() - 1) as f64 * quantile.clamp(0.0, 1.0)).round() as usize;
    values[index]
}

fn save_surface(surface: &Surface, path: &Path) -> Result<()> {
    let image =
        image::RgbaImage::from_raw(surface.width(), surface.height(), surface.pixels().to_vec())
            .context("invalid verification diagnostic image")?;
    image
        .save(path)
        .with_context(|| format!("failed to save verification diagnostic {}", path.display()))?;
    Ok(())
}

fn draw_quad(surface: &mut Surface, quad: crate::geometry::Quad, color: Rgba) {
    let points = quad.points();
    for index in 0..4 {
        draw_line(surface, points[index], points[(index + 1) % 4], color);
    }
}

fn draw_line(
    surface: &mut Surface,
    start: crate::geometry::Point,
    end: crate::geometry::Point,
    color: Rgba,
) {
    let steps = (end.x - start.x)
        .abs()
        .max((end.y - start.y).abs())
        .ceil()
        .max(1.0) as usize;
    for step in 0..=steps {
        let t = step as f64 / steps as f64;
        let x = (start.x + (end.x - start.x) * t).round() as i32;
        let y = (start.y + (end.y - start.y) * t).round() as i32;
        for dy in -2..=2 {
            for dx in -2..=2 {
                if x + dx >= 0 && y + dy >= 0 {
                    surface.set_pixel((x + dx) as u32, (y + dy) as u32, color);
                }
            }
        }
    }
}

fn structural_edge_alignment(observed: &Surface, template: &Surface, mask: &[u8]) -> f64 {
    let width = observed.width() as usize;
    let height = observed.height() as usize;
    if width != template.width() as usize
        || height != template.height() as usize
        || mask.len() != width * height
        || width < 3
        || height < 3
    {
        return 0.0;
    }

    let luma = |surface: &Surface, index: usize| -> f64 {
        let pixel = &surface.pixels()[index * 4..index * 4 + 3];
        (f64::from(pixel[0]) * 54.0 + f64::from(pixel[1]) * 183.0 + f64::from(pixel[2]) * 19.0)
            / 256.0
    };
    let mut weighted_score = 0.0;
    let mut total_weight = 0.0;
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let index = y * width + x;
            if mask[index] <= 64 {
                continue;
            }
            let template_x = luma(template, index + 1) - luma(template, index - 1);
            let template_y = luma(template, index + width) - luma(template, index - width);
            let template_magnitude = template_x.hypot(template_y);
            if template_magnitude < 8.0 {
                continue;
            }
            let observed_x = luma(observed, index + 1) - luma(observed, index - 1);
            let observed_y = luma(observed, index + width) - luma(observed, index - width);
            let observed_magnitude = observed_x.hypot(observed_y);
            let direction = ((template_x * observed_x + template_y * observed_y)
                / (template_magnitude * observed_magnitude).max(1.0))
            .clamp(0.0, 1.0);
            let presence = (observed_magnitude / 8.0).clamp(0.0, 1.0);
            let weight = f64::from(mask[index]) / 255.0 * template_magnitude.min(255.0);
            weighted_score += direction * presence * weight;
            total_weight += weight;
        }
    }
    if total_weight <= f64::EPSILON {
        0.0
    } else {
        (weighted_score / total_weight).clamp(0.0, 1.0)
    }
}

fn registration_correction_pixels(
    registration: &StructuralRegistration,
    width: u32,
    height: u32,
) -> f64 {
    if registration.after + 0.25 >= registration.before {
        return 0.0;
    }
    let corners = [
        (0.0, 0.0),
        (f64::from(width.saturating_sub(1)), 0.0),
        (
            f64::from(width.saturating_sub(1)),
            f64::from(height.saturating_sub(1)),
        ),
        (0.0, f64::from(height.saturating_sub(1))),
    ];
    corners
        .iter()
        .map(|&(x, y)| {
            let corrected = registration
                .transform
                .transform(crate::model::PointF { x, y });
            (corrected.x - x).hypot(corrected.y - y)
        })
        .sum::<f64>()
        / corners.len() as f64
}

fn registration_lock_score(correction_pixels: f64) -> f64 {
    if !correction_pixels.is_finite() {
        return 0.0;
    }
    let excess = (correction_pixels - 0.75).max(0.0);
    (-(excess / 2.0).powi(2)).exp().clamp(0.0, 1.0)
}

fn tracking_lock_score(correction_pixels: f64, edge_alignment: f64) -> f64 {
    // Edge alignment is a support/observability gate, not a direct quality
    // multiplier: animated specular highlights naturally change gradient strength
    // on a correctly rectified metal or glass plaque.
    if edge_alignment < 0.18 {
        0.0
    } else {
        registration_lock_score(correction_pixels)
    }
}

fn source_flow_observation_score(error_pixels: f64) -> f64 {
    if !error_pixels.is_finite() {
        return 0.0;
    }
    let excess = (error_pixels - 0.75).max(0.0);
    (-(excess / 1.5).powi(2)).exp().clamp(0.0, 1.0)
}

fn source_flow_distribution_score(p95_error_pixels: f64, p99_error_pixels: f64) -> f64 {
    if !p95_error_pixels.is_finite() || !p99_error_pixels.is_finite() {
        return 0.0;
    }
    let p95_excess = (p95_error_pixels - 0.85).max(0.0);
    let p99_excess = (p99_error_pixels - 1.50).max(0.0);
    (-((p95_excess / 1.6).powi(2) + (p99_excess / 2.4).powi(2)))
        .exp()
        .clamp(0.0, 1.0)
}

fn source_flow_lock_score(
    p95_error_pixels_by_lag: [f64; 3],
    p99_error_pixels_by_lag: [f64; 3],
) -> f64 {
    if p95_error_pixels_by_lag
        .iter()
        .any(|value| !value.is_finite())
    {
        return 0.0;
    }

    // Consecutive-frame tails are the reliable signal for a localized slip.
    // At longer baselines, a few otherwise-valid flow tracks can cross a thin
    // foreground strand or cease to describe the same material point. Their p95
    // still exposes sustained drift without allowing that sparse tail to dominate.
    let consecutive =
        source_flow_distribution_score(p95_error_pixels_by_lag[0], p99_error_pixels_by_lag[0]);
    let multiscale_p95 = p95_error_pixels_by_lag.into_iter().fold(0.0_f64, f64::max);
    let sustained_excess = (multiscale_p95 - 0.85).max(0.0);
    let sustained = (-(sustained_excess / 1.6).powi(2)).exp().clamp(0.0, 1.0);
    consecutive.min(sustained)
}

fn is_fully_reviewed_trajectory(
    model: &str,
    locked_keyframes: usize,
    guide_keyframes: usize,
    frames: usize,
    motion: &[crate::model::MotionSample],
) -> bool {
    model.starts_with("reviewed-dense-quad-track-")
        && locked_keyframes == frames
        && guide_keyframes == 0
        && motion.len() == frames
        && motion.iter().all(|sample| {
            sample.measurement_valid && sample.measurement_source == "reviewed-dense-quad"
        })
}

fn authoritative_tracking_lock(
    screen_canvas: bool,
    reviewed_title_evidence: bool,
    rendered_title_plane_lock: f64,
    source_flow_measurement_valid: bool,
    source_flow_lock: f64,
    source_flow_uses_writing_surface_support: bool,
) -> (f64, &'static str) {
    if screen_canvas {
        (1.0, "not-applicable-screen-canvas")
    } else if reviewed_title_evidence {
        (
            rendered_title_plane_lock.clamp(0.0, 1.0),
            "fully-reviewed-dense-trajectory-and-direct-rendered-title-plane-lock",
        )
    } else if !source_flow_measurement_valid {
        (0.0, "unmeasurable-independent-source-evidence")
    } else if source_flow_uses_writing_surface_support {
        (
            source_flow_lock,
            "independent-lag-1-tail-and-multiscale-p95-source-material-flow-with-writing-surface-support-versus-four-corner-trajectory",
        )
    } else {
        (
            source_flow_lock,
            "independent-lag-1-tail-and-multiscale-p95-source-material-flow-versus-four-corner-trajectory",
        )
    }
}

fn temporal_stability_score(
    trajectory_curvature_score: f64,
    tracking_lock: f64,
    tracking_measurement_valid: bool,
) -> f64 {
    let curvature = if trajectory_curvature_score.is_finite() {
        trajectory_curvature_score.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if tracking_measurement_valid && tracking_lock.is_finite() {
        // Curvature alone cannot distinguish physical acceleration from tracker
        // jitter. Corroborating source flow can do so for automatic tracks; direct
        // title-plane lock provides the equivalent evidence for a fully reviewed one.
        curvature.max(tracking_lock.clamp(0.0, 1.0))
    } else {
        curvature
    }
}

fn canonical_seam_error(
    first: Option<&[i16]>,
    last: &[i16],
    text_mask: &[u8],
    first_occluder: Option<&[u8]>,
    last_occluder: &[u8],
) -> f64 {
    let Some(first) = first else {
        return 255.0;
    };
    if first.len() != last.len() || first.len() != text_mask.len() * 3 {
        return 255.0;
    }
    let mut error = 0_u64;
    let mut count = 0_u64;
    for (pixel, &text_alpha) in text_mask.iter().enumerate() {
        let first_hidden = first_occluder.is_some_and(|mask| mask[pixel] >= 250);
        let last_hidden = last_occluder.get(pixel).is_some_and(|&alpha| alpha >= 250);
        if text_alpha <= 32 || first_hidden || last_hidden {
            continue;
        }
        for channel in 0..3 {
            let index = pixel * 3 + channel;
            error += (first[index] - last[index]).unsigned_abs() as u64;
            count += 1;
        }
    }
    if count == 0 {
        255.0
    } else {
        error as f64 / count as f64
    }
}

fn check_score(
    name: &str,
    value: f64,
    threshold: f64,
    failures: &mut Vec<String>,
    remedies: &mut Vec<String>,
    remedy: String,
) {
    if value + 1.0e-12 < threshold {
        failures.push(format!("{name} {value:.4} is below {threshold:.4}"));
        remedies.push(remedy);
    }
}

fn weighted_geometric_mean(values: &[(f64, f64)]) -> f64 {
    let total_weight: f64 = values.iter().map(|(_, weight)| weight).sum();
    let weighted_log_sum: f64 = values
        .iter()
        .map(|(value, weight)| value.clamp(1.0e-9, 1.0).ln() * weight)
        .sum();
    (weighted_log_sum / total_weight).exp()
}

#[cfg(test)]
mod tests {
    use super::{
        authoritative_tracking_lock, canonical_seam_error, is_fully_reviewed_trajectory,
        mask_excluding_foreground, registration_correction_pixels, registration_lock_score,
        restoration_channel_error, scene_channel_error, source_flow_lock_score, source_over_alpha,
        structural_edge_alignment, temporal_stability_score, tracking_lock_score,
        trajectory_dynamics,
    };
    use crate::{
        analyze::extraction::measure_structural_registration,
        color::Rgba,
        geometry::{Point, Quad},
        model::{Mat3, MotionSample, RectF},
        surface::Surface,
    };

    fn motion(frame: usize, x: f64) -> MotionSample {
        MotionSample {
            frame,
            transform: Mat3::translation(x, 0.0),
            measurement_valid: true,
            tracked_points: 20,
            spatial_coverage: 1.0,
            uncertainty_px: 0.25,
            measurement_source: "test".into(),
            inlier_ratio: 1.0,
            reprojection_error: 0.0,
            ecc: Some(1.0),
            plaque_visibility: 1.0,
            occluder_coverage: 0.0,
        }
    }

    #[test]
    fn canonical_seam_ignores_pixels_outside_text_and_occlusion() {
        let first = [10_i16, 20, 30, 80, 90, 100];
        let last = [10_i16, 20, 30, 0, 0, 0];
        let text = [255_u8, 255];
        let occluder = [0_u8, 255];
        assert_eq!(
            canonical_seam_error(Some(&first), &last, &text, Some(&occluder), &occluder),
            0.0
        );
    }

    #[test]
    fn alpha_aware_integrity_bounds_match_source_over_compositing() {
        assert_eq!(source_over_alpha(0, 128), 128);
        assert_eq!(source_over_alpha(128, 0), 128);
        assert_eq!(source_over_alpha(128, 128), 192);
        assert_eq!(source_over_alpha(255, 128), 255);

        assert_eq!(scene_channel_error(0, 188, 128), 0);
        assert_eq!(scene_channel_error(0, 190, 128), 1);
        assert_eq!(scene_channel_error(20, 21, 0), 0);
        assert_eq!(scene_channel_error(20, 23, 0), 2);

        assert_eq!(restoration_channel_error(0, 188, 128), 0);
        assert_eq!(restoration_channel_error(0, 190, 128), 2);
        assert_eq!(restoration_channel_error(20, 21, 255), 0);
        assert_eq!(restoration_channel_error(20, 23, 255), 2);
    }

    #[test]
    fn structural_alignment_rewards_registered_edges() {
        let mut template = Surface::new(16, 16);
        let mut shifted = Surface::new(16, 16);
        for y in 0..16 {
            for x in 0..16 {
                let value = if x >= 8 { 240 } else { 10 };
                template.set_pixel(x, y, Rgba::new(value, value, value, 255));
                let shifted_value = if x >= 11 { 240 } else { 10 };
                shifted.set_pixel(
                    x,
                    y,
                    Rgba::new(shifted_value, shifted_value, shifted_value, 255),
                );
            }
        }
        let mut mask = vec![0_u8; 16 * 16];
        for y in 1..15 {
            mask[y * 16 + 7] = 255;
            mask[y * 16 + 8] = 255;
        }
        let registered = structural_edge_alignment(&template, &template, &mask);
        let displaced = structural_edge_alignment(&shifted, &template, &mask);
        assert!(registered > 0.99);
        assert!(displaced < 0.10);
    }

    #[test]
    fn foreground_exclusion_removes_only_hidden_registration_pixels() {
        let stable = [255, 128, 64, 255];
        let foreground = [0, 15, 16, 255];
        assert_eq!(
            mask_excluding_foreground(&stable, Some(&foreground), 16),
            vec![255, 128, 0, 0]
        );
    }

    #[test]
    fn trajectory_score_rejects_a_single_frame_jump() {
        let rect = RectF {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 50.0,
        };
        let smooth = (0..9)
            .map(|frame| motion(frame, frame as f64))
            .collect::<Vec<_>>();
        let mut jumped = smooth.clone();
        jumped[4].transform = Mat3::translation(8.0, 0.0);

        assert!(trajectory_dynamics(&smooth, rect, false).temporal_score > 0.99);
        assert!(trajectory_dynamics(&jumped, rect, false).temporal_score < 0.80);
    }

    #[test]
    fn registration_score_rejects_a_displaced_structure() {
        let mut template = Surface::new(64, 64);
        let mut shifted = Surface::new(64, 64);
        let mut mask = vec![0_u8; 64 * 64];
        for y in 8..56 {
            for x in 8..56 {
                let edge = x == 8 || x == 55 || y == 8 || y == 55;
                if edge {
                    template.set_pixel(x, y, Rgba::new(240, 240, 240, 255));
                    shifted.set_pixel(x + 3, y, Rgba::new(240, 240, 240, 255));
                }
                if edge {
                    mask[y as usize * 64 + x as usize] = 255;
                }
            }
        }
        let registration = measure_structural_registration(&template, &shifted, &mask, 4).unwrap();
        let correction = registration_correction_pixels(&registration, 64, 64);

        assert!(registration.after < registration.before);
        assert!(correction > 2.0);
        assert!(registration_lock_score(correction) < 0.80);
        assert!(tracking_lock_score(correction, 0.20) < 0.80);
        assert!(tracking_lock_score(correction, 0.98) < 0.80);
    }

    #[test]
    fn source_flow_score_separates_frame_slips_from_sustained_drift() {
        assert_eq!(
            source_flow_lock_score([0.36, 0.74, 0.78], [0.50, 2.09, 2.60]),
            1.0,
            "sparse long-baseline tails can be foreground crossings when lag-1 tails and every baseline p95 remain subpixel"
        );
        assert!(
            source_flow_lock_score([0.36, 0.74, 0.78], [3.50, 2.09, 2.60]) < 0.50,
            "lag-1 tail errors must still reject a transient tracking slip"
        );
        assert!(
            source_flow_lock_score([0.36, 1.80, 3.50], [0.50, 2.09, 4.20]) < 0.50,
            "long-baseline p95 errors must reject sustained drift"
        );
        assert_eq!(
            source_flow_lock_score([f64::INFINITY, 0.5, 0.5], [0.5; 3]),
            0.0
        );
    }

    #[test]
    fn temporal_stability_distinguishes_observed_acceleration_from_tracker_jitter() {
        assert_eq!(temporal_stability_score(0.80, 1.0, true), 1.0);
        assert_eq!(temporal_stability_score(0.80, 1.0, false), 0.80);
        assert_eq!(temporal_stability_score(0.99, 0.40, true), 0.99);
    }

    #[test]
    fn fully_reviewed_title_evidence_is_authoritative_over_occluded_source_flow() {
        let motion = (0..3)
            .map(|frame| {
                let mut sample = motion(frame, frame as f64);
                sample.measurement_source = "reviewed-dense-quad".into();
                sample
            })
            .collect::<Vec<_>>();
        assert!(is_fully_reviewed_trajectory(
            "reviewed-dense-quad-track-3-frames",
            3,
            0,
            3,
            &motion,
        ));

        let (score, basis) = authoritative_tracking_lock(false, true, 0.99, true, 0.01, true);
        assert_eq!(score, 0.99);
        assert_eq!(
            basis,
            "fully-reviewed-dense-trajectory-and-direct-rendered-title-plane-lock"
        );
    }

    #[test]
    fn incomplete_review_cannot_override_independent_source_flow() {
        let mut motion = (0..3)
            .map(|frame| {
                let mut sample = motion(frame, frame as f64);
                sample.measurement_source = "reviewed-dense-quad".into();
                sample
            })
            .collect::<Vec<_>>();
        motion[1].measurement_source = "tracker".into();
        assert!(!is_fully_reviewed_trajectory(
            "reviewed-dense-quad-track-3-frames",
            3,
            0,
            3,
            &motion,
        ));

        let (score, basis) = authoritative_tracking_lock(false, false, 1.0, true, 0.42, true);
        assert_eq!(score, 0.42);
        assert!(basis.contains("source-material-flow"));
    }

    #[test]
    fn structural_registration_recovers_affine_motion() {
        let mut template = Surface::new(96, 64);
        for y in 0..64 {
            for x in 0..96 {
                let value = ((x * 17 + y * 29 + x * y * 3) % 211 + 30) as u8;
                template.set_pixel(x, y, Rgba::new(value, value, value, 255));
            }
        }
        let mut current = Surface::new(96, 64);
        current
            .warp_blend(
                &template,
                Quad::new(
                    Point::new(3.0, 2.0),
                    Point::new(93.0, 4.0),
                    Point::new(95.0, 62.0),
                    Point::new(5.0, 60.0),
                ),
                1.0,
            )
            .unwrap();
        let mask = vec![255; 96 * 64];

        let registration = measure_structural_registration(&template, &current, &mask, 8).unwrap();

        assert!(registration.ecc.is_some());
        assert!(registration.after + 1.0 < registration.before);
        assert!(registration_correction_pixels(&registration, 96, 64) > 2.0);
    }
}
