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
    analysis::OCCLUDER_DIR,
    cli::SegmentArgs,
    model::RectF,
    scene::{
        LayerArtifact, LayerArtifactKind, LayerCoordinates, LayerRole, Scene, SegmentationPrompt,
        SpatialCoordinates, find_scene, resolve_relative,
    },
    video::{self, VideoInfo},
    workspace,
};

const WORKER_REQUEST_FORMAT: &str = "plaque-forge.segmentation-request/1";
const WORKER_RESULT_FORMAT: &str = "plaque-forge.segmentation-result/1";
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
    let worker_sha256 = worker_sha256(&args.worker)?;
    let runtime_sha256 = runtime_sha256()?;
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
        backend: args.backend.clone(),
        model: args.model.clone(),
        device: args.device.clone(),
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
        "[ml] launching segmentation worker: layer={:?}, backend={}, model={}, device={}, worker={}",
        layer.id,
        args.backend,
        args.model,
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
    crate::staged_output::remove_child(&partial, &partial.join("result.json"))?;
    staged.commit(args.force)?;
    println!("layer artifact: {}", output.join("artifact.toml").display());
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
    let expected_runtime = runtime_sha256()?;
    prune_temporary_layer_cache()?;
    let mut pending = Vec::new();
    let mut reused = 0usize;
    for layer in &prompted {
        let output = output_root.join(&layer.id);
        let artifact = output.join("artifact.toml");
        let prompt = prompt_sha256(&worker_layer(layer, info)?)?;
        let expected = || LayerCacheIdentity {
            backend,
            model,
            device,
            source_sha256,
            prompt_sha256: &prompt,
            worker_sha256: &expected_worker,
            runtime_sha256: expected_runtime.as_deref(),
        };
        if !force && layer_cache_is_current(&artifact, expected()) {
            reused += 1;
            continue;
        }
        if !force && let Some(reuse_root) = reuse_root {
            let cached = reuse_root.join(&layer.id).join("artifact.toml");
            if layer_cache_is_current(&cached, expected()) {
                copy_layer_cache(&cached, &output)?;
                reused += 1;
                continue;
            }
        }
        let temporary = temporary_layer_cache(expected());
        let temporary_artifact = temporary.join("artifact.toml");
        if !force && layer_cache_is_current(&temporary_artifact, expected()) {
            copy_layer_cache(&temporary_artifact, &output)?;
            reused += 1;
            eprintln!(
                "[ml] reused validated temporary layer cache for {:?}",
                layer.id
            );
            continue;
        }
        pending.push((layer.id.clone(), prompt));
    }
    if pending.is_empty() {
        eprintln!(
            "[ml] segmentation cache hit: {} prompted layer artifact(s) already exist; Python will not run",
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

    for (layer, prompt) in &pending {
        run_with_executor(
            SegmentArgs {
                input: input.to_path_buf(),
                scene: Some(loaded.path.clone()),
                surface: Some(plaque.id.clone()),
                layer: layer.clone(),
                worker: worker.to_path_buf(),
                backend: backend.to_string(),
                model: model.to_string(),
                device: device.to_string(),
                output: Some(output_root.join(layer)),
                force: false,
                ffprobe: ffprobe.to_path_buf(),
            },
            commands,
        )?;
        let artifact = output_root.join(layer).join("artifact.toml");
        store_temporary_layer_cache(
            &artifact,
            LayerCacheIdentity {
                backend,
                model,
                device,
                source_sha256,
                prompt_sha256: prompt,
                worker_sha256: &expected_worker,
                runtime_sha256: expected_runtime.as_deref(),
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
    let request_document = seal_request(WorkerRequest {
        format: WORKER_REQUEST_FORMAT.into(),
        backend: request.backend.to_string(),
        model: request.model.to_string(),
        device: request.device.to_string(),
        prompt_sha256: prompt_sha256(&worker_layer)?,
        worker_sha256: worker_sha256(request.worker)?,
        runtime_sha256: runtime_sha256()?,
        request_sha256: String::new(),
        source: WorkerSource {
            path: request
                .input
                .canonicalize()
                .unwrap_or_else(|_| request.input.to_path_buf()),
            sha256: crate::digest::file_sha256(request.input)?,
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
        layer: worker_layer,
    })?;

    if !request.force
        && let Some(reuse_root) = request.reuse_root
    {
        let cached = reuse_root.join("ml-foreground");
        if cached.is_dir()
            && validate_worker_output(&cached, &request_document, request.info).is_ok()
        {
            copy_automatic_foreground_cache(&cached, &output, request.info.frames)?;
            if !install_automatic_foreground_masks(request.analysis_root, request.info.frames)? {
                return Ok(false);
            }
            eprintln!(
                "[ml] automatic foreground cache hit: reused {} validated lossless mask(s)",
                request.info.frames
            );
            return Ok(true);
        }
    }

    if output.exists() {
        crate::staged_output::remove_child(request.analysis_root, &output)?;
    }
    fs::create_dir_all(&output)
        .with_context(|| format!("failed to create automatic ML output {}", output.display()))?;
    let request_path = output.join("request.json");
    fs::write(&request_path, serde_json::to_vec_pretty(&request_document)?)?;

    eprintln!(
        "[ml] automatic foreground required: {} seed frame(s), backend={}, model={}, device={}",
        request_document.layer.prompts.len(),
        request.backend,
        request.model,
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
    crate::staged_output::remove_child(&output, &request_path)?;
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
    Ok(true)
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
    // Semantic membership is not optical opacity, while a photometric residual is
    // not necessarily a solid object (a cast shadow is the common counterexample).
    // Automatic ML therefore gates a complete lossless photometric material
    // sequence. Neither source is installed on its own.
    if !destination.is_dir() {
        return Ok(false);
    }
    for frame in 0..expected_frames {
        let photometric = destination.join(format!("{frame:06}.png"));
        if !photometric.is_file() {
            bail!(
                "automatic ML foreground requires a complete photometric mask sequence; missing {}",
                photometric.display()
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
        let photometric = destination.join(format!("{frame:06}.png"));
        let installed = fuse_automatic_foreground_mask(source, &photometric, &target);
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
    let fused = fuse_automatic_foreground_alpha(
        semantic.as_raw(),
        photometric.as_raw(),
        semantic.width() as usize,
        semantic.height() as usize,
    );
    let image = ImageBuffer::<Luma<u16>, _>::from_raw(semantic.width(), semantic.height(), fused)
        .context("automatic foreground fusion produced invalid mask dimensions")?;
    image
        .save(target)
        .with_context(|| format!("failed to save fused foreground mask {}", target.display()))
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
    use super::fuse_automatic_foreground_alpha;

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
