//! Boundary between the Rust application and optional ML segmentation workers.
//!
//! Rust owns validation, staging, provenance, and the versioned request/result protocol.
//! A worker is replaceable and is not trusted to mutate project data outside its output.

use std::{
    cmp::Reverse,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use image::{ImageBuffer, Luma};
use serde::{Deserialize, Serialize};

use crate::{
    analysis::{AUTHORED_OCCLUDER_WORK_DIR, AUTOMATIC_MATERIAL_WORK_DIR, OCCLUDER_DIR},
    cli::SegmentArgs,
    model::RectF,
    scene::{
        LayerArtifact, LayerArtifactKind, LayerCoordinates, LayerMatteMode, LayerRole,
        LayerSubject, Scene, SceneLayer, SegmentationPrompt, SpatialCoordinates, find_scene,
        resolve_relative,
    },
    segmentation_strategy::{
        self, AcceptancePolicy, PlanningInput, SegmentationPlan, SegmentationPrecision,
        SegmentationProfile, SegmentationStrategy, SemanticBackend,
    },
    stats::percentile_u16,
    video::{self, VideoInfo},
    workspace,
};

const WORKER_REQUEST_FORMAT: &str = "plaque-forge.segmentation-request/2";
const WORKER_RESULT_FORMAT: &str = "plaque-forge.segmentation-result/2";
const TEMP_LAYER_CACHE_MAX_AGE: Duration = Duration::from_secs(14 * 24 * 60 * 60);
const TEMP_LAYER_CACHE_MAX_ENTRIES: usize = 32;

fn run_worker_process(
    executor: &dyn crate::infrastructure::CommandExecutor,
    worker: &Path,
    request: &Path,
    output: &Path,
    label: &str,
) -> Result<()> {
    let args = vec![
        OsString::from("--request"),
        request.as_os_str().to_os_string(),
        OsString::from("--output"),
        output.as_os_str().to_os_string(),
    ];
    let status = executor
        .status(worker, &args)
        .with_context(|| format!("failed to start {label} worker {}", worker.display()))?;
    if !status.success {
        let code = status
            .code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_string());
        bail!(
            "{label} worker {} exited unsuccessfully (code={})",
            worker.display(),
            code
        );
    }
    eprintln!("[ml] {label} worker completed successfully");
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
struct WorkerRequest {
    format: String,
    backend: String,
    model: String,
    device: String,
    plan: SegmentationPlan,
    plan_sha256: String,
    prompt_sha256: String,
    worker_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_sha256: Option<String>,
    request_sha256: String,
    source: WorkerSource,
    plaque: WorkerPlaque,
    layer: WorkerLayer,
}

#[derive(Debug, Clone, Serialize)]
struct WorkerSource {
    path: std::path::PathBuf,
    sha256: String,
    width: u32,
    height: u32,
    fps: f64,
    frames: usize,
}

#[derive(Debug, Clone, Serialize)]
struct WorkerPlaque {
    id: String,
    reference_frame: Option<usize>,
    bounds: Option<[f64; 4]>,
    trajectory: Option<std::path::PathBuf>,
    trajectory_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct WorkerLayer {
    id: String,
    role: LayerRole,
    affects_layout: bool,
    matte_mode: LayerMatteMode,
    subject: LayerSubject,
    active_frames: Option<[usize; 2]>,
    prompts: Vec<crate::scene::SegmentationPrompt>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    seed_masks: Vec<WorkerSeedMask>,
}

#[derive(Debug, Clone, Serialize)]
struct WorkerSeedMask {
    frame: usize,
    path: PathBuf,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerResult {
    format: String,
    backend: String,
    model: String,
    version: String,
    plan_sha256: String,
    precision: SegmentationPrecision,
    frames: usize,
    mean_confidence: f64,
    minimum_confidence: f64,
    request_sha256: String,
    source_sha256: String,
    prompt_sha256: String,
    worker_sha256: String,
    runtime_sha256: Option<String>,
    nonempty_frames: usize,
    mean_coverage: f64,
    maximum_coverage: f64,
    soft_edge_pixels: u64,
    execution: Vec<WorkerStageResult>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerStageResult {
    stage: String,
    device: String,
    precision: String,
    seconds: f64,
    cache_hit: bool,
    #[serde(default)]
    process_peak_rss_mib: Option<f64>,
    #[serde(default)]
    accelerator_peak_mib: Option<f64>,
    #[serde(default)]
    note: Option<String>,
}

fn seal_request(mut request: WorkerRequest) -> Result<WorkerRequest> {
    // The semantic request identity intentionally excludes workstation-local paths.
    // Their content identities are already represented by source/motion hashes.
    let mut value = serde_json::to_value(&request)?;
    let object = value
        .as_object_mut()
        .context("worker request did not serialize as an object")?;
    object.remove("request_sha256");
    if let Some(source) = object
        .get_mut("source")
        .and_then(|value| value.as_object_mut())
    {
        source.remove("path");
    }
    if let Some(plaque) = object
        .get_mut("plaque")
        .and_then(|value| value.as_object_mut())
    {
        plaque.remove("trajectory");
    }
    strip_seed_paths(object.get_mut("layer"));
    request.request_sha256 = crate::digest::bytes_sha256(&serde_json::to_vec(&value)?);
    Ok(request)
}

fn prompt_sha256(layer: &WorkerLayer) -> Result<String> {
    let mut value = serde_json::to_value(layer)?;
    strip_seed_paths(Some(&mut value));
    Ok(crate::digest::bytes_sha256(&serde_json::to_vec(&value)?))
}

fn plan_sha256(plan: &SegmentationPlan) -> Result<String> {
    Ok(crate::digest::bytes_sha256(&serde_json::to_vec(plan)?))
}

fn resolve_plan(
    layer: &SceneLayer,
    backend: &str,
    model: &str,
    profile: &str,
    precision: &str,
) -> Result<SegmentationPlan> {
    segmentation_strategy::plan(PlanningInput {
        profile: SegmentationProfile::parse(profile)?,
        precision_override: SegmentationPrecision::parse(precision)?,
        backend_override: backend,
        model_override: model,
        role: layer.role,
        matte_mode: layer.matte.mode,
        subject: layer.subject,
        prompts: &layer.prompts,
    })
}

fn resolve_strategy(
    layer: &SceneLayer,
    backend: &str,
    model: &str,
    profile: &str,
    precision: &str,
) -> Result<SegmentationStrategy> {
    segmentation_strategy::strategy(PlanningInput {
        profile: SegmentationProfile::parse(profile)?,
        precision_override: SegmentationPrecision::parse(precision)?,
        backend_override: backend,
        model_override: model,
        role: layer.role,
        matte_mode: layer.matte.mode,
        subject: layer.subject,
        prompts: &layer.prompts,
    })
}

fn materialize_candidate_plan(
    layer: &SceneLayer,
    candidate: &SegmentationPlan,
) -> Result<SegmentationPlan> {
    // Re-enter the normal explicit planner so the exact plan executed by the lower-level
    // `segment` workflow is also the exact plan represented by its provenance hash.
    resolve_plan(
        layer,
        candidate.backend_label(),
        &candidate.semantic_model,
        candidate.profile.label(),
        candidate.precision.label(),
    )
}

fn runtime_sha256_for_plan(plan: &SegmentationPlan) -> Result<Option<String>> {
    if plan.semantic_backend == SemanticBackend::Sam31 {
        runtime_sha256_for_backend("sam3.1")
    } else {
        runtime_sha256()
    }
}

pub(crate) fn runtime_sha256_for_backend(backend: &str) -> Result<Option<String>> {
    let primary = runtime_sha256()?;
    if !matches!(backend, "sam3.1" | "sam31" | "sam3.1-vitmatte") {
        return Ok(primary);
    }
    let sam31_manifest = std::env::var_os("PLAQUE_FORGE_SAM31_RUNTIME_MANIFEST")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("plaque-forge-sam31/runtime-manifest.json"));
    if !sam31_manifest.is_file() {
        bail!(
            "SAM 3.1 plan requires its isolated CUDA runtime; run ./scripts/setup_sam31.sh first (expected {})",
            sam31_manifest.display()
        );
    }
    let sam31 = crate::digest::file_sha256(&sam31_manifest)?;
    Ok(Some(crate::digest::bytes_sha256(&serde_json::to_vec(
        &serde_json::json!({
            "segmentation_runtime": primary,
            "sam31_runtime": sam31,
        }),
    )?)))
}

fn strip_seed_paths(layer: Option<&mut serde_json::Value>) {
    let Some(seeds) = layer
        .and_then(|value| value.get_mut("seed_masks"))
        .and_then(|value| value.as_array_mut())
    else {
        return;
    };
    for seed in seeds {
        if let Some(seed) = seed.as_object_mut() {
            seed.remove("path");
        }
    }
}

pub(crate) fn worker_sha256(worker: &Path) -> Result<String> {
    let mut identities = Vec::new();
    for path in [
        worker.to_path_buf(),
        worker.with_file_name("segmentation_worker.py"),
        worker.with_file_name("segmentation_runtime.py"),
        worker.with_file_name("segmentation_service.py"),
        worker.with_file_name("segmentation-requirements.txt"),
    ] {
        if path.is_file() {
            identities.push((
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                crate::digest::file_sha256(&path)?,
            ));
        }
    }
    if identities.is_empty() {
        bail!("segmentation worker identity has no readable files");
    }
    identities.sort();
    Ok(crate::digest::bytes_sha256(&serde_json::to_vec(
        &identities,
    )?))
}

pub(crate) fn runtime_sha256() -> Result<Option<String>> {
    let path = std::env::var_os("PLAQUE_FORGE_RUNTIME_MANIFEST")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("plaque-forge-python/runtime-manifest.json"));
    path.is_file()
        .then(|| crate::digest::file_sha256(&path))
        .transpose()
}

fn worker_layer(layer: &crate::scene::SceneLayer, info: &VideoInfo) -> Result<WorkerLayer> {
    Ok(WorkerLayer {
        id: layer.id.clone(),
        role: layer.role,
        affects_layout: layer.affects_layout,
        matte_mode: layer.matte.mode,
        subject: layer.subject,
        active_frames: layer.active_frames,
        prompts: layer
            .prompts
            .iter()
            .map(|prompt| prompt.source_pixels(info.width, info.height))
            .collect::<Result<Vec<_>>>()?,
        seed_masks: Vec::new(),
    })
}

struct LayerCacheIdentity<'a> {
    backend: &'a str,
    model: &'a str,
    device: &'a str,
    source_sha256: &'a str,
    prompt_sha256: &'a str,
    worker_sha256: &'a str,
    runtime_sha256: Option<&'a str>,
    plan_sha256: &'a str,
}

fn temporary_layer_cache(identity: LayerCacheIdentity<'_>) -> PathBuf {
    let key = crate::digest::bytes_sha256(
        &serde_json::to_vec(&serde_json::json!({
            "backend": identity.backend,
            "model": identity.model,
            "device": identity.device,
            "source_sha256": identity.source_sha256,
            "prompt_sha256": identity.prompt_sha256,
            "worker_sha256": identity.worker_sha256,
            "runtime_sha256": identity.runtime_sha256,
            "plan_sha256": identity.plan_sha256,
        }))
        .expect("cache identity is serializable"),
    );
    std::env::temp_dir()
        .join("plaque-forge")
        .join("ml-layer-cache")
        .join(key)
}

fn layer_cache_is_current(path: &Path, expected: LayerCacheIdentity<'_>) -> bool {
    let Ok(artifact) = LayerArtifact::load(path) else {
        return false;
    };
    if artifact.format != crate::scene::LAYER_ARTIFACT_FORMAT
        || artifact
            .referenced_paths(path)
            .iter()
            .any(|asset| !asset.is_file())
    {
        return false;
    }
    let Some(generator) = artifact.generator else {
        return false;
    };
    generator.backend == expected.backend
        && generator.model == expected.model
        && generator.requested_device.as_deref() == Some(expected.device)
        && generator.source_sha256.as_deref() == Some(expected.source_sha256)
        && generator.prompt_sha256.as_deref() == Some(expected.prompt_sha256)
        && generator.worker_sha256.as_deref() == Some(expected.worker_sha256)
        && generator.runtime_sha256.as_deref() == expected.runtime_sha256
        && generator.plan_sha256.as_deref() == Some(expected.plan_sha256)
}

pub(crate) fn prompted_artifact_matches_source_and_prompt(
    artifact: &LayerArtifact,
    layer: &SceneLayer,
    info: &VideoInfo,
    source_sha256: &str,
) -> Result<bool> {
    artifact.validate_generated_provenance()?;
    let generator = artifact
        .generator
        .as_ref()
        .context("generated layer artifact is missing generator provenance")?;
    let prompt_sha256 = prompt_sha256(&worker_layer(layer, info)?)?;
    Ok(generator.source_sha256.as_deref() == Some(source_sha256)
        && generator.prompt_sha256.as_deref() == Some(prompt_sha256.as_str()))
}

pub fn run(args: SegmentArgs) -> Result<()> {
    run_with_executor(args, &crate::infrastructure::OS_COMMAND_EXECUTOR)
}

fn run_with_executor(
    args: SegmentArgs,
    commands: &dyn crate::infrastructure::CommandExecutor,
) -> Result<()> {
    if !args.input.is_file() {
        bail!("input video does not exist: {}", args.input.display());
    }
    if !args.worker.is_file() {
        bail!(
            "segmentation worker does not exist: {}",
            args.worker.display()
        );
    }
    let scene_path = args
        .scene
        .clone()
        .map(Ok)
        .unwrap_or_else(|| workspace::scene_path(&args.input))?;
    let scene = Scene::load(&scene_path)?;
    let declared_source = resolve_relative(&scene_path, &scene.source);
    ensure_same_file(&declared_source, &args.input)?;
    let plaque = scene.select_surface(args.surface.as_deref())?;
    let layer = scene
        .layers
        .iter()
        .find(|layer| layer.id == args.layer && layer.surface == plaque.id)
        .with_context(|| {
            format!(
                "scene does not declare layer {:?} for plaque {:?}",
                args.layer, plaque.id
            )
        })?;
    if layer.prompts.is_empty() {
        bail!("layer {:?} has no segmentation prompts", layer.id);
    }

    let info = video::probe_with(commands, &args.ffprobe, &args.input)?;
    info.ensure_supported_compositing_color()?;
    for prompt in &layer.prompts {
        if prompt.frame >= info.frames {
            bail!(
                "layer {:?} prompt frame {} exceeds the {}-frame source",
                layer.id,
                prompt.frame,
                info.frames
            );
        }
    }
    if let Some([_, last]) = layer.active_frames
        && last >= info.frames
    {
        bail!(
            "layer {:?} active_frames exceed the {}-frame source",
            layer.id,
            info.frames
        );
    }
    let output = args.output.clone().unwrap_or_else(|| {
        layer
            .artifact
            .as_ref()
            .map(|path| resolve_relative(&scene_path, path))
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| workspace::layer_path(&scene_path, &layer.id))
    });
    if output.exists() && !args.force {
        bail!(
            "segmentation output already exists: {}\nhelp: use --force to delete and replace it after a successful run",
            output.display()
        );
    }
    if output.exists() {
        eprintln!(
            "replacing segmentation after successful run: {}",
            output.display()
        );
    }
    let staged = crate::staged_output::create(&output)?;
    let partial = staged.path().to_path_buf();

    let source_sha256 = crate::digest::file_sha256(&args.input)?;
    let worker_layer = worker_layer(layer, &info)?;
    let prompt_sha256 = prompt_sha256(&worker_layer)?;
    let plan = resolve_plan(
        layer,
        &args.backend,
        &args.model,
        &args.profile,
        &args.precision,
    )?;
    let plan_sha256 = plan_sha256(&plan)?;
    let worker_sha256 = worker_sha256(&args.worker)?;
    let runtime_sha256 = runtime_sha256_for_plan(&plan)?;
    let trajectory = plaque
        .trajectory
        .as_ref()
        .map(|path| resolve_relative(&scene_path, path));
    let trajectory_sha256 = trajectory
        .as_deref()
        .map(crate::digest::file_sha256)
        .transpose()?;
    let request = seal_request(WorkerRequest {
        format: WORKER_REQUEST_FORMAT.into(),
        backend: plan.backend_label().to_string(),
        model: plan.semantic_model.clone(),
        device: args.device.clone(),
        plan: plan.clone(),
        plan_sha256,
        prompt_sha256,
        worker_sha256,
        runtime_sha256,
        request_sha256: String::new(),
        source: WorkerSource {
            path: args.input.canonicalize().unwrap_or(args.input.clone()),
            sha256: source_sha256,
            width: info.width,
            height: info.height,
            fps: info.fps,
            frames: info.frames,
        },
        plaque: WorkerPlaque {
            id: plaque.id.clone(),
            reference_frame: plaque.reference_frame,
            bounds: plaque.tracking_bounds(),
            trajectory,
            trajectory_sha256,
        },
        layer: worker_layer,
    })?;
    let request_path = partial.join("request.json");
    fs::write(&request_path, serde_json::to_vec_pretty(&request)?)?;

    eprintln!(
        "[ml] launching segmentation worker: layer={:?}, profile={:?}, backend={}, model={}, precision={:?}, device={}, worker={}",
        layer.id,
        plan.profile,
        plan.backend_label(),
        plan.semantic_model,
        plan.precision,
        args.device,
        args.worker.display()
    );
    run_worker_process(
        commands,
        &args.worker,
        &request_path,
        &partial,
        "segmentation",
    )?;
    validate_worker_output(&partial, &request, &info)?;
    crate::staged_output::remove_child(&partial, &request_path)?;
    // Keep the compact execution report beside the masks. It is provenance-bound to
    // the exact request and is needed by bake-offs/performance diagnostics.
    staged.commit(args.force)?;
    println!("layer artifact: {}", output.join("artifact.toml").display());
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
struct SegmentationEvidence {
    minimum_prompt_alpha_u16: Option<u16>,
    maximum_negative_alpha_u16: Option<u16>,
    nonempty_permille: u16,
    maximum_coverage_permille: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    maximum_prompt_box_fill_permille: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    minimum_interprompt_area_ratio_permille: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    interprompt_area_ratio_p05_permille: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    adjacent_iou_p05_permille: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
struct StrategyAttemptReport {
    backend: String,
    model: String,
    precision: String,
    plan_sha256: String,
    accepted: bool,
    evidence: SegmentationEvidence,
    reasons: Vec<String>,
}

#[derive(Debug, Serialize)]
struct StrategySelectionReport<'a> {
    format: &'static str,
    policy_id: &'a str,
    selected_plan_sha256: &'a str,
    attempts: &'a [StrategyAttemptReport],
}

fn strategy_evidence(
    artifact_path: &Path,
    layer: &SceneLayer,
    info: &VideoInfo,
) -> Result<SegmentationEvidence> {
    strategy_evidence_for_inputs(
        artifact_path,
        layer.role,
        layer.active_frames,
        &layer.prompts,
        layer.matte.support_threshold,
        info,
    )
}

fn strategy_evidence_for_inputs(
    artifact_path: &Path,
    role: LayerRole,
    active_frames: Option<[usize; 2]>,
    prompts: &[SegmentationPrompt],
    support_threshold: f64,
    info: &VideoInfo,
) -> Result<SegmentationEvidence> {
    let artifact = LayerArtifact::load(artifact_path)?;
    let paths = artifact.referenced_paths(artifact_path);
    if paths.len() != info.frames {
        bail!(
            "strategy evidence expected {} masks, found {}",
            info.frames,
            paths.len()
        );
    }
    let [active_start, active_end] = active_frames.unwrap_or([0, info.frames.saturating_sub(1)]);
    let pixels_per_frame = u64::from(info.width) * u64::from(info.height);
    let support_alpha = (support_threshold.clamp(0.0, 1.0) * 65_535.0).round() as u16;
    let mut nonempty = 0usize;
    let mut maximum_coverage = 0.0_f64;
    let mut positive_samples = Vec::new();
    let mut negative_samples = Vec::new();
    let mut prompt_box_fill = Vec::new();
    let mut frame_areas = vec![0_u64; paths.len()];
    let persistent_track = persistent_prompt_track(role, prompts, active_start, active_end);
    let mut adjacent_ious = Vec::new();
    let mut previous_support: Option<Vec<u8>> = None;

    for (frame, path) in paths.iter().enumerate() {
        let mask = image::open(path)
            .with_context(|| format!("failed to inspect segmentation evidence {}", path.display()))?
            .to_luma16();
        let active_pixels = mask.iter().filter(|&&alpha| alpha > support_alpha).count() as u64;
        frame_areas[frame] = active_pixels;
        if (active_start..=active_end).contains(&frame) {
            if active_pixels > 0 {
                nonempty += 1;
            }
            maximum_coverage =
                maximum_coverage.max(active_pixels as f64 / pixels_per_frame.max(1) as f64);
        }
        for prompt in prompts.iter().filter(|prompt| prompt.frame == frame) {
            for &point in &prompt.positive_points {
                positive_samples.push(sample_neighborhood_max_u16(&mask, point, 4));
            }
            if prompt.positive_points.is_empty()
                && let Some(point) = implicit_positive_point(prompt)
            {
                positive_samples.push(sample_neighborhood_max_u16(&mask, point, 4));
            }
            for &point in &prompt.negative_points {
                negative_samples.push(sample_pixel_u16(&mask, point));
            }
            if role == LayerRole::Foreground
                && let Some(bounds) = prompt.box_bounds
            {
                prompt_box_fill.push(box_fill_permille(&mask, bounds, support_alpha));
            }
        }
        if persistent_track.is_some_and(|(start, end)| (start..=end).contains(&frame)) {
            let support = mask
                .iter()
                .map(|&alpha| u8::from(alpha > support_alpha))
                .collect::<Vec<_>>();
            if let Some(previous) = previous_support.as_deref() {
                adjacent_ious.push(binary_iou_permille(previous, &support));
            }
            previous_support = Some(support);
        }
    }

    let active_frames = active_end.saturating_sub(active_start) + 1;
    let temporal = persistent_track.map(|(start, end)| {
        persistent_temporal_evidence(&frame_areas, prompts, start, end, &adjacent_ious)
    });
    Ok(SegmentationEvidence {
        minimum_prompt_alpha_u16: positive_samples.into_iter().min(),
        maximum_negative_alpha_u16: negative_samples.into_iter().max(),
        nonempty_permille: ((nonempty * 1_000) / active_frames.max(1)).min(1_000) as u16,
        maximum_coverage_permille: (maximum_coverage * 1_000.0).round().clamp(0.0, 1_000.0) as u16,
        maximum_prompt_box_fill_permille: prompt_box_fill.into_iter().max(),
        minimum_interprompt_area_ratio_permille: temporal
            .as_ref()
            .map(|evidence| evidence.minimum_area_ratio_permille),
        interprompt_area_ratio_p05_permille: temporal
            .as_ref()
            .map(|evidence| evidence.area_ratio_p05_permille),
        adjacent_iou_p05_permille: temporal
            .as_ref()
            .map(|evidence| evidence.adjacent_iou_p05_permille),
    })
}

fn box_fill_permille(
    mask: &image::ImageBuffer<image::Luma<u16>, Vec<u16>>,
    [x, y, width, height]: [f64; 4],
    support_alpha: u16,
) -> u16 {
    let left = x.floor().clamp(0.0, f64::from(mask.width())) as u32;
    let top = y.floor().clamp(0.0, f64::from(mask.height())) as u32;
    let right = (x + width).ceil().clamp(0.0, f64::from(mask.width())) as u32;
    let bottom = (y + height).ceil().clamp(0.0, f64::from(mask.height())) as u32;
    let mut active = 0_u64;
    let mut total = 0_u64;
    for yy in top..bottom {
        for xx in left..right {
            active += u64::from(mask.get_pixel(xx, yy)[0] > support_alpha);
            total += 1;
        }
    }
    ((active * 1_000 + total / 2) / total.max(1)).min(1_000) as u16
}

#[derive(Debug, Clone, Copy)]
struct PersistentTemporalEvidence {
    minimum_area_ratio_permille: u16,
    area_ratio_p05_permille: u16,
    adjacent_iou_p05_permille: u16,
}

fn persistent_prompt_track(
    role: LayerRole,
    prompts: &[SegmentationPrompt],
    active_start: usize,
    active_end: usize,
) -> Option<(usize, usize)> {
    if role != LayerRole::Foreground {
        return None;
    }
    let mut boxed = prompts
        .iter()
        .filter(|prompt| prompt.box_bounds.is_some())
        .collect::<Vec<_>>();
    boxed.sort_by_key(|prompt| prompt.frame);
    if boxed.len() < 2
        || boxed[0].object.is_none()
        || boxed.iter().any(|prompt| prompt.object != boxed[0].object)
    {
        return None;
    }
    let start = active_start.max(boxed[0].frame);
    let end = active_end.min(boxed.last()?.frame);
    (start < end).then_some((start, end))
}

fn persistent_temporal_evidence(
    frame_areas: &[u64],
    prompts: &[SegmentationPrompt],
    start: usize,
    end: usize,
    adjacent_ious: &[u16],
) -> PersistentTemporalEvidence {
    let mut anchors = prompts
        .iter()
        .filter(|prompt| prompt.box_bounds.is_some() && (start..=end).contains(&prompt.frame))
        .map(|prompt| prompt.frame)
        .collect::<Vec<_>>();
    anchors.sort_unstable();
    anchors.dedup();
    let mut ratios = Vec::with_capacity(end.saturating_sub(start) + 1);
    for frame in start..=end {
        let right = anchors.partition_point(|&anchor| anchor < frame);
        let (left_frame, right_frame) = match (right.checked_sub(1), anchors.get(right)) {
            (Some(left), Some(&right_frame)) => (anchors[left], right_frame),
            (_, Some(&right_frame)) => (right_frame, right_frame),
            (Some(left), None) => (anchors[left], anchors[left]),
            (None, None) => (frame, frame),
        };
        let left_area = frame_areas.get(left_frame).copied().unwrap_or(0) as f64;
        let right_area = frame_areas.get(right_frame).copied().unwrap_or(0) as f64;
        let expected = if right_frame == left_frame {
            left_area
        } else {
            let weight = (frame - left_frame) as f64 / (right_frame - left_frame) as f64;
            left_area * (1.0 - weight) + right_area * weight
        };
        let observed = frame_areas.get(frame).copied().unwrap_or(0) as f64;
        let ratio = if expected <= f64::EPSILON {
            0
        } else {
            (observed / expected * 1_000.0).round().clamp(0.0, 1_000.0) as u16
        };
        ratios.push(ratio);
    }
    PersistentTemporalEvidence {
        minimum_area_ratio_permille: ratios.iter().copied().min().unwrap_or(0),
        area_ratio_p05_permille: percentile_u16(&mut ratios, 0.05),
        adjacent_iou_p05_permille: percentile_u16(&mut adjacent_ious.to_vec(), 0.05),
    }
}

fn binary_iou_permille(left: &[u8], right: &[u8]) -> u16 {
    let mut intersection = 0_u64;
    let mut union = 0_u64;
    for (&left, &right) in left.iter().zip(right) {
        intersection += u64::from(left != 0 && right != 0);
        union += u64::from(left != 0 || right != 0);
    }
    (intersection * 1_000 + union / 2)
        .checked_div(union)
        .unwrap_or(0)
        .min(1_000) as u16
}

fn sample_pixel_u16(mask: &image::ImageBuffer<image::Luma<u16>, Vec<u16>>, point: [f64; 2]) -> u16 {
    let x = point[0]
        .round()
        .clamp(0.0, mask.width().saturating_sub(1) as f64) as u32;
    let y = point[1]
        .round()
        .clamp(0.0, mask.height().saturating_sub(1) as f64) as u32;
    mask.get_pixel(x, y)[0]
}

fn sample_neighborhood_max_u16(
    mask: &image::ImageBuffer<image::Luma<u16>, Vec<u16>>,
    point: [f64; 2],
    radius: i32,
) -> u16 {
    let x = point[0]
        .round()
        .clamp(0.0, mask.width().saturating_sub(1) as f64) as i32;
    let y = point[1]
        .round()
        .clamp(0.0, mask.height().saturating_sub(1) as f64) as i32;
    let mut maximum = 0u16;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let xx = (x + dx).clamp(0, mask.width().saturating_sub(1) as i32) as u32;
            let yy = (y + dy).clamp(0, mask.height().saturating_sub(1) as i32) as u32;
            maximum = maximum.max(mask.get_pixel(xx, yy)[0]);
        }
    }
    maximum
}

fn implicit_positive_point(prompt: &SegmentationPrompt) -> Option<[f64; 2]> {
    if let Some([x, y, width, height]) = prompt.box_bounds {
        return Some([x + width * 0.5, y + height * 0.5]);
    }
    if let Some(quad) = prompt.quad {
        return Some([
            quad.iter().map(|point| point[0]).sum::<f64>() / quad.len() as f64,
            quad.iter().map(|point| point[1]).sum::<f64>() / quad.len() as f64,
        ]);
    }
    if !prompt.polygon.is_empty() {
        return Some([
            prompt.polygon.iter().map(|point| point[0]).sum::<f64>() / prompt.polygon.len() as f64,
            prompt.polygon.iter().map(|point| point[1]).sum::<f64>() / prompt.polygon.len() as f64,
        ]);
    }
    None
}

fn evidence_acceptance(
    evidence: &SegmentationEvidence,
    policy: AcceptancePolicy,
    role: LayerRole,
) -> (bool, Vec<String>) {
    let mut failures = Vec::new();
    if let Some(alpha) = evidence.minimum_prompt_alpha_u16
        && alpha < policy.min_prompt_alpha_u16
    {
        failures.push(format!(
            "minimum prompt alpha {alpha} < {}",
            policy.min_prompt_alpha_u16
        ));
    }
    if let Some(alpha) = evidence.maximum_negative_alpha_u16
        && alpha > policy.max_negative_alpha_u16
    {
        failures.push(format!(
            "maximum negative-prompt alpha {alpha} > {}",
            policy.max_negative_alpha_u16
        ));
    }
    if evidence.nonempty_permille < policy.min_nonempty_permille {
        failures.push(format!(
            "non-empty frame fraction {}/1000 < {}/1000",
            evidence.nonempty_permille, policy.min_nonempty_permille
        ));
    }
    let maximum = if role == LayerRole::Foreground {
        policy.max_foreground_coverage_permille
    } else {
        policy.max_surface_coverage_permille
    };
    if evidence.maximum_coverage_permille > maximum {
        failures.push(format!(
            "maximum frame coverage {}/1000 > {maximum}/1000",
            evidence.maximum_coverage_permille
        ));
    }
    if let Some(value) = evidence.maximum_prompt_box_fill_permille
        && value > policy.max_prompt_box_fill_permille
    {
        failures.push(format!(
            "prompt-box fill {value}/1000 > {}/1000 (mask resembles a prompt rectangle rather than the object)",
            policy.max_prompt_box_fill_permille
        ));
    }
    if let Some(value) = evidence.minimum_interprompt_area_ratio_permille
        && value < policy.min_interprompt_area_ratio_permille
    {
        failures.push(format!(
            "minimum inter-prompt area ratio {value}/1000 < {}/1000",
            policy.min_interprompt_area_ratio_permille
        ));
    }
    if let Some(value) = evidence.interprompt_area_ratio_p05_permille
        && value < policy.min_interprompt_area_p05_permille
    {
        failures.push(format!(
            "p05 inter-prompt area ratio {value}/1000 < {}/1000",
            policy.min_interprompt_area_p05_permille
        ));
    }
    if let Some(value) = evidence.adjacent_iou_p05_permille
        && value < policy.min_adjacent_iou_p05_permille
    {
        failures.push(format!(
            "p05 adjacent mask IoU {value}/1000 < {}/1000",
            policy.min_adjacent_iou_p05_permille
        ));
    }
    (failures.is_empty(), failures)
}

fn write_strategy_selection(
    output: &Path,
    policy_id: &str,
    selected_plan_sha256: &str,
    attempts: &[StrategyAttemptReport],
) -> Result<()> {
    let report = StrategySelectionReport {
        format: "plaque-forge.segmentation-strategy-selection/1",
        policy_id,
        selected_plan_sha256,
        attempts,
    };
    fs::write(
        output.join("strategy-selection.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}

/// Materialize every prompted scene layer that is still missing its artifact.
///
/// This is used by the high-level analyze workflow so a user does not need to discover
/// and invoke the lower-level `segment` command for each layer. Existing artifacts are
/// reused unless `force` is explicitly requested.
pub struct PromptedLayersRequest<'a> {
    pub input: &'a Path,
    pub explicit_scene: Option<&'a Path>,
    pub surface_id: Option<&'a str>,
    pub worker: &'a Path,
    pub backend: &'a str,
    pub model: &'a str,
    pub device: &'a str,
    pub profile: &'a str,
    pub precision: &'a str,
    pub force: bool,
    pub ffprobe: &'a Path,
    pub info: &'a VideoInfo,
    pub source_sha256: &'a str,
    /// Transaction-owned root where one directory per generated layer is written.
    pub output_root: &'a Path,
    /// Previously published layer root eligible for validated cache reuse.
    pub reuse_root: Option<&'a Path>,
    pub commands: &'a dyn crate::infrastructure::CommandExecutor,
}

pub fn ensure_prompted_layers(request: PromptedLayersRequest<'_>) -> Result<usize> {
    let PromptedLayersRequest {
        input,
        explicit_scene,
        surface_id,
        worker,
        backend,
        model,
        device,
        profile,
        precision,
        force,
        ffprobe,
        info,
        source_sha256,
        output_root,
        reuse_root,
        commands,
    } = request;
    let Some(loaded) = find_scene(input, explicit_scene)? else {
        eprintln!(
            "[ml] segmentation skipped: no scene manifest for {}",
            input.display()
        );
        return Ok(0);
    };
    let plaque = loaded.document.select_surface(surface_id)?;
    let prompted = loaded
        .document
        .layers
        .iter()
        .filter(|layer| layer.surface == plaque.id && !layer.prompts.is_empty())
        .collect::<Vec<_>>();
    if prompted.is_empty() {
        eprintln!(
            "[ml] segmentation skipped: plaque {:?} declares no prompted ML layers",
            plaque.id
        );
        return Ok(0);
    }

    let expected_worker = worker_sha256(worker)?;
    prune_temporary_layer_cache()?;
    let mut pending = Vec::new();
    let mut reused = 0usize;

    for layer in &prompted {
        let output = output_root.join(&layer.id);
        let prompt = prompt_sha256(&worker_layer(layer, info)?)?;
        let strategy = resolve_strategy(layer, backend, model, profile, precision)?;
        let mut accepted_cache = false;

        if !force {
            'candidate: for candidate in &strategy.candidates {
                let plan = materialize_candidate_plan(layer, candidate)?;
                let plan_hash = plan_sha256(&plan)?;
                let planned_backend = plan.backend_label().to_string();
                let planned_model = plan.semantic_model.clone();
                let expected_runtime = runtime_sha256_for_plan(&plan)?;
                let expected = || LayerCacheIdentity {
                    backend: &planned_backend,
                    model: &planned_model,
                    device,
                    source_sha256,
                    prompt_sha256: &prompt,
                    worker_sha256: &expected_worker,
                    runtime_sha256: expected_runtime.as_deref(),
                    plan_sha256: &plan_hash,
                };

                let candidates = [
                    Some(output.join("artifact.toml")),
                    reuse_root.map(|root| root.join(&layer.id).join("artifact.toml")),
                    Some(temporary_layer_cache(expected()).join("artifact.toml")),
                ];
                for artifact in candidates.into_iter().flatten() {
                    if !layer_cache_is_current(&artifact, expected()) {
                        continue;
                    }
                    let evidence = strategy_evidence(&artifact, layer, info)?;
                    let (accepted, failures) =
                        evidence_acceptance(&evidence, strategy.acceptance, layer.role);
                    if !accepted {
                        eprintln!(
                            "[ml] cached candidate rejected by adaptive evidence for {:?}: {}",
                            layer.id,
                            failures.join("; ")
                        );
                        continue;
                    }
                    if artifact != output.join("artifact.toml") {
                        copy_layer_cache(&artifact, &output)?;
                    }
                    let report = [StrategyAttemptReport {
                        backend: planned_backend.clone(),
                        model: planned_model.clone(),
                        precision: plan.precision.label().to_string(),
                        plan_sha256: plan_hash.clone(),
                        accepted: true,
                        evidence,
                        reasons: Vec::new(),
                    }];
                    write_strategy_selection(&output, &strategy.policy_id, &plan_hash, &report)?;
                    reused += 1;
                    accepted_cache = true;
                    eprintln!(
                        "[ml] adaptive cache hit for {:?}: backend={}, model={}",
                        layer.id, planned_backend, planned_model
                    );
                    break 'candidate;
                }
            }
        }
        if !accepted_cache {
            pending.push((layer.id.clone(), prompt));
        }
    }

    if pending.is_empty() {
        eprintln!(
            "[ml] segmentation cache hit: {} prompted layer artifact(s) already satisfy adaptive policy; Python will not run",
            reused
        );
        return Ok(0);
    }
    eprintln!(
        "[ml] segmentation required: {} layer(s) pending, {} reused{}",
        pending.len(),
        reused,
        if force { " (forced regeneration)" } else { "" }
    );

    for (layer_id, prompt) in &pending {
        let scene_layer = prompted
            .iter()
            .copied()
            .find(|candidate| candidate.id == *layer_id)
            .context("pending prompted layer disappeared from the scene")?;
        let strategy = resolve_strategy(scene_layer, backend, model, profile, precision)?;
        let output = output_root.join(layer_id);
        let mut attempts = Vec::new();
        let mut selected = None;

        for (index, candidate) in strategy.candidates.iter().enumerate() {
            let plan = materialize_candidate_plan(scene_layer, candidate)?;
            let plan_hash = plan_sha256(&plan)?;
            let candidate_backend = plan.backend_label().to_string();
            let candidate_model = plan.semantic_model.clone();
            eprintln!(
                "[ml] Rust strategy candidate {}/{} for {:?}: backend={}, model={}, precision={}",
                index + 1,
                strategy.candidates.len(),
                layer_id,
                candidate_backend,
                candidate_model,
                plan.precision.label()
            );
            run_with_executor(
                SegmentArgs {
                    input: input.to_path_buf(),
                    scene: Some(loaded.path.clone()),
                    surface: Some(plaque.id.clone()),
                    layer: layer_id.clone(),
                    worker: worker.to_path_buf(),
                    backend: candidate_backend.clone(),
                    model: candidate_model.clone(),
                    device: device.to_string(),
                    profile: plan.profile.label().to_string(),
                    precision: plan.precision.label().to_string(),
                    output: Some(output.clone()),
                    force: output.exists(),
                    ffprobe: ffprobe.to_path_buf(),
                },
                commands,
            )?;
            let artifact = output.join("artifact.toml");
            let evidence = strategy_evidence(&artifact, scene_layer, info)?;
            let (accepted, failures) =
                evidence_acceptance(&evidence, strategy.acceptance, scene_layer.role);
            attempts.push(StrategyAttemptReport {
                backend: candidate_backend.clone(),
                model: candidate_model.clone(),
                precision: plan.precision.label().to_string(),
                plan_sha256: plan_hash.clone(),
                accepted,
                evidence,
                reasons: failures.clone(),
            });
            if accepted {
                selected = Some((plan, plan_hash));
                break;
            }
            if index + 1 < strategy.candidates.len() {
                eprintln!(
                    "[ml] Rust strategy escalating {:?}: {}",
                    layer_id,
                    failures.join("; ")
                );
            }
        }

        let Some((selected_plan, selected_hash)) = selected else {
            write_strategy_selection(&output, &strategy.policy_id, "none", &attempts)?;
            let reasons = attempts
                .last()
                .map(|attempt| attempt.reasons.join("; "))
                .unwrap_or_else(|| "no candidate executed".to_string());
            bail!(
                "all Rust-planned segmentation candidates failed independent evidence for {:?}: {}",
                layer_id,
                reasons
            );
        };
        write_strategy_selection(&output, &strategy.policy_id, &selected_hash, &attempts)?;

        let selected_backend = selected_plan.backend_label().to_string();
        let selected_model = selected_plan.semantic_model.clone();
        let expected_runtime = runtime_sha256_for_plan(&selected_plan)?;
        store_temporary_layer_cache(
            &output.join("artifact.toml"),
            LayerCacheIdentity {
                backend: &selected_backend,
                model: &selected_model,
                device,
                source_sha256,
                prompt_sha256: prompt,
                worker_sha256: &expected_worker,
                runtime_sha256: expected_runtime.as_deref(),
                plan_sha256: &selected_hash,
            },
        )?;
    }
    Ok(pending.len())
}

fn store_temporary_layer_cache(
    artifact_path: &Path,
    identity: LayerCacheIdentity<'_>,
) -> Result<()> {
    let target = temporary_layer_cache(identity);
    let staged = crate::staged_output::create(&target)?;
    copy_layer_cache(artifact_path, staged.path())?;
    staged.commit(true)?;
    prune_temporary_layer_cache()
}

fn prune_temporary_layer_cache() -> Result<()> {
    let root = std::env::temp_dir()
        .join("plaque-forge")
        .join("ml-layer-cache");
    let directory = match fs::read_dir(&root) {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let mut entries = directory.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| {
        Reverse(
            entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(UNIX_EPOCH),
        )
    });
    for (index, entry) in entries.into_iter().enumerate() {
        let path = entry.path();
        let expired = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > TEMP_LAYER_CACHE_MAX_AGE);
        if index >= TEMP_LAYER_CACHE_MAX_ENTRIES || expired {
            if entry.file_type()?.is_dir() {
                fs::remove_dir_all(&path)?;
            } else {
                fs::remove_file(&path)?;
            }
        }
    }
    Ok(())
}

fn copy_layer_cache(artifact_path: &Path, output: &Path) -> Result<()> {
    let artifact = LayerArtifact::load(artifact_path)?;
    let owner = artifact_path
        .parent()
        .context("layer artifact has no parent directory")?;
    fs::create_dir_all(output)?;
    for source in artifact.referenced_paths(artifact_path) {
        let relative = source.strip_prefix(owner).with_context(|| {
            format!(
                "layer artifact asset escapes its cache: {}",
                source.display()
            )
        })?;
        let destination = output.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&source, &destination).with_context(|| {
            format!(
                "failed to reuse validated layer asset {} as {}",
                source.display(),
                destination.display()
            )
        })?;
    }
    fs::copy(artifact_path, output.join("artifact.toml"))?;
    for sidecar in ["result.json", "strategy-selection.json"] {
        let source = owner.join(sidecar);
        if source.is_file() {
            fs::copy(source, output.join(sidecar))?;
        }
    }
    Ok(())
}

/// Inputs for the analyzer's automatic ML scene of a Rust-derived foreground mask.
///
/// Unlike authored scene layers, this is generated cache state: the Rust analyzer
/// first finds a rough crossing, then asks the replaceable Python worker to sharpen that
/// source-pixel mask. A worker failure is handled by the caller as an optional fallback.
pub struct AutomaticForegroundRequest<'a> {
    pub input: &'a Path,
    pub worker: &'a Path,
    pub backend: &'a str,
    pub model: &'a str,
    pub device: &'a str,
    pub profile: &'a str,
    pub precision: &'a str,
    pub info: &'a VideoInfo,
    pub plaque: RectF,
    pub seed_masks: &'a Path,
    pub analysis_root: &'a Path,
    /// Bypass otherwise-valid ML cache entries on an explicit `--force-ml` run.
    pub force: bool,
    /// Previous complete analysis cache, when replacing one. Automatic ML output
    /// may be copied from it only after validating the complete semantic request.
    pub reuse_root: Option<&'a Path>,
    pub commands: &'a dyn crate::infrastructure::CommandExecutor,
}

pub fn refine_automatic_foreground(request: AutomaticForegroundRequest<'_>) -> Result<bool> {
    let prompts = automatic_foreground_prompts(request.seed_masks, request.info)?;
    if prompts.is_empty() {
        eprintln!("[ml] automatic foreground skipped: Rust found no stable non-empty seed mask");
        return Ok(false);
    }

    let output = request.analysis_root.join("ml-foreground");
    let worker_layer = WorkerLayer {
        id: "automatic-foreground".to_string(),
        role: LayerRole::Foreground,
        affects_layout: false,
        matte_mode: LayerMatteMode::Opaque,
        subject: LayerSubject::Unspecified,
        active_frames: None,
        seed_masks: prompts
            .iter()
            .map(|prompt| {
                let path = request.seed_masks.join(format!("{:06}.png", prompt.frame));
                Ok(WorkerSeedMask {
                    frame: prompt.frame,
                    sha256: crate::digest::file_sha256(&path)?,
                    path,
                })
            })
            .collect::<Result<Vec<_>>>()?,
        prompts,
    };
    let strategy = segmentation_strategy::strategy(PlanningInput {
        profile: SegmentationProfile::parse(request.profile)?,
        precision_override: SegmentationPrecision::parse(request.precision)?,
        backend_override: request.backend,
        model_override: request.model,
        role: LayerRole::Foreground,
        matte_mode: LayerMatteMode::Opaque,
        subject: LayerSubject::Unspecified,
        prompts: &worker_layer.prompts,
    })?;
    let source_sha256 = crate::digest::file_sha256(request.input)?;
    let worker_hash = worker_sha256(request.worker)?;
    let prompt_hash = prompt_sha256(&worker_layer)?;

    let build_request = |plan: &SegmentationPlan| -> Result<WorkerRequest> {
        let plan_hash = plan_sha256(plan)?;
        seal_request(WorkerRequest {
            format: WORKER_REQUEST_FORMAT.into(),
            backend: plan.backend_label().to_string(),
            model: plan.semantic_model.clone(),
            device: request.device.to_string(),
            plan: plan.clone(),
            plan_sha256: plan_hash,
            prompt_sha256: prompt_hash.clone(),
            worker_sha256: worker_hash.clone(),
            runtime_sha256: runtime_sha256_for_plan(plan)?,
            request_sha256: String::new(),
            source: WorkerSource {
                path: request
                    .input
                    .canonicalize()
                    .unwrap_or_else(|_| request.input.to_path_buf()),
                sha256: source_sha256.clone(),
                width: request.info.width,
                height: request.info.height,
                fps: request.info.fps,
                frames: request.info.frames,
            },
            plaque: WorkerPlaque {
                id: "automatic".to_string(),
                reference_frame: worker_layer.prompts.first().map(|prompt| prompt.frame),
                bounds: Some([
                    request.plaque.x,
                    request.plaque.y,
                    request.plaque.width,
                    request.plaque.height,
                ]),
                trajectory: None,
                trajectory_sha256: None,
            },
            layer: worker_layer.clone(),
        })
    };

    if !request.force
        && let Some(reuse_root) = request.reuse_root
    {
        let cached = reuse_root.join("ml-foreground");
        for plan in &strategy.candidates {
            let request_document = build_request(plan)?;
            if cached.is_dir()
                && validate_worker_output(&cached, &request_document, request.info).is_ok()
            {
                let evidence = strategy_evidence_for_inputs(
                    &cached.join("artifact.toml"),
                    LayerRole::Foreground,
                    None,
                    &worker_layer.prompts,
                    0.03,
                    request.info,
                )?;
                let (accepted, failures) =
                    evidence_acceptance(&evidence, strategy.acceptance, LayerRole::Foreground);
                if !accepted {
                    eprintln!(
                        "[ml] automatic foreground cache rejected by adaptive evidence: {}",
                        failures.join("; ")
                    );
                    continue;
                }
                copy_automatic_foreground_cache(&cached, &output, request.info.frames)?;
                let attempt = [StrategyAttemptReport {
                    backend: request_document.backend.clone(),
                    model: request_document.model.clone(),
                    precision: request_document.plan.precision.label().to_string(),
                    plan_sha256: request_document.plan_sha256.clone(),
                    accepted: true,
                    evidence,
                    reasons: Vec::new(),
                }];
                write_strategy_selection(
                    &output,
                    &strategy.policy_id,
                    &request_document.plan_sha256,
                    &attempt,
                )?;
                if !install_automatic_foreground_masks(request.analysis_root, request.info.frames)?
                {
                    return Ok(false);
                }
                eprintln!(
                    "[ml] automatic foreground cache hit: reused {} validated lossless mask(s)",
                    request.info.frames
                );
                return Ok(true);
            }
        }
    }

    let mut attempts = Vec::new();
    for (index, plan) in strategy.candidates.iter().enumerate() {
        if output.exists() {
            crate::staged_output::remove_child(request.analysis_root, &output)?;
        }
        fs::create_dir_all(&output).with_context(|| {
            format!("failed to create automatic ML output {}", output.display())
        })?;
        let request_document = build_request(plan)?;
        let request_path = output.join("request.json");
        fs::write(&request_path, serde_json::to_vec_pretty(&request_document)?)?;

        eprintln!(
            "[ml] automatic foreground Rust candidate {}/{}: {} seed frame(s), profile={:?}, backend={}, model={}, precision={:?}, device={}",
            index + 1,
            strategy.candidates.len(),
            request_document.layer.prompts.len(),
            plan.profile,
            plan.backend_label(),
            plan.semantic_model,
            plan.precision,
            request.device
        );
        if let Err(error) = run_worker_process(
            request.commands,
            request.worker,
            &request_path,
            &output,
            "automatic foreground",
        ) {
            crate::staged_output::remove_child(request.analysis_root, &output)?;
            return Err(error);
        }
        if let Err(error) = validate_worker_output(&output, &request_document, request.info) {
            crate::staged_output::remove_child(request.analysis_root, &output)?;
            return Err(error.context("automatic foreground worker output was rejected"));
        }
        let evidence = strategy_evidence_for_inputs(
            &output.join("artifact.toml"),
            LayerRole::Foreground,
            None,
            &worker_layer.prompts,
            0.03,
            request.info,
        )?;
        let (accepted, failures) =
            evidence_acceptance(&evidence, strategy.acceptance, LayerRole::Foreground);
        attempts.push(StrategyAttemptReport {
            backend: request_document.backend.clone(),
            model: request_document.model.clone(),
            precision: plan.precision.label().to_string(),
            plan_sha256: request_document.plan_sha256.clone(),
            accepted,
            evidence,
            reasons: failures.clone(),
        });
        if !accepted {
            if index + 1 < strategy.candidates.len() {
                eprintln!(
                    "[ml] automatic foreground escalating: {}",
                    failures.join("; ")
                );
                continue;
            }
            write_strategy_selection(&output, &strategy.policy_id, "none", &attempts)?;
            bail!(
                "all Rust-planned automatic foreground candidates failed independent evidence: {}",
                failures.join("; ")
            );
        }

        crate::staged_output::remove_child(&output, &request_path)?;
        write_strategy_selection(
            &output,
            &strategy.policy_id,
            &request_document.plan_sha256,
            &attempts,
        )?;
        match install_automatic_foreground_masks(request.analysis_root, request.info.frames) {
            Ok(true) => {}
            Ok(false) => {
                crate::staged_output::remove_child(request.analysis_root, &output)?;
                return Ok(false);
            }
            Err(error) => {
                crate::staged_output::remove_child(request.analysis_root, &output)?;
                return Err(error);
            }
        }
        eprintln!(
            "[ml] automatic foreground installed: {} source-pixel mask(s)",
            request.info.frames
        );
        return Ok(true);
    }
    bail!("automatic foreground strategy unexpectedly had no candidates")
}

fn copy_automatic_foreground_cache(
    source: &Path,
    destination: &Path,
    expected_frames: usize,
) -> Result<()> {
    if destination.exists() {
        crate::staged_output::remove_child(
            destination
                .parent()
                .context("automatic ML cache destination has no parent")?,
            destination,
        )?;
    }
    fs::create_dir_all(destination)?;
    for name in ["artifact.toml", "result.json"] {
        fs::copy(source.join(name), destination.join(name))
            .with_context(|| format!("failed to reuse automatic ML cache member {name}"))?;
    }
    let selection = source.join("strategy-selection.json");
    if selection.is_file() {
        fs::copy(selection, destination.join("strategy-selection.json"))?;
    }
    for frame in 0..expected_frames {
        let name = format!("{frame:06}.png");
        fs::copy(source.join(&name), destination.join(&name))
            .with_context(|| format!("failed to reuse automatic ML mask {name}"))?;
    }
    Ok(())
}

/// Reinstall already-generated ML masks after Rust recomputes extraction/occlusion during
/// masked retracking. The ML masks are in source coordinates, so they remain valid across
/// that internal scene pass and do not need a second Python invocation.
pub fn install_automatic_foreground_masks(
    analysis_root: &Path,
    expected_frames: usize,
) -> Result<bool> {
    let ml_root = analysis_root.join("ml-foreground");
    let artifact_path = ml_root.join("artifact.toml");
    if !artifact_path.is_file() {
        return Ok(false);
    }
    let artifact = LayerArtifact::load(&artifact_path)?;
    if artifact.kind != LayerArtifactKind::AlphaSequence
        || artifact.coordinates != LayerCoordinates::SourcePixels
    {
        bail!("automatic ML foreground artifact must be a source-pixel alpha sequence");
    }
    let sources = artifact.referenced_paths(&artifact_path);
    if sources.len() != expected_frames {
        bail!(
            "automatic ML foreground contains {} frames, expected {expected_frames}",
            sources.len()
        );
    }

    let destination = analysis_root.join(OCCLUDER_DIR);
    let automatic_photometric_root = analysis_root.join(AUTOMATIC_MATERIAL_WORK_DIR);
    let authored_photometric_root = analysis_root.join(AUTHORED_OCCLUDER_WORK_DIR);
    // Semantic membership is not optical opacity, while a photometric residual is
    // not necessarily a solid object (a cast shadow is the common counterexample).
    // Automatic ML therefore gates a complete lossless photometric material
    // sequence. Neither source is installed on its own.
    if !destination.is_dir() || !automatic_photometric_root.is_dir() {
        return Ok(false);
    }
    for frame in 0..expected_frames {
        let photometric = automatic_photometric_root.join(format!("{frame:06}.png"));
        if !photometric.is_file() {
            bail!(
                "automatic ML foreground requires a complete photometric mask sequence; missing {}",
                photometric.display()
            );
        }
        let authored = authored_photometric_root.join(format!("{frame:06}.png"));
        if authored_photometric_root.is_dir() && !authored.is_file() {
            bail!(
                "automatic ML foreground requires a complete authored-detail mask sequence; missing {}",
                authored.display()
            );
        }
    }
    let incoming = analysis_root.join(".occluder-ml-incoming");
    let previous = analysis_root.join(".occluder-rust-backup");
    for owned in [&incoming, &previous] {
        if owned.exists() {
            crate::staged_output::remove_child(analysis_root, owned)?;
        }
    }
    fs::create_dir(&incoming)?;
    for (frame, source) in sources.iter().enumerate() {
        let target = incoming.join(format!("{frame:06}.png"));
        let photometric = automatic_photometric_root.join(format!("{frame:06}.png"));
        let authored = authored_photometric_root.join(format!("{frame:06}.png"));
        let installed = fuse_automatic_foreground_mask(
            source,
            &photometric,
            authored.is_file().then_some(authored.as_path()),
            &target,
        );
        if let Err(error) = installed {
            let _ = crate::staged_output::remove_child(analysis_root, &incoming);
            return Err(error).with_context(|| {
                format!(
                    "failed to stage automatic foreground mask {} -> {}",
                    source.display(),
                    target.display()
                )
            });
        }
    }
    if destination.exists() {
        fs::rename(&destination, &previous).with_context(|| {
            format!("failed to preserve Rust masks in {}", destination.display())
        })?;
    }
    if let Err(error) = fs::rename(&incoming, &destination) {
        if previous.exists() {
            let _ = fs::rename(&previous, &destination);
        }
        return Err(error).context("failed to install validated ML foreground masks");
    }
    if previous.exists() {
        crate::staged_output::remove_child(analysis_root, &previous)?;
    }
    Ok(true)
}

/// Fuse semantic temporal continuity with direct lossless-image evidence.
///
/// SAM2/ViTMatte identifies the foreground object; the Rust residual establishes
/// where visible material actually differs from the writing surface. Their
/// intersection rejects cast shadows and background animation. A two-pixel
/// semantic fringe absorbs matte/registration disagreement without filling holes
/// in webs, vines, feathers, or other porous silhouettes.
fn fuse_automatic_foreground_mask(
    semantic_path: &Path,
    photometric_path: &Path,
    authored_path: Option<&Path>,
    target: &Path,
) -> Result<()> {
    let semantic = image::open(semantic_path)
        .with_context(|| format!("failed to load semantic mask {}", semantic_path.display()))?
        .to_luma16();
    let photometric = image::open(photometric_path)
        .with_context(|| {
            format!(
                "failed to load photometric mask {}",
                photometric_path.display()
            )
        })?
        .to_luma16();
    if semantic.dimensions() != photometric.dimensions() {
        bail!(
            "semantic mask {} is {}x{}, but photometric mask {} is {}x{}",
            semantic_path.display(),
            semantic.width(),
            semantic.height(),
            photometric_path.display(),
            photometric.width(),
            photometric.height()
        );
    }
    let automatic = fuse_automatic_foreground_alpha(
        semantic.as_raw(),
        photometric.as_raw(),
        semantic.width() as usize,
        semantic.height() as usize,
    );
    let fused = if let Some(authored_path) = authored_path {
        let authored = image::open(authored_path)
            .with_context(|| {
                format!(
                    "failed to load authored foreground detail {}",
                    authored_path.display()
                )
            })?
            .to_luma16();
        if authored.dimensions() != semantic.dimensions() {
            bail!(
                "authored foreground detail {} is {}x{}, but semantic mask {} is {}x{}",
                authored_path.display(),
                authored.width(),
                authored.height(),
                semantic_path.display(),
                semantic.width(),
                semantic.height()
            );
        }
        merge_automatic_and_authored_alpha(&automatic, authored.as_raw())
    } else {
        automatic
    };
    let image = ImageBuffer::<Luma<u16>, _>::from_raw(semantic.width(), semantic.height(), fused)
        .context("automatic foreground fusion produced invalid mask dimensions")?;
    image
        .save(target)
        .with_context(|| format!("failed to save fused foreground mask {}", target.display()))
}

fn merge_automatic_and_authored_alpha(automatic: &[u16], authored: &[u16]) -> Vec<u16> {
    if automatic.len() != authored.len() {
        return Vec::new();
    }
    automatic
        .iter()
        .zip(authored)
        .map(|(&automatic, &authored)| automatic.max(authored))
        .collect()
}

fn fuse_automatic_foreground_alpha(
    semantic: &[u16],
    photometric: &[u16],
    width: usize,
    height: usize,
) -> Vec<u16> {
    if semantic.len() != photometric.len() || semantic.len() != width.saturating_mul(height) {
        return Vec::new();
    }
    let mut semantic_guard = semantic.to_vec();
    // A two-pixel source-space fringe absorbs antialiasing and small matte/
    // homography disagreement. It decays with distance and never creates alpha
    // without direct photometric material evidence at that pixel.
    const WEIGHTS: [u32; 3] = [65_535, 42_598, 19_661]; // 1.00, 0.65, 0.30
    for y in 0..height {
        for x in 0..width {
            let source = semantic[y * width + x];
            if source == 0 {
                continue;
            }
            for dy in -2_i32..=2 {
                for dx in -2_i32..=2 {
                    let xx = x as i32 + dx;
                    let yy = y as i32 + dy;
                    if xx < 0 || yy < 0 || xx >= width as i32 || yy >= height as i32 {
                        continue;
                    }
                    let distance = dx.unsigned_abs().max(dy.unsigned_abs()) as usize;
                    let weighted =
                        ((u32::from(source) * WEIGHTS[distance] + 32_767) / 65_535) as u16;
                    let target = yy as usize * width + xx as usize;
                    semantic_guard[target] = semantic_guard[target].max(weighted);
                }
            }
        }
    }
    semantic
        .iter()
        .zip(photometric)
        .zip(semantic_guard)
        .map(|((&semantic, &photometric), guard)| photometric.min(semantic.max(guard)))
        .collect()
}

fn automatic_foreground_prompts(
    seed_masks: &Path,
    info: &VideoInfo,
) -> Result<Vec<SegmentationPrompt>> {
    if !seed_masks.is_dir() {
        return Ok(Vec::new());
    }
    let mut candidates = Vec::<(usize, u64, [f64; 4], [f64; 2])>::new();
    for entry in fs::read_dir(seed_masks)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("png") {
            continue;
        }
        let Some(frame) = path
            .file_stem()
            .and_then(|value| value.to_str())
            .and_then(|value| value.parse::<usize>().ok())
        else {
            continue;
        };
        if frame >= info.frames {
            continue;
        }
        let image = image::open(&path)
            .with_context(|| {
                format!(
                    "failed to inspect automatic occluder seed {}",
                    path.display()
                )
            })?
            .to_luma8();
        let mut min_x = image.width();
        let mut min_y = image.height();
        let mut max_x = 0u32;
        let mut max_y = 0u32;
        let mut count = 0u64;
        let mut sum_x = 0u64;
        let mut sum_y = 0u64;
        for (x, y, pixel) in image.enumerate_pixels() {
            if pixel[0] <= 24 {
                continue;
            }
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
            count += 1;
            sum_x += u64::from(x);
            sum_y += u64::from(y);
        }
        if count < 24 || min_x > max_x || min_y > max_y {
            continue;
        }
        let raw_width = max_x - min_x + 1;
        let raw_height = max_y - min_y + 1;
        let margin_x = ((raw_width as f64 * 0.14).round() as u32).max(6);
        let margin_y = ((raw_height as f64 * 0.14).round() as u32).max(6);
        let left = min_x.saturating_sub(margin_x);
        let top = min_y.saturating_sub(margin_y);
        let right = (max_x + margin_x).min(info.width.saturating_sub(1));
        let bottom = (max_y + margin_y).min(info.height.saturating_sub(1));
        let centroid = [sum_x as f64 / count as f64, sum_y as f64 / count as f64];
        let positive = image
            .enumerate_pixels()
            .filter(|(_, _, pixel)| pixel[0] > 24)
            .min_by(|(left_x, left_y, _), (right_x, right_y, _)| {
                let distance = |x: &u32, y: &u32| {
                    (f64::from(*x) - centroid[0]).powi(2) + (f64::from(*y) - centroid[1]).powi(2)
                };
                distance(left_x, left_y).total_cmp(&distance(right_x, right_y))
            })
            .map(|(x, y, _)| [f64::from(x), f64::from(y)])
            .context("automatic seed unexpectedly lost its foreground pixels")?;
        candidates.push((
            frame,
            count,
            [
                left as f64,
                top as f64,
                (right - left + 1) as f64,
                (bottom - top + 1) as f64,
            ],
            positive,
        ));
    }

    candidates.sort_by_key(|candidate| Reverse(candidate.1));
    let minimum_spacing = (info.frames / 8).max(1);
    let mut selected = Vec::new();
    for (frame, _count, bounds, positive) in candidates {
        if selected
            .iter()
            .any(|prompt: &SegmentationPrompt| prompt.frame.abs_diff(frame) < minimum_spacing)
        {
            continue;
        }
        selected.push(SegmentationPrompt {
            frame,
            coordinates: SpatialCoordinates::SourcePixels,
            // A seed is a distinct temporal hypothesis, not a correction to every
            // other foreground crossing. Treating a spider at frame 1 and a web at
            // frame 102 as one SAM2 object lets the later prompt redefine the former.
            // The worker tracks each hypothesis independently and unions their soft
            // alpha only after propagation; duplicate hypotheses are harmless under
            // max-union, while conflated semantic actors are not recoverable.
            object: Some(format!("automatic-foreground-{frame}")),
            concept: None,
            box_bounds: Some(bounds),
            positive_points: vec![positive],
            negative_points: Vec::new(),
            polygon: Vec::new(),
            quad: None,
        });
        if selected.len() == 3 {
            break;
        }
    }
    selected.sort_by_key(|prompt| prompt.frame);
    Ok(selected)
}

fn validate_worker_output(
    root: &Path,
    request: &WorkerRequest,
    info: &video::VideoInfo,
) -> Result<()> {
    let result_path = root.join("result.json");
    let result: WorkerResult = serde_json::from_slice(
        &fs::read(&result_path)
            .with_context(|| format!("worker did not create {}", result_path.display()))?,
    )?;
    if result.format != WORKER_RESULT_FORMAT
        || result.backend != request.backend
        || result.model != request.model
        || result.version.trim().is_empty()
        || result.plan_sha256 != request.plan_sha256
        || result.precision != request.plan.precision
        || result.frames != info.frames
        || result.request_sha256 != request.request_sha256
        || result.source_sha256 != request.source.sha256
        || result.prompt_sha256 != request.prompt_sha256
        || result.worker_sha256 != request.worker_sha256
        || result.runtime_sha256 != request.runtime_sha256
        || ![result.mean_confidence, result.minimum_confidence]
            .iter()
            .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
        || ![result.mean_coverage, result.maximum_coverage]
            .iter()
            .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
        || result.execution.iter().any(|stage| {
            stage.stage.trim().is_empty()
                || stage.device.trim().is_empty()
                || stage.precision.trim().is_empty()
                || !stage.seconds.is_finite()
                || stage.seconds < 0.0
                || stage
                    .process_peak_rss_mib
                    .is_some_and(|value| !value.is_finite() || value < 0.0)
                || stage
                    .accelerator_peak_mib
                    .is_some_and(|value| !value.is_finite() || value < 0.0)
                || (stage.cache_hit && stage.seconds > 0.1)
                || stage.note.as_deref().is_some_and(str::is_empty)
        })
    {
        bail!("worker result.json does not match the request or source");
    }
    let artifact_path = root.join("artifact.toml");
    let artifact = LayerArtifact::load(&artifact_path)?;
    if artifact.kind != LayerArtifactKind::AlphaSequence
        || artifact.coordinates != LayerCoordinates::SourcePixels
        || artifact.first_frame != Some(0)
        || artifact.last_frame != Some(info.frames.saturating_sub(1))
    {
        bail!("segmentation worker must produce a complete source-pixel alpha sequence");
    }
    let generator = artifact
        .generator
        .as_ref()
        .context("worker artifact is missing generator provenance")?;
    if generator.backend != request.backend
        || generator.model != request.model
        || generator.version != result.version
        || generator.requested_device.as_deref() != Some(request.device.as_str())
        || generator.source_sha256.as_deref() != Some(request.source.sha256.as_str())
        || generator.prompt_sha256.as_deref() != Some(request.prompt_sha256.as_str())
        || generator.worker_sha256.as_deref() != Some(request.worker_sha256.as_str())
        || generator.runtime_sha256.as_deref() != request.runtime_sha256.as_deref()
        || generator.request_sha256.as_deref() != Some(request.request_sha256.as_str())
        || generator.plan_sha256.as_deref() != Some(request.plan_sha256.as_str())
    {
        bail!("artifact generator provenance differs from result.json");
    }
    let paths = artifact.referenced_paths(&artifact_path);
    if paths.len() != info.frames {
        bail!(
            "segmentation worker produced {} mask references, expected {}",
            paths.len(),
            info.frames
        );
    }
    let [active_start, active_end] = request
        .layer
        .active_frames
        .unwrap_or([0, info.frames.saturating_sub(1)]);
    let boxed_prompts = request
        .layer
        .prompts
        .iter()
        .filter(|prompt| prompt.box_bounds.is_some())
        .collect::<Vec<_>>();
    let bounded_authored_foreground = request.layer.role == LayerRole::Foreground
        && boxed_prompts.len() >= 2
        && boxed_prompts
            .iter()
            .all(|prompt| prompt.object == boxed_prompts[0].object);
    let pixels_per_frame = u64::from(info.width) * u64::from(info.height);
    let mut nonempty_frames = 0usize;
    let mut coverage_sum = 0.0;
    let mut maximum_coverage = 0.0_f64;
    let mut soft_edge_pixels = 0u64;
    for (frame, path) in paths.iter().enumerate() {
        let reader = image::ImageReader::open(path)
            .with_context(|| format!("failed to load worker mask {}", path.display()))?
            .with_guessed_format()?;
        if reader.format() != Some(image::ImageFormat::Png) {
            bail!("worker mask is not a lossless PNG: {}", path.display());
        }
        let image = reader
            .decode()
            .with_context(|| format!("failed to load worker mask {}", path.display()))?;
        if image.width() != info.width || image.height() != info.height {
            bail!(
                "worker mask {} is {}x{}, expected {}x{}",
                path.display(),
                image.width(),
                image.height(),
                info.width,
                info.height
            );
        }
        let mask = match image {
            image::DynamicImage::ImageLuma16(mask) => image::GrayImage::from_raw(
                mask.width(),
                mask.height(),
                mask.into_raw()
                    .into_iter()
                    .map(|value| ((u32::from(value) * 255 + 32_767) / 65_535) as u8)
                    .collect(),
            )
            .context("failed to normalize 16-bit worker mask")?,
            other => other.to_luma8(),
        };
        let active_pixels = mask.iter().filter(|&&alpha| alpha > 8).count() as u64;
        soft_edge_pixels += mask
            .iter()
            .filter(|&&alpha| (1..=254).contains(&alpha))
            .count() as u64;
        if active_pixels > 0 {
            nonempty_frames += 1;
        }
        if !(active_start..=active_end).contains(&frame) && active_pixels > 0 {
            bail!("worker mask is non-empty outside active_frames at frame {frame}");
        }
        if (active_start..=active_end).contains(&frame) {
            let coverage = active_pixels as f64 / pixels_per_frame.max(1) as f64;
            coverage_sum += coverage;
            maximum_coverage = maximum_coverage.max(coverage);
        }
        for prompt in request
            .layer
            .prompts
            .iter()
            .filter(|prompt| prompt.frame == frame)
        {
            validate_prompt_response(
                prompt,
                mask.as_raw(),
                info.width,
                info.height,
                bounded_authored_foreground,
            )?;
        }
    }
    let active_frames = active_end.saturating_sub(active_start) + 1;
    let mean_coverage = coverage_sum / active_frames.max(1) as f64;
    if nonempty_frames == 0 {
        bail!("segmentation worker produced an all-black mask sequence");
    }
    if maximum_coverage > 0.98 {
        bail!(
            "segmentation worker mask covers {:.2}% of a frame; this is probably background leakage",
            maximum_coverage * 100.0
        );
    }
    if result.nonempty_frames != nonempty_frames
        || result.soft_edge_pixels != soft_edge_pixels
        || (result.mean_coverage - mean_coverage).abs() > 1.0e-6
        || (result.maximum_coverage - maximum_coverage).abs() > 1.0e-6
    {
        bail!("worker mask statistics differ from result.json");
    }
    Ok(())
}

fn validate_prompt_response(
    prompt: &SegmentationPrompt,
    mask: &[u8],
    width: u32,
    height: u32,
    enforce_box_envelope: bool,
) -> Result<()> {
    let sample = |point: [f64; 2]| -> u8 {
        let x = point[0].round().clamp(0.0, width.saturating_sub(1) as f64) as usize;
        let y = point[1].round().clamp(0.0, height.saturating_sub(1) as f64) as usize;
        mask[y * width as usize + x]
    };
    let positive_survives = |point: [f64; 2]| -> bool {
        let x = point[0].round().clamp(0.0, width.saturating_sub(1) as f64) as i32;
        let y = point[1].round().clamp(0.0, height.saturating_sub(1) as f64) as i32;
        // SAM's hard seed is subsequently propagated, alpha-matted, and temporally
        // stabilized. Those lossless quality stages may move a soft edge by a few
        // pixels, so requiring one exact raster pixel to remain nonzero rejects a
        // valid matte (especially a thin web strand). Still require foreground in
        // the immediate neighborhood so a genuinely lost positive seed is rejected.
        (-4..=4).any(|dy| {
            (-4..=4).any(|dx| {
                let xx = (x + dx).clamp(0, width.saturating_sub(1) as i32) as usize;
                let yy = (y + dy).clamp(0, height.saturating_sub(1) as i32) as usize;
                mask[yy * width as usize + xx] > 4
            })
        })
    };
    if prompt
        .positive_points
        .iter()
        .any(|&point| !positive_survives(point))
    {
        bail!(
            "segmentation mask contradicts a positive prompt on frame {}",
            prompt.frame
        );
    }
    if prompt
        .negative_points
        .iter()
        .any(|&point| sample(point) >= 251)
    {
        bail!(
            "segmentation mask contradicts a negative prompt on frame {}",
            prompt.frame
        );
    }
    if enforce_box_envelope {
        let [x, y, box_width, box_height] = prompt
            .box_bounds
            .context("bounded foreground prompt is missing its box")?;
        let margin = 20.0_f64.max(0.10 * box_width.max(box_height));
        let left = (x - margin).floor().max(0.0) as usize;
        let top = (y - margin).floor().max(0.0) as usize;
        let right = (x + box_width + margin).ceil().min(f64::from(width)) as usize;
        let bottom = (y + box_height + margin).ceil().min(f64::from(height)) as usize;
        let leaked = mask.iter().enumerate().any(|(offset, &alpha)| {
            if alpha <= 8 {
                return false;
            }
            let px = offset % width as usize;
            let py = offset / width as usize;
            px < left || px >= right || py < top || py >= bottom
        });
        if leaked {
            bail!(
                "segmentation mask escapes the authored object box on frame {}",
                prompt.frame
            );
        }
    }
    Ok(())
}

fn ensure_same_file(left: &Path, right: &Path) -> Result<()> {
    let left = left
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", left.display()))?;
    let right = right
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", right.display()))?;
    if left != right {
        bail!(
            "scene source {} differs from input {}",
            left.display(),
            right.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod prompt_validation_tests {
    use super::validate_prompt_response;
    use crate::scene::{SegmentationPrompt, SpatialCoordinates};

    fn prompt(positive: [f64; 2]) -> SegmentationPrompt {
        SegmentationPrompt {
            frame: 7,
            coordinates: SpatialCoordinates::SourcePixels,
            object: None,
            concept: None,
            box_bounds: None,
            positive_points: vec![positive],
            negative_points: Vec::new(),
            polygon: Vec::new(),
            quad: None,
        }
    }

    #[test]
    fn soft_matte_may_shift_a_positive_seed_within_four_pixels() {
        let mut mask = vec![0_u8; 20 * 20];
        mask[10 * 20 + 14] = 32;
        validate_prompt_response(&prompt([10.0, 10.0]), &mask, 20, 20, false).unwrap();
    }

    #[test]
    fn a_lost_positive_seed_is_still_rejected() {
        let mask = vec![0_u8; 20 * 20];
        assert!(validate_prompt_response(&prompt([10.0, 10.0]), &mask, 20, 20, false).is_err());
    }

    #[test]
    fn authored_object_box_rejects_remote_background_leakage() {
        let mut prompt = prompt([40.0, 40.0]);
        prompt.box_bounds = Some([30.0, 30.0, 20.0, 20.0]);
        let mut mask = vec![0_u8; 100 * 100];
        mask[40 * 100 + 40] = 255;
        mask[90 * 100 + 90] = 255;
        assert!(validate_prompt_response(&prompt, &mask, 100, 100, true).is_err());
    }
}

#[cfg(test)]
mod automatic_prompt_tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        process,
        time::{SystemTime, UNIX_EPOCH},
    };

    use image::{GrayImage, Luma};

    use super::automatic_foreground_prompts;
    use crate::video::VideoInfo;

    fn seed(root: &Path, frame: usize, x: u32, y: u32) {
        let mut image = GrayImage::new(96, 64);
        for yy in y..y + 6 {
            for xx in x..x + 6 {
                image.put_pixel(xx, yy, Luma([255]));
            }
        }
        image.save(root.join(format!("{frame:06}.png"))).unwrap();
    }

    #[test]
    fn temporally_distinct_automatic_seeds_are_distinct_objects() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = PathBuf::from(format!(
            "/tmp/plaque-forge-segmentation-test-{}-{unique}",
            process::id()
        ));
        fs::create_dir(&directory).unwrap();
        seed(&directory, 1, 8, 8);
        seed(&directory, 31, 44, 26);
        let info = VideoInfo {
            width: 96,
            height: 64,
            fps: 24.0,
            fps_expression: "24/1".to_string(),
            frames: 64,
            duration_seconds: 64.0 / 24.0,
            start_time_seconds: 0.0,
            constant_frame_rate: true,
            color_range: None,
            color_space: None,
            color_transfer: None,
            color_primaries: None,
            rotation_degrees: 0,
        };
        let prompts = automatic_foreground_prompts(&directory, &info).unwrap();
        assert_eq!(prompts.len(), 2);
        assert_ne!(prompts[0].object, prompts[1].object);
        assert!(prompts.iter().all(|prompt| {
            prompt
                .object
                .as_deref()
                .is_some_and(|object| object.ends_with(&prompt.frame.to_string()))
        }));
        fs::remove_dir_all(directory).unwrap();
    }
}

#[cfg(test)]
mod foreground_fusion_tests {
    use super::{fuse_automatic_foreground_alpha, merge_automatic_and_authored_alpha};

    #[test]
    fn semantic_region_cannot_fill_a_photometric_hole() {
        let semantic = vec![u16::MAX; 11 * 5];
        let mut photometric = vec![0_u16; 11 * 5];
        photometric[2 * 11 + 1] = u16::MAX;
        photometric[2 * 11 + 9] = u16::MAX;
        let fused = fuse_automatic_foreground_alpha(&semantic, &photometric, 11, 5);
        assert_eq!(fused[2 * 11 + 5], 0);
        assert_eq!(fused[2 * 11 + 1], u16::MAX);
        assert_eq!(fused[2 * 11 + 9], u16::MAX);
    }

    #[test]
    fn photometric_alpha_is_admitted_only_near_semantic_object_support() {
        let mut semantic = vec![0_u16; 7 * 7];
        semantic[3 * 7 + 3] = u16::MAX;
        let mut photometric = vec![0_u16; 7 * 7];
        for x in 0..7 {
            photometric[3 * 7 + x] = u16::MAX;
        }
        let fused = fuse_automatic_foreground_alpha(&semantic, &photometric, 7, 7);
        assert_eq!(fused[3 * 7 + 3], u16::MAX);
        assert_eq!(fused[3 * 7 + 4], 42_598);
        assert_eq!(fused[3 * 7 + 5], 19_661);
        assert_eq!(fused[3 * 7 + 6], 0);
    }

    #[test]
    fn cast_shadow_without_semantic_object_support_is_rejected() {
        let semantic = vec![0_u16; 9];
        let mut photometric = vec![0_u16; 9];
        photometric[4] = 48_000;
        let fused = fuse_automatic_foreground_alpha(&semantic, &photometric, 3, 3);
        assert_eq!(fused[4], 0);
    }

    #[test]
    fn final_union_preserves_authored_detail_without_filling_web_holes() {
        let mut semantic_web = vec![0_u16; 9 * 3];
        let mut photometric_web = semantic_web.clone();
        semantic_web[13] = u16::MAX;
        for index in [12, 14] {
            photometric_web[index] = u16::MAX;
        }
        let automatic = fuse_automatic_foreground_alpha(&semantic_web, &photometric_web, 9, 3);
        let mut authored = vec![0_u16; 9 * 3];
        authored[17] = 48_000;

        let combined = merge_automatic_and_authored_alpha(&automatic, &authored);

        assert_eq!(combined[12], 42_598, "web thread keeps calibrated alpha");
        assert_eq!(combined[13], 0, "transparent inter-thread space stays open");
        assert_eq!(
            combined[17], 48_000,
            "authored spider detail survives fusion"
        );
    }
}

#[cfg(test)]
mod worker_process_contract_tests {
    use std::{ffi::OsString, path::Path};

    use anyhow::Result;

    use super::run_worker_process;
    use crate::infrastructure::{CommandExecutor, CommandOutput, CommandStatus};

    struct StubExecutor {
        success: bool,
    }

    impl CommandExecutor for StubExecutor {
        fn output(&self, _program: &Path, _args: &[OsString]) -> Result<CommandOutput> {
            unreachable!("worker execution uses status, not collected output")
        }

        fn status(&self, _program: &Path, args: &[OsString]) -> Result<CommandStatus> {
            assert_eq!(args[0].to_string_lossy(), "--request");
            assert_eq!(args[2].to_string_lossy(), "--output");
            Ok(CommandStatus {
                success: self.success,
                code: Some(if self.success { 0 } else { 17 }),
            })
        }
    }

    #[test]
    fn segmentation_worker_process_is_replaceable_by_the_production_contract() {
        run_worker_process(
            &StubExecutor { success: true },
            Path::new("worker"),
            Path::new("request.json"),
            Path::new("output"),
            "test",
        )
        .unwrap();

        let error = run_worker_process(
            &StubExecutor { success: false },
            Path::new("worker"),
            Path::new("request.json"),
            Path::new("output"),
            "test",
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("code=17"));
    }
}

#[cfg(test)]
mod adaptive_evidence_tests {
    use super::{
        AcceptancePolicy, SegmentationEvidence, evidence_acceptance, persistent_temporal_evidence,
    };
    use crate::scene::{LayerRole, SegmentationPrompt, SpatialCoordinates};

    const POLICY: AcceptancePolicy = AcceptancePolicy {
        min_prompt_alpha_u16: 32_768,
        max_negative_alpha_u16: 32_767,
        min_nonempty_permille: 0,
        max_foreground_coverage_permille: 980,
        max_surface_coverage_permille: 980,
        max_prompt_box_fill_permille: 800,
        min_interprompt_area_ratio_permille: 250,
        min_interprompt_area_p05_permille: 350,
        min_adjacent_iou_p05_permille: 500,
    };

    #[test]
    fn independent_evidence_accepts_a_well_behaved_foreground() {
        let evidence = SegmentationEvidence {
            minimum_prompt_alpha_u16: Some(50_000),
            maximum_negative_alpha_u16: Some(2_000),
            nonempty_permille: 700,
            maximum_coverage_permille: 220,
            maximum_prompt_box_fill_permille: Some(320),
            minimum_interprompt_area_ratio_permille: None,
            interprompt_area_ratio_p05_permille: None,
            adjacent_iou_p05_permille: None,
        };
        assert!(evidence_acceptance(&evidence, POLICY, LayerRole::Foreground).0);
    }

    #[test]
    fn independent_evidence_requests_escalation_on_prompt_loss() {
        let evidence = SegmentationEvidence {
            minimum_prompt_alpha_u16: Some(12_000),
            maximum_negative_alpha_u16: None,
            nonempty_permille: 700,
            maximum_coverage_permille: 220,
            maximum_prompt_box_fill_permille: Some(320),
            minimum_interprompt_area_ratio_permille: None,
            interprompt_area_ratio_p05_permille: None,
            adjacent_iou_p05_permille: None,
        };
        let (accepted, reasons) = evidence_acceptance(&evidence, POLICY, LayerRole::Foreground);
        assert!(!accepted);
        assert!(reasons.iter().any(|reason| reason.contains("prompt alpha")));
    }

    #[test]
    fn independent_evidence_rejects_anchor_pulses_between_prompts() {
        let evidence = SegmentationEvidence {
            minimum_prompt_alpha_u16: Some(u16::MAX),
            maximum_negative_alpha_u16: Some(0),
            nonempty_permille: 1_000,
            maximum_coverage_permille: 20,
            maximum_prompt_box_fill_permille: Some(320),
            minimum_interprompt_area_ratio_permille: Some(0),
            interprompt_area_ratio_p05_permille: Some(0),
            adjacent_iou_p05_permille: Some(0),
        };

        let (accepted, reasons) = evidence_acceptance(&evidence, POLICY, LayerRole::Foreground);

        assert!(!accepted);
        assert!(reasons.iter().any(|reason| reason.contains("inter-prompt")));
        assert!(
            reasons
                .iter()
                .any(|reason| reason.contains("adjacent mask IoU"))
        );
    }

    #[test]
    fn independent_evidence_rejects_a_filled_prompt_rectangle() {
        let evidence = SegmentationEvidence {
            minimum_prompt_alpha_u16: Some(u16::MAX),
            maximum_negative_alpha_u16: Some(0),
            nonempty_permille: 1_000,
            maximum_coverage_permille: 20,
            maximum_prompt_box_fill_permille: Some(980),
            minimum_interprompt_area_ratio_permille: None,
            interprompt_area_ratio_p05_permille: None,
            adjacent_iou_p05_permille: None,
        };

        let (accepted, reasons) = evidence_acceptance(&evidence, POLICY, LayerRole::Foreground);

        assert!(!accepted);
        assert!(
            reasons
                .iter()
                .any(|reason| reason.contains("prompt-box fill"))
        );
    }

    #[test]
    fn temporal_evidence_measures_every_inter_prompt_frame() {
        let prompt = |frame| SegmentationPrompt {
            frame,
            coordinates: SpatialCoordinates::SourcePixels,
            object: Some("spider".into()),
            concept: None,
            box_bounds: Some([0.0, 0.0, 20.0, 20.0]),
            positive_points: Vec::new(),
            negative_points: Vec::new(),
            polygon: Vec::new(),
            quad: None,
        };
        let prompts = [prompt(0), prompt(12), prompt(24)];
        let continuous = persistent_temporal_evidence(&[5_000; 25], &prompts, 0, 24, &[700; 24]);
        assert_eq!(continuous.minimum_area_ratio_permille, 1_000);
        assert_eq!(continuous.area_ratio_p05_permille, 1_000);
        assert_eq!(continuous.adjacent_iou_p05_permille, 700);

        let mut pulsing = vec![0; 25];
        pulsing[0] = 5_000;
        pulsing[12] = 5_000;
        pulsing[24] = 5_000;
        let collapsed = persistent_temporal_evidence(&pulsing, &prompts, 0, 24, &[0; 24]);
        assert_eq!(collapsed.minimum_area_ratio_permille, 0);
        assert_eq!(collapsed.area_ratio_p05_permille, 0);
        assert_eq!(collapsed.adjacent_iou_p05_permille, 0);
    }
}
