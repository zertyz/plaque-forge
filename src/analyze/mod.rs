pub(crate) mod candidate;
pub(crate) mod extraction;
mod occlusion;
pub(crate) mod tracking;

use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use image::{GrayImage, ImageBuffer, Luma};

use crate::{
    analysis::{
        ANALYSIS_FORMAT, Analysis, AnalysisManifest, AnalysisStatus, INJECTED_SURFACE_FILE,
        InjectedSurfaceAsset, OCCLUDER_DIR, SegmentationConfig, SourceInfo, TRAJECTORY_FILE,
    },
    application::AnalyzeRequest,
    layers::{self, LayerInput},
    model::AnalysisConfidence,
    progress::ProgressReporter,
    scene::{
        DepthMode, SceneProvenance, SurfaceAppearance, SurfaceSpace, SurfaceTrajectory, find_scene,
        layer_artifact_provenance, provenance, resolve_relative, semantic_provenance,
    },
    video, workspace,
};

struct InjectedSurfaceInput {
    path: std::path::PathBuf,
    inset: [f64; 4],
}

struct AnalysisScenes {
    trajectory: Option<SurfaceTrajectory>,
    layers: Vec<LayerInput>,
    provenance: Option<SceneProvenance>,
    injected_surface: Option<InjectedSurfaceInput>,
    surface_space: SurfaceSpace,
    occlusion_mode: DepthMode,
}

impl AnalysisScenes {
    fn has_dense_locked_track(&self, frame_count: usize) -> bool {
        self.trajectory
            .as_ref()
            .is_some_and(|track| track.is_dense_locked(frame_count))
    }
}

pub fn run(
    mut args: AnalyzeRequest,
    commands: &dyn crate::infrastructure::CommandExecutor,
) -> Result<()> {
    if !args.source_is_text_free {
        bail!(
            "analysis requires an explicit --source-is-text-free assertion; Plaque Forge does not remove or inpaint existing titles"
        );
    }
    if args.force_ml && args.segmentation_worker.is_none() {
        bail!(
            "--force-ml requires --segmentation-worker; use the high-level analyze script for the configured ML runtime"
        );
    }
    if !(0.0..=1.0).contains(&args.minimum_analysis_confidence) {
        bail!("--minimum-analysis-confidence must be between 0 and 1");
    }
    let mut progress = ProgressReporter::new(args.progress, args.progress_interval_ms);
    progress.start(1, 7, "Probe and validate source", None);
    if !args.input.is_file() {
        bail!(
            "input video does not exist or is not a file: {}",
            args.input.display()
        );
    }
    let info = video::probe_with(commands, &args.ffprobe, &args.input)
        .with_context(|| format!("failed to probe input video {}", args.input.display()))?;
    info.ensure_supported_compositing_color()?;
    if !info.constant_frame_rate {
        bail!(
            "variable-frame-rate input is unsupported; transcode it to a constant frame rate before analysis"
        );
    }
    let source_sha256 = crate::digest::file_sha256(&args.input)
        .with_context(|| format!("failed to hash source {}", args.input.display()))?;
    let output = args
        .output
        .clone()
        .map(Ok)
        .unwrap_or_else(|| workspace::analysis_path(&args.input))?;
    let segmentation = segmentation_config(&args)?;
    let can_generate_prompted = args.segmentation_worker.is_some();
    let published_layer_root = output.join(crate::analysis::LAYERS_DIR);
    let mut scenes = resolve_scenes(
        &mut args,
        &info,
        &source_sha256,
        &published_layer_root,
        &published_layer_root,
        can_generate_prompted,
    )?;
    if output.exists() && args.if_needed && !args.force {
        if analysis_cache_is_current(
            &output,
            &source_sha256,
            scenes.provenance.as_ref(),
            segmentation.clone(),
        ) {
            println!("analysis cache is current: {}", output.display());
            return Ok(());
        }
        eprintln!("analysis cache is stale; rebuilding: {}", output.display());
        args.force = true;
    }
    if output.exists() && !args.force {
        bail!(
            "analysis output already exists: {}\nhelp: use --force to delete and replace it after a successful rebuild",
            output.display()
        );
    }
    if output.exists() {
        eprintln!(
            "replacing analysis after successful rebuild: {}",
            output.display()
        );
    }
    let staged = crate::staged_output::create(&output)?;
    let partial = staged.path().to_path_buf();
    let generated_layer_root = partial.join(".generated-layers");

    // Prompted masks are generated inside the /tmp-owned analysis transaction. A
    // worker failure therefore cannot leave a partial tree under assets/analysis.
    if let Some(worker) = args.segmentation_worker.as_deref() {
        eprintln!(
            "[ml] segmentation enabled: worker={}, backend={}, model={}, device={}",
            worker.display(),
            args.segmentation_backend,
            args.segmentation_model,
            args.segmentation_device
        );
        let generated = crate::segmentation::ensure_prompted_layers(
            crate::segmentation::PromptedLayersRequest {
                input: &args.input,
                explicit_scene: args.scene.as_deref(),
                surface_id: args.surface.as_deref(),
                worker,
                backend: &args.segmentation_backend,
                model: &args.segmentation_model,
                device: &args.segmentation_device,
                force: args.force_ml,
                ffprobe: &args.ffprobe,
                info: &info,
                source_sha256: &source_sha256,
                output_root: &generated_layer_root,
                reuse_root: output.is_dir().then_some(published_layer_root.as_path()),
                commands,
            },
        )
        .context("failed to materialize prompted scene layers")?;
        if generated > 0 {
            eprintln!("[ml] generated {generated} prompted scene layer artifact(s)");
            scenes = resolve_scenes(
                &mut args,
                &info,
                &source_sha256,
                &generated_layer_root,
                &published_layer_root,
                false,
            )?;
        } else {
            scenes = resolve_scenes(
                &mut args,
                &info,
                &source_sha256,
                &generated_layer_root,
                &published_layer_root,
                false,
            )?;
        }
    } else {
        eprintln!("[ml] segmentation disabled: no worker configured (high-level --no-ml mode)");
    }
    progress.finish(format!(
        "{}x{}, {:.3} fps, {} frames",
        info.width, info.height, info.fps, info.frames
    ));
    let diagnostics = match &args.diagnostics {
        Some(path) => path.clone(),
        None => partial.join("diagnostics"),
    };
    fs::create_dir_all(&diagnostics).with_context(|| {
        format!(
            "failed to create diagnostics directory {}",
            diagnostics.display()
        )
    })?;

    let result = (|| -> Result<Analysis> {
        progress.start(
            2,
            7,
            "Resolve writing surface",
            Some(args.candidate_samples),
        );
        let candidate = candidate::detect(&args, &info, &diagnostics)
            .context("writing-surface proposal failed")?;
        progress.finish(format!(
            "frame {}, confidence {:.3}, rect {:.0},{:.0},{:.0},{:.0}",
            candidate.frame_index,
            candidate.confidence,
            candidate.rect.x,
            candidate.rect.y,
            candidate.rect.width,
            candidate.rect.height
        ));

        // Authored source-pixel surface/foreground layers already exist before
        // automatic occlusion analysis. Apply them to the very first tracker pass
        // so background, birds, vines, spiders, and matte holes can never become
        // plaque reference points. The same directory is rebuilt later with the
        // automatic foreground union once that evidence is available.
        let combined_exclusions = partial.join("tracking-exclusions");
        let empty_exclusions = partial.join("no-automatic-exclusions");
        let has_initial_scene_exclusions = layers::build_tracking_exclusions(
            &scenes.layers,
            &empty_exclusions,
            &combined_exclusions,
            info.width,
            info.height,
            info.frames,
            None,
        )
        .context("failed to build initial scene tracking support")?;

        let mut track = if let Some(scene_track) = &scenes.trajectory
            && scene_track.is_dense_locked(info.frames)
        {
            tracking::load_dense_scene(
                &args,
                &info,
                candidate.rect,
                scene_track,
                &diagnostics,
                &mut progress,
            )
            .context("failed to load the reviewed dense plaque trajectory")?
        } else if scenes.surface_space == SurfaceSpace::ScreenCanvas {
            eprintln!("surface space: intentional screen canvas");
            tracking::screen_fixed(info.frames, candidate.frame_index, 1.0, "screen-canvas")
        } else {
            tracking::track(
                &args,
                &info,
                candidate.rect,
                candidate.frame_index,
                &diagnostics,
                &mut progress,
                has_initial_scene_exclusions.then_some(combined_exclusions.as_path()),
            )
            .context(
                "physical surface tracking failed; analysis will not freeze it to the screen",
            )?
        };

        // Surface space chooses the base coordinate system; sparse scene anchors
        // constrain that system after automatic estimation. A physical scene-plane
        // never becomes screen-fixed merely because evidence is difficult.
        if let Some(scene_track) = &scenes.trajectory
            && !scene_track.is_dense_locked(info.frames)
        {
            tracking::apply_motion_scene(&mut track, scene_track, candidate.rect)
                .context("failed to apply trajectory constraints")?;
        }

        let mut extraction = extraction::recover(
            &args.ffmpeg,
            &args.input,
            &info,
            candidate.rect,
            &mut track.samples,
            &partial,
            &diagnostics,
            args.extraction_samples,
            args.local_scene_radius,
            false,
            scenes.trajectory.as_ref(),
            track.reference_frame,
            args.tracking_inertia,
            track.loop_closed,
            &mut progress,
        )
        .context("canonical plaque/background structure analysis failed")?;
        let surface_intent = SurfaceIntentContext {
            args: &args,
            scenes: &scenes,
            tracking_rect: candidate.rect,
            width: candidate.canonical_width,
            height: candidate.canonical_height,
            output_root: &partial,
            diagnostics: &diagnostics,
        };
        let mut injected_surface_asset = apply_surface_intent(&surface_intent, &mut extraction)?;

        progress.start(7, 7, "Analyze foreground occlusion", Some(info.frames));
        let automatic_occlusion =
            !args.disable_occlusion && scenes.occlusion_mode == DepthMode::Automatic;
        let mut occlusion = if !automatic_occlusion {
            occlusion::OcclusionResult {
                has_occluder: false,
                confidence: 0.80,
                mean_coverage: 0.0,
            }
        } else {
            occlusion::extract(
                &args.ffmpeg,
                &args.input,
                &info,
                candidate.rect,
                &mut track.samples,
                &extraction,
                &partial,
                &diagnostics,
                args.occlusion_sensitivity,
                track.loop_closed,
                scenes.trajectory.as_ref(),
                &mut progress,
            )
            .context("foreground occlusion extraction failed")?
        };
        let authored_foreground = layers::has_authored_foreground(&scenes.layers);
        let mut automatic_ml_foreground = false;
        if automatic_occlusion && occlusion.has_occluder {
            if let Some(worker) = args.segmentation_worker.as_deref() {
                match crate::segmentation::refine_automatic_foreground(
                    crate::segmentation::AutomaticForegroundRequest {
                        input: &args.input,
                        worker,
                        backend: &args.segmentation_backend,
                        model: &args.segmentation_model,
                        device: &args.segmentation_device,
                        info: &info,
                        plaque: candidate.rect,
                        seed_masks: &partial.join(OCCLUDER_DIR),
                        analysis_root: &partial,
                        force: args.force_ml,
                        reuse_root: output.is_dir().then_some(output.as_path()),
                        commands,
                    },
                ) {
                    Ok(true) => {
                        automatic_ml_foreground = true;
                        occlusion.has_occluder = true;
                    }
                    Ok(false) => {}
                    Err(error) => {
                        // Automatic ML is a scene of the already-available Rust
                        // occlusion path. A model/runtime failure must not destroy an
                        // otherwise useful analysis; authored ML layers remain strict.
                        eprintln!(
                            "warning: automatic ML foreground scene failed; keeping Rust occlusion masks: {error:#}"
                        );
                    }
                }
            } else {
                eprintln!(
                    "[ml] automatic foreground opportunity detected, but no worker is configured"
                );
            }
        } else if automatic_occlusion {
            eprintln!(
                "[ml] automatic foreground skipped: Rust found no persistent foreground crossing"
            );
        }

        let automatic_exclusions = partial.join(OCCLUDER_DIR);
        let automatic_tracking_exclusions = if automatic_occlusion {
            &automatic_exclusions
        } else {
            &empty_exclusions
        };
        let has_scene_exclusions = layers::build_tracking_exclusions(
            &scenes.layers,
            automatic_tracking_exclusions,
            &combined_exclusions,
            info.width,
            info.height,
            info.frames,
            Some((&track.samples, candidate.rect)),
        )
        .context("failed to combine scene and automatic tracking exclusions")?;
        let tracking_exclusions = if has_scene_exclusions {
            &combined_exclusions
        } else if !automatic_occlusion && authored_foreground {
            &empty_exclusions
        } else {
            &automatic_exclusions
        };
        if automatic_occlusion
            && !scenes.has_dense_locked_track(info.frames)
            && !track.model_name.contains("screen-fixed")
            && (has_scene_exclusions || occlusion.has_occluder)
        {
            match tracking::retrack_masked(
                &args,
                &info,
                candidate.rect,
                track.reference_frame,
                &diagnostics,
                &mut progress,
                tracking_exclusions,
            ) {
                Ok(mut refined) => {
                    if let Some(scene_track) = &scenes.trajectory {
                        tracking::apply_motion_scene(&mut refined, scene_track, candidate.rect)?;
                    }
                    for (sample, previous) in refined.samples.iter_mut().zip(&track.samples) {
                        sample.occluder_coverage = previous.occluder_coverage;
                    }
                    track = tracking::refine_scene_with_masked_flow(
                        &args,
                        &info,
                        candidate.rect,
                        track,
                        refined,
                        tracking_exclusions,
                        &mut progress,
                    )?;
                }
                Err(error) => {
                    eprintln!(
                        "warning: masked tracking pass had less usable surface evidence; retaining its complete baseline before the final foreground-aware flow solve: {error:#}"
                    );
                    track = tracking::refine_baseline_with_foreground_flow(
                        &args,
                        &info,
                        candidate.rect,
                        track,
                        tracking_exclusions,
                        &mut progress,
                    )?;
                }
            }
            if let Some(scene_track) = &scenes.trajectory {
                tracking::reapply_locked_scenes(&mut track.samples, scene_track, candidate.rect)?;
            }
            extraction = extraction::recover(
                &args.ffmpeg,
                &args.input,
                &info,
                candidate.rect,
                &mut track.samples,
                &partial,
                &diagnostics,
                args.extraction_samples,
                args.local_scene_radius,
                false,
                scenes.trajectory.as_ref(),
                track.reference_frame,
                args.tracking_inertia,
                track.loop_closed,
                &mut progress,
            )
            .context("failed to rebuild the plaque/background model after masked tracking")?;
            injected_surface_asset = apply_surface_intent(&surface_intent, &mut extraction)?;
            progress.start(7, 7, "Rebuild foreground occlusion", Some(info.frames));
            occlusion = occlusion::extract(
                &args.ffmpeg,
                &args.input,
                &info,
                candidate.rect,
                &mut track.samples,
                &extraction,
                &partial,
                &diagnostics,
                args.occlusion_sensitivity,
                track.loop_closed,
                scenes.trajectory.as_ref(),
                &mut progress,
            )
            .context("failed to rebuild foreground masks after masked tracking")?;
            if automatic_ml_foreground {
                automatic_ml_foreground =
                    crate::segmentation::install_automatic_foreground_masks(&partial, info.frames)
                        .context("failed to restore ML foreground masks after masked retracking")?;
            }
        }
        if automatic_ml_foreground {
            occlusion = occlusion::summarize_installed_masks(
                &info,
                candidate.rect,
                &mut track.samples,
                &extraction,
                &partial,
                &diagnostics,
            )
            .context("failed to summarize the final installed foreground masks")?;
            if !occlusion.has_occluder {
                automatic_ml_foreground = false;
            }
        }
        if !automatic_occlusion && authored_foreground {
            occlusion.has_occluder = false;
            if automatic_exclusions.is_dir() {
                crate::staged_output::remove_child(&partial, &automatic_exclusions)?;
            }
        }
        if has_scene_exclusions {
            crate::staged_output::remove_child(&partial, &combined_exclusions)?;
        }
        if !automatic_occlusion && let Some(scene_track) = &scenes.trajectory {
            tracking::apply_visibility_scenes(&mut track.samples, scene_track)
                .context("failed to apply reviewed plaque visibility constraints")?;
        }
        progress.finish(format!(
            "occluder {}, confidence {:.3}",
            if occlusion.has_occluder {
                "detected"
            } else {
                "not detected"
            },
            occlusion.confidence
        ));

        let packed_layers = layers::package(
            &scenes.layers,
            &partial,
            candidate.canonical_width,
            candidate.canonical_height,
            info.width,
            info.height,
            candidate.rect,
            &track.samples,
            &mut extraction.content_mask,
        )
        .context("failed to import scene layer artifacts")?;
        if generated_layer_root.is_dir() {
            crate::staged_output::remove_child(&partial, &generated_layer_root)
                .context("failed to remove temporary prompted-layer sources")?;
        }

        let overall = geometric_mean(&[
            candidate.confidence,
            track.confidence,
            extraction.confidence,
            occlusion.confidence,
        ]);
        let declared_writable_region = args.writable_region_hint.is_some();
        let injected_surface = scenes.injected_surface.is_some();
        let structureless_surface = declared_writable_region || injected_surface;
        let observable_frames = track
            .samples
            .iter()
            .filter(|sample| {
                tracking::surface_visible_fraction(
                    candidate.rect,
                    sample.transform,
                    info.width,
                    info.height,
                ) >= 0.15
            })
            .count();
        let measured = track
            .samples
            .iter()
            .filter(|sample| {
                sample.measurement_valid
                    && tracking::surface_visible_fraction(
                        candidate.rect,
                        sample.transform,
                        info.width,
                        info.height,
                    ) >= 0.15
            })
            .collect::<Vec<_>>();
        let measurement_fraction = measured.len() as f64 / observable_frames.max(1) as f64;
        let median_spatial_coverage = tracking::median(
            measured
                .iter()
                .map(|sample| sample.spatial_coverage)
                .collect(),
        );
        let physical_tracking_gate = scenes.surface_space == SurfaceSpace::ScreenCanvas
            || (measurement_fraction >= 0.60 && median_spatial_coverage >= 0.42);
        let extraction_floor = if structureless_surface { 0.45 } else { 0.65 };
        let structure_gate = structureless_surface || extraction.structural_area >= 0.001;
        let component_gate_passed = candidate.confidence >= 0.60
            && extraction.confidence >= extraction_floor
            && structure_gate
            && occlusion.confidence >= 0.55
            && physical_tracking_gate;
        let analysis_gate_passed =
            overall >= args.minimum_analysis_confidence && component_gate_passed;
        let recovery = remedies(&AnalysisQuality {
            args: &args,
            rect: candidate.rect,
            candidate: candidate.confidence,
            motion: track.confidence,
            structure: extraction.confidence,
            structural_area: extraction.structural_area,
            occlusion: occlusion.confidence,
            structure_optional: structureless_surface,
        });

        fs::write(
            partial.join("analysis-summary.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "format": ANALYSIS_FORMAT,
                "source_contract": "text-free-writing-surface",
                "declared_writable_region": args.writable_region_hint.as_ref().map(|region| region.kind()),
                "surface_source": if injected_surface { "injected" } else { "source" },
                "candidate_screen_stationarity": candidate.screen_stationarity,
                "surface_space": format!("{:?}", scenes.surface_space),
                "candidate_confidence": candidate.confidence,
                "candidate_temporal_support": candidate.temporal_support,
                "motion_confidence": track.confidence,
                "candidate_edge_completeness": candidate.edge_completeness,
                "tracking_observable_frames": observable_frames,
                "tracking_measurement_fraction": measurement_fraction,
                "tracking_median_spatial_coverage": median_spatial_coverage,
                "structural_confidence": extraction.confidence,
                "content_cavity_area": extraction.cavity_area,
                "registration_area": extraction.registration_area,
                "structural_area": extraction.structural_area,
                "occlusion_confidence": occlusion.confidence,
                "has_occluder": occlusion.has_occluder,
                "occluder_mean_coverage": occlusion.mean_coverage,
                "automatic_ml_foreground": automatic_ml_foreground,
                "overall": overall,
                "minimum_analysis_confidence": args.minimum_analysis_confidence,
                "component_gate_passed": component_gate_passed,
                "analysis_gate_passed": analysis_gate_passed,
                "remedies": recovery
            }))?,
        )
        .with_context(|| format!("failed to write analysis summary in {}", partial.display()))?;
        fs::write(
            partial.join(TRAJECTORY_FILE),
            serde_json::to_vec_pretty(&track.samples)?,
        )
        .with_context(|| format!("failed to write diagnostic motion in {}", partial.display()))?;

        if !analysis_gate_passed {
            bail!(
                "analysis quality gate failed: writing surface {:.0},{:.0},{:.0},{:.0}; overall {overall:.3} (minimum {:.3}), components candidate {:.3}, tracking {:.3}, extraction {:.3} (structural area {:.3}%), occlusion {:.3}. {}",
                candidate.rect.x,
                candidate.rect.y,
                candidate.rect.width,
                candidate.rect.height,
                args.minimum_analysis_confidence,
                candidate.confidence,
                track.confidence,
                extraction.confidence,
                extraction.structural_area * 100.0,
                occlusion.confidence,
                recovery.join(". ")
            );
        }

        let manifest = AnalysisManifest {
            format: ANALYSIS_FORMAT.to_string(),
            status: AnalysisStatus::Complete,
            source_is_text_free: true,
            analyzer_build: crate::build_info::ANALYZER_CACHE_VERSION.to_string(),
            source: SourceInfo {
                path: crate::portable_path::relative_reference(
                    &output.join("manifest.toml"),
                    &args.input,
                )?,
                sha256: source_sha256.clone(),
                width: info.width,
                height: info.height,
                fps: info.fps,
                frames: info.frames,
                duration_seconds: info.duration_seconds,
            },
            reference_frame: track.reference_frame,
            canonical_width: candidate.canonical_width,
            canonical_height: candidate.canonical_height,
            source_plaque_rect: candidate.rect,
            surface_space: scenes.surface_space,
            trajectory_model: track.model_name,
            loop_closed: track.loop_closed,
            has_occluder: occlusion.has_occluder,
            occlusion_mode: scenes.occlusion_mode,
            segmentation,
            automatic_ml_foreground,
            injected_surface: injected_surface_asset,
            layers: packed_layers,
            scenes: scenes
                .provenance
                .as_ref()
                .map(|provenance| provenance.portable_for(&output.join("manifest.toml")))
                .transpose()?,
            analysis_gate_passed,
            confidence: AnalysisConfidence {
                plaque_detection: candidate.confidence,
                motion: track.confidence,
                extraction: extraction.confidence,
                occlusion: occlusion.confidence,
                overall,
            },
        };

        Analysis::create(&partial, manifest, track.samples)
    })();

    let pack = match result {
        Ok(pack) => pack,
        Err(error) => {
            let disposition = match crate::staged_output::retain_failure(&partial, &output) {
                Ok(Some(path)) => {
                    format!("compact diagnostics were retained in {}", path.display())
                }
                Ok(None) => "temporary work was cleaned up".to_string(),
                Err(retention_error) => {
                    eprintln!(
                        "warning: compact failure diagnostics could not be retained: {retention_error:#}"
                    );
                    "temporary work was cleaned up; diagnostic retention also failed".to_string()
                }
            };
            return Err(error.context(format!("analysis failed; {disposition}")));
        }
    };

    drop(pack);
    staged.commit(args.force)?;
    let pack = Analysis::open(&output)?;
    println!("analysis: {}", pack.root.display());
    println!(
        "overall analysis confidence: {:.3}",
        pack.manifest.confidence.overall
    );
    Ok(())
}

fn resolve_scenes(
    args: &mut AnalyzeRequest,
    info: &video::VideoInfo,
    source_sha256: &str,
    generated_layer_root: &Path,
    published_layer_root: &Path,
    allow_missing_prompted: bool,
) -> Result<AnalysisScenes> {
    let loaded = find_scene(&args.input, args.scene.as_deref())?;
    let mut identity = SceneProvenance::default();
    let mut selected_id = None;
    let mut referenced_track = None;
    let mut embedded_track = None;
    let mut layer_inputs = Vec::new();
    let mut injected_surface = None;
    let mut surface_space = SurfaceSpace::ScenePlane;
    let mut occlusion_mode = DepthMode::Automatic;

    if let Some(loaded) = &loaded {
        let declared_source = resolve_relative(&loaded.path, &loaded.document.source);
        same_file(&declared_source, &args.input).with_context(|| {
            format!(
                "scene {} declares source {}, not input {}",
                loaded.path.display(),
                declared_source.display(),
                args.input.display()
            )
        })?;
        let selected = loaded.document.select_surface(args.surface.as_deref())?;
        occlusion_mode = selected.depth;
        surface_space = selected.space;
        selected_id = Some(selected.id.clone());
        identity.manifest = Some(semantic_provenance(&loaded.path, &loaded.document)?);
        identity.surface_id = Some(selected.id.clone());
        match &selected.appearance {
            SurfaceAppearance::Image { image, inset } => {
                let path = resolve_relative(&loaded.path, image);
                if !path.is_file() {
                    bail!("injected plaque image does not exist: {}", path.display());
                }
                image::open(&path).with_context(|| {
                    format!("failed to decode injected plaque image {}", path.display())
                })?;
                identity.surface_asset = Some(provenance(&path)?);
                injected_surface = Some(InjectedSurfaceInput {
                    path,
                    inset: *inset,
                });
            }
            SurfaceAppearance::Observed => {}
        }

        if args.writable_region_hint.is_none()
            && let Some(region) = &selected.writable_region
        {
            args.writable_region_hint = Some(region.resolve(&loaded.path));
        }
        if args.surface_hint.is_none()
            && let Some(bounds) = selected.tracking_bounds()
        {
            args.surface_hint = Some(bounds);
            args.surface_frame = selected.reference_frame;
        }
        if let Some(frame) = selected.reference_frame
            && frame >= info.frames
        {
            bail!(
                "plaque {:?} reference_frame {} is outside the {}-frame source",
                selected.id,
                frame,
                info.frames
            );
        }
        for prompt in &selected.prompts {
            if prompt.frame >= info.frames {
                bail!(
                    "plaque {:?} prompt frame {} is outside the {}-frame source",
                    selected.id,
                    prompt.frame,
                    info.frames
                );
            }
        }
        for layer in loaded
            .document
            .layers
            .iter()
            .filter(|layer| layer.surface == selected.id)
        {
            for prompt in &layer.prompts {
                if prompt.frame >= info.frames {
                    bail!(
                        "layer {:?} prompt frame {} is outside the {}-frame source",
                        layer.id,
                        prompt.frame,
                        info.frames
                    );
                }
            }
            let (artifact_path, published_path) = if let Some(artifact) = &layer.artifact {
                let path = resolve_relative(&loaded.path, artifact);
                (path.clone(), path)
            } else if !layer.prompts.is_empty() {
                (
                    generated_layer_root.join(&layer.id).join("artifact.toml"),
                    published_layer_root.join(&layer.id).join("artifact.toml"),
                )
            } else {
                continue;
            };
            if !artifact_path.is_file() {
                if allow_missing_prompted && !layer.prompts.is_empty() {
                    continue;
                }
                bail!(
                    "layer {:?} artifact is missing at {}; run the high-level analyze script after ./scripts/setup_segmentation.sh",
                    layer.id,
                    artifact_path.display()
                );
            }
            let artifact = match crate::scene::LayerArtifact::load(&artifact_path) {
                Ok(artifact) => artifact,
                Err(error) if allow_missing_prompted && !layer.prompts.is_empty() => {
                    eprintln!(
                        "prompted layer {:?} has an incompatible cached artifact at {}; regenerating it: {error:#}",
                        layer.id,
                        artifact_path.display()
                    );
                    continue;
                }
                Err(error) => return Err(error),
            };
            let mut artifact_identity = match layer_artifact_provenance(&artifact_path, &artifact) {
                Ok(identity) => identity,
                Err(error) if allow_missing_prompted && !layer.prompts.is_empty() => {
                    eprintln!(
                        "prompted layer {:?} has incomplete cached provenance at {}; regenerating it: {error:#}",
                        layer.id,
                        artifact_path.display()
                    );
                    continue;
                }
                Err(error) => return Err(error),
            };
            artifact_identity.path = published_path;
            identity.layer_artifacts.push(artifact_identity);
            layer_inputs.push(LayerInput {
                scene: layer.clone(),
                artifact_path,
                artifact,
            });
        }
        referenced_track = selected
            .trajectory
            .as_ref()
            .map(|path| resolve_relative(&loaded.path, path));
        embedded_track = selected.sparse_trajectory(info.width, info.height, source_sha256)?;
        if let Some(track) = &embedded_track {
            identity.locked_keyframes = track.locked_keyframes();
            identity.guide_keyframes = track.guide_keyframes();
        }
    } else if let Some(id) = &args.surface {
        bail!("--plaque {id:?} requires a scene manifest");
    }

    let (trajectory, trajectory_path) = if let Some(track) = embedded_track {
        (Some(track), None)
    } else if let Some(path) = referenced_track {
        (Some(SurfaceTrajectory::load(&path)?), Some(path))
    } else {
        (None, None)
    };
    let trajectory = if let Some(track) = trajectory {
        if let Some(selected_id) = &selected_id
            && track.surface != *selected_id
        {
            bail!(
                "trajectory describes surface {:?}, but scene selected {:?}",
                track.surface,
                selected_id
            );
        }
        if let Some(expected) = &track.source_sha256
            && !expected.eq_ignore_ascii_case(source_sha256)
        {
            bail!(
                "trajectory source hash does not match {}; export or review it against this source",
                args.input.display()
            );
        }
        for keyframe in &track.keyframes {
            if keyframe.frame >= info.frames {
                bail!(
                    "motion keyframe {} is outside the {}-frame source",
                    keyframe.frame,
                    info.frames
                );
            }
        }
        if args.surface_hint.is_none() {
            let first = track.sorted_keyframes()[0];
            args.surface_hint = Some(quad_bounds(first.quad));
            args.surface_frame = Some(first.frame);
        }
        identity.surface_id = Some(track.surface.clone());
        if let Some(path) = trajectory_path {
            identity.trajectory = Some(semantic_provenance(&path, &track)?);
        }
        identity.locked_keyframes = track.locked_keyframes();
        identity.guide_keyframes = track.guide_keyframes();
        Some(track)
    } else {
        None
    };

    if let Some([x, y, width, height]) = args.surface_hint {
        if ![x, y, width, height].iter().all(|value| value.is_finite())
            || width <= 0.0
            || height <= 0.0
        {
            bail!("plaque bounds must be finite with positive width and height");
        }
        let frame = args.surface_frame.unwrap_or(0);
        if frame >= info.frames {
            bail!(
                "plaque bounds reference frame {} is outside the {}-frame source",
                frame,
                info.frames
            );
        }
    }

    let provenance = if identity == SceneProvenance::default() {
        None
    } else {
        Some(identity)
    };
    Ok(AnalysisScenes {
        trajectory,
        layers: layer_inputs,
        provenance,
        injected_surface,
        surface_space,
        occlusion_mode,
    })
}

fn analysis_cache_is_current(
    output: &Path,
    source_sha256: &str,
    scenes: Option<&SceneProvenance>,
    segmentation: Option<SegmentationConfig>,
) -> bool {
    let Ok(pack) = Analysis::open(output) else {
        return false;
    };
    if pack.manifest.analyzer_build != crate::build_info::ANALYZER_CACHE_VERSION
        || !pack
            .manifest
            .source
            .sha256
            .eq_ignore_ascii_case(source_sha256)
    {
        return false;
    }
    if pack.manifest.segmentation != segmentation {
        return false;
    }
    match (pack.manifest.scenes.as_ref(), scenes) {
        (None, None) => true,
        (Some(cached), Some(current)) => cached.content_matches(current),
        _ => false,
    }
}

fn segmentation_config(args: &AnalyzeRequest) -> Result<Option<SegmentationConfig>> {
    args.segmentation_worker
        .as_ref()
        .map(|worker| {
            Ok(SegmentationConfig {
                backend: args.segmentation_backend.clone(),
                model: args.segmentation_model.clone(),
                device: args.segmentation_device.clone(),
                worker_sha256: crate::segmentation::worker_sha256(worker)?,
                runtime_sha256: crate::segmentation::runtime_sha256()?,
            })
        })
        .transpose()
}

struct SurfaceIntentContext<'a> {
    args: &'a AnalyzeRequest,
    scenes: &'a AnalysisScenes,
    tracking_rect: crate::model::RectF,
    width: u32,
    height: u32,
    output_root: &'a Path,
    diagnostics: &'a Path,
}

fn apply_surface_intent(
    context: &SurfaceIntentContext<'_>,
    extraction: &mut extraction::ExtractionResult,
) -> Result<Option<InjectedSurfaceAsset>> {
    let SurfaceIntentContext {
        args,
        scenes,
        tracking_rect,
        width,
        height,
        output_root,
        diagnostics,
    } = *context;
    let tracking_bounds = [
        tracking_rect.x,
        tracking_rect.y,
        tracking_rect.width,
        tracking_rect.height,
    ];

    let Some(injected) = scenes.injected_surface.as_ref() else {
        if let Some(region) = args.writable_region_hint.as_ref() {
            apply_declared_writable_region(
                region,
                tracking_bounds,
                width,
                height,
                output_root,
                diagnostics,
                extraction,
            )?;
        }
        return Ok(None);
    };

    let source_image = image::open(&injected.path)
        .with_context(|| format!("failed to load injected plaque {}", injected.path.display()))?
        .to_rgba8();
    let canonical = if source_image.width() == width && source_image.height() == height {
        source_image
    } else {
        image::imageops::resize(
            &source_image,
            width,
            height,
            image::imageops::FilterType::Lanczos3,
        )
    };
    let canonical_path = output_root.join(INJECTED_SURFACE_FILE);
    canonical.save(&canonical_path).with_context(|| {
        format!(
            "failed to save injected plaque {}",
            canonical_path.display()
        )
    })?;

    // Plaque appearance and writability are intentionally orthogonal. In particular,
    // a glass/holographic plaque may have low alpha throughout its writing cavity:
    // multiplying by artwork alpha would make valid title pixels look clipped and
    // would incorrectly attenuate the title a second time. The explicit region or
    // inset owns layout geometry; the PNG alpha is used only when compositing the
    // injected plaque itself.
    let writable = if let Some(region) = args.writable_region_hint.as_ref() {
        region.canonical_mask_in(tracking_bounds, width, height)?
    } else {
        let [left, top, right, bottom] = injected.inset;
        let x0 = (left * width as f64).round().clamp(0.0, width as f64) as u32;
        let y0 = (top * height as f64).round().clamp(0.0, height as f64) as u32;
        let x1 = ((1.0 - right) * width as f64)
            .round()
            .clamp(0.0, width as f64) as u32;
        let y1 = ((1.0 - bottom) * height as f64)
            .round()
            .clamp(0.0, height as f64) as u32;
        let mut mask = vec![0_u8; width as usize * height as usize];
        for y in y0.min(height)..y1.min(height) {
            for x in x0.min(width)..x1.min(width) {
                mask[(y * width + x) as usize] = 255;
            }
        }
        mask
    };
    let area =
        writable.iter().filter(|&&value| value > 127).count() as f64 / writable.len().max(1) as f64;
    if area < 0.01 {
        bail!(
            "injected plaque writable region covers only {:.3}% of its canonical surface",
            area * 100.0
        );
    }
    extraction.content_mask = writable.clone();
    extraction.cavity_area = area;
    save_luma_mask(
        width,
        height,
        &writable,
        &output_root.join("content-mask.png"),
    )?;
    save_luma_mask(
        width,
        height,
        &writable,
        &diagnostics.join("injected-writable-mask.png"),
    )?;

    Ok(Some(InjectedSurfaceAsset {
        path: crate::portable_path::PortablePath::bundle(INJECTED_SURFACE_FILE)?,
        source_sha256: crate::digest::file_sha256(&injected.path)?,
    }))
}

fn apply_declared_writable_region(
    region: &crate::writable_region::ResolvedWritableRegion,
    tracking_bounds: [f64; 4],
    width: u32,
    height: u32,
    output_root: &Path,
    diagnostics: &Path,
    extraction: &mut extraction::ExtractionResult,
) -> Result<()> {
    let mask = region.canonical_mask_in(tracking_bounds, width, height)?;
    let area = mask.iter().filter(|&&value| value > 127).count() as f64 / mask.len().max(1) as f64;
    if area < 0.01 {
        bail!(
            "declared {} writable region covers only {:.3}% of its canonical bounds",
            region.kind(),
            area * 100.0
        );
    }
    extraction.content_mask = mask.clone();
    extraction.cavity_area = area;
    save_luma_mask(width, height, &mask, &output_root.join("content-mask.png"))?;
    save_luma_mask(
        width,
        height,
        &mask,
        &diagnostics.join("declared-writable-mask.png"),
    )?;
    Ok(())
}

fn save_luma_mask(width: u32, height: u32, mask: &[u8], path: &Path) -> Result<()> {
    let image: GrayImage = ImageBuffer::<Luma<u8>, _>::from_raw(width, height, mask.to_vec())
        .context("invalid writable-region mask dimensions")?;
    image
        .save(path)
        .with_context(|| format!("failed to save writable-region mask {}", path.display()))
}

fn same_file(a: &Path, b: &Path) -> Result<()> {
    let a = a
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", a.display()))?;
    let b = b
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", b.display()))?;
    if a != b {
        bail!("resolved paths differ: {} != {}", a.display(), b.display());
    }
    Ok(())
}

fn quad_bounds(quad: [[f64; 2]; 4]) -> [f64; 4] {
    let min_x = quad
        .iter()
        .map(|point| point[0])
        .fold(f64::INFINITY, f64::min);
    let min_y = quad
        .iter()
        .map(|point| point[1])
        .fold(f64::INFINITY, f64::min);
    let max_x = quad
        .iter()
        .map(|point| point[0])
        .fold(f64::NEG_INFINITY, f64::max);
    let max_y = quad
        .iter()
        .map(|point| point[1])
        .fold(f64::NEG_INFINITY, f64::max);
    [min_x, min_y, max_x - min_x, max_y - min_y]
}

fn geometric_mean(values: &[f64]) -> f64 {
    let product = values
        .iter()
        .map(|value| value.clamp(1.0e-9, 1.0))
        .product::<f64>();
    product.powf(1.0 / values.len() as f64)
}

struct AnalysisQuality<'a> {
    args: &'a AnalyzeRequest,
    rect: crate::model::RectF,
    candidate: f64,
    motion: f64,
    structure: f64,
    structural_area: f64,
    occlusion: f64,
    structure_optional: bool,
}

fn remedies(quality: &AnalysisQuality<'_>) -> Vec<String> {
    let AnalysisQuality {
        args,
        rect,
        candidate,
        motion,
        structure,
        structural_area,
        occlusion,
        structure_optional,
    } = *quality;
    let mut output = Vec::new();
    if structural_area < 0.001 && !structure_optional && args.writable_region_hint.is_none() {
        output.push(if args.surface_hint.is_some() {
            format!(
                "the scene selected {:.0},{:.0},{:.0},{:.0}, which contains {:.3}% stable structure; correct it to the full plaque bounds",
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                structural_area * 100.0
            )
        } else {
            format!(
                "automatic selection chose {:.0},{:.0},{:.0},{:.0}, which contains {:.3}% stable structure; inspect candidate.png and add the smallest writable-region scene needed to identify the intended surface",
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                structural_area * 100.0
            )
        });
        return output;
    }
    if candidate < 0.75 {
        output.push(format!(
            "automatic detection selected {:.0},{:.0},{:.0},{:.0} with confidence {candidate:.3}; inspect candidate.png and add a writable-region scene if the selected enclosure is wrong",
            rect.x, rect.y, rect.width, rect.height
        ));
    }
    if motion < 0.70 {
        output.push(format!(
            "automatic motion confidence is {motion:.3}; export the motion and lock only incorrect frames before reanalysis"
        ));
    }
    if structure < 0.75 && !structure_optional {
        output.push(format!(
            "canonical extraction confidence is {structure:.3}; inspect canonical-reference.png and temporal-mad.png"
        ));
    }
    if occlusion < 0.80 {
        output.push(format!(
            "occlusion confidence is {occlusion:.3}; inspect occlusion-summary.json and add a foreground scene when automatic separation is wrong"
        ));
    }
    output
}
