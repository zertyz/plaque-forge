//! Boundary between the Rust application and optional ML segmentation workers.
//!
//! Rust owns validation, staging, provenance, and the versioned request/result protocol.
//! A worker is replaceable and is not trusted to mutate project data outside its output.

use std::{fs, path::Path, process::Command};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
    cli::SegmentArgs,
    refinement::{
        LayerArtifact, LayerArtifactKind, LayerCoordinates, LayerRole, Refinement, find_refinement,
        layer_artifact_path, resolve_relative,
    },
    video, workspace,
};

const WORKER_PROTOCOL_VERSION: u32 = 1;

#[derive(Serialize)]
struct WorkerRequest {
    schema_version: u32,
    backend: String,
    model: String,
    device: String,
    source: WorkerSource,
    plaque: WorkerPlaque,
    layer: WorkerLayer,
}

#[derive(Serialize)]
struct WorkerSource {
    path: std::path::PathBuf,
    sha256: String,
    width: u32,
    height: u32,
    fps: f64,
    frames: usize,
}

#[derive(Serialize)]
struct WorkerPlaque {
    id: String,
    reference_frame: Option<usize>,
    bounds: Option<[f64; 4]>,
    motion_track: Option<std::path::PathBuf>,
}

#[derive(Serialize)]
struct WorkerLayer {
    id: String,
    role: LayerRole,
    affects_layout: bool,
    active_frames: Option<[usize; 2]>,
    prompts: Vec<crate::refinement::SegmentationPrompt>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerResult {
    schema_version: u32,
    backend: String,
    model: String,
    version: String,
    frames: usize,
    mean_confidence: f64,
    minimum_confidence: f64,
}

pub fn run(args: SegmentArgs) -> Result<()> {
    if !args.input.is_file() {
        bail!("input video does not exist: {}", args.input.display());
    }
    if !args.worker.is_file() {
        bail!(
            "segmentation worker does not exist: {}",
            args.worker.display()
        );
    }
    let refinement_path = args
        .refinement
        .clone()
        .map(Ok)
        .unwrap_or_else(|| workspace::refinement_path(&args.input))?;
    let refinement = Refinement::load(&refinement_path)?;
    let declared_source = resolve_relative(&refinement_path, &refinement.source);
    ensure_same_file(&declared_source, &args.input)?;
    let plaque = refinement.select_plaque(args.plaque.as_deref())?;
    let layer = refinement
        .layers
        .iter()
        .find(|layer| layer.id == args.layer && layer.plaque == plaque.id)
        .with_context(|| {
            format!(
                "refinement does not declare layer {:?} for plaque {:?}",
                args.layer, plaque.id
            )
        })?;
    if layer.prompts.is_empty() {
        bail!("layer {:?} has no segmentation prompts", layer.id);
    }

    let info = video::probe(&args.ffprobe, &args.input)?;
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
            .map(|path| resolve_relative(&refinement_path, path))
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| workspace::layer_path(&refinement_path, &layer.id))
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
    let partial = crate::staged_output::create(&output)?;

    let request = WorkerRequest {
        schema_version: WORKER_PROTOCOL_VERSION,
        backend: args.backend.clone(),
        model: args.model.clone(),
        device: args.device.clone(),
        source: WorkerSource {
            path: args.input.canonicalize().unwrap_or(args.input.clone()),
            sha256: crate::digest::file_sha256(&args.input)?,
            width: info.width,
            height: info.height,
            fps: info.fps,
            frames: info.frames,
        },
        plaque: WorkerPlaque {
            id: plaque.id.clone(),
            reference_frame: plaque.reference_frame,
            bounds: plaque.tracking_bounds(),
            motion_track: plaque
                .motion_track
                .as_ref()
                .map(|path| resolve_relative(&refinement_path, path)),
        },
        layer: WorkerLayer {
            id: layer.id.clone(),
            role: layer.role,
            affects_layout: layer.affects_layout,
            active_frames: layer.active_frames,
            prompts: layer
                .prompts
                .iter()
                .map(|prompt| prompt.source_pixels(info.width, info.height))
                .collect::<Result<Vec<_>>>()?,
        },
    };
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
    let mut child = Command::new(&args.worker)
        .arg("--request")
        .arg(&request_path)
        .arg("--output")
        .arg(&partial)
        .spawn()
        .with_context(|| format!("failed to start worker {}", args.worker.display()))?;
    eprintln!("[ml] segmentation worker started: pid={}", child.id());
    let status = child.wait().context("failed while waiting for segmentation worker")?;
    eprintln!("[ml] segmentation worker exited: pid={}, status={status}", child.id());
    if !status.success() {
        bail!(
            "segmentation worker exited with {status}; partial output retained at {}",
            partial.display()
        );
    }
    validate_worker_output(&partial, &args.backend, &args.model, &info)?;
    crate::staged_output::remove_child(&partial, &request_path)?;
    crate::staged_output::remove_child(&partial, &partial.join("result.json"))?;
    crate::staged_output::commit(&partial, &output, args.force)?;
    println!("layer artifact: {}", output.join("artifact.toml").display());
    Ok(())
}

/// Materialize every prompted refinement layer that is still missing its artifact.
///
/// This is used by the high-level analyze workflow so a user does not need to discover
/// and invoke the lower-level `segment` command for each layer. Existing artifacts are
/// reused unless `force` is explicitly requested.
pub fn ensure_prompted_layers(
    input: &Path,
    explicit_refinement: Option<&Path>,
    plaque_id: Option<&str>,
    worker: &Path,
    backend: &str,
    model: &str,
    device: &str,
    force: bool,
    ffprobe: &Path,
) -> Result<usize> {
    let Some(loaded) = find_refinement(input, explicit_refinement)? else {
        eprintln!("[ml] segmentation skipped: no refinement manifest for {}", input.display());
        return Ok(0);
    };
    let plaque = loaded.document.select_plaque(plaque_id)?;
    let prompted = loaded
        .document
        .layers
        .iter()
        .filter(|layer| layer.plaque == plaque.id && !layer.prompts.is_empty())
        .collect::<Vec<_>>();
    if prompted.is_empty() {
        eprintln!(
            "[ml] segmentation skipped: plaque {:?} declares no prompted ML layers",
            plaque.id
        );
        return Ok(0);
    }
    let pending = prompted
        .iter()
        .filter_map(|layer| {
            let artifact = layer_artifact_path(&loaded.path, layer)?;
            (force || !artifact.is_file()).then_some(layer.id.clone())
        })
        .collect::<Vec<_>>();
    let reused = prompted.len().saturating_sub(pending.len());
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

    for layer in &pending {
        run(SegmentArgs {
            input: input.to_path_buf(),
            refinement: Some(loaded.path.clone()),
            plaque: Some(plaque.id.clone()),
            layer: layer.clone(),
            worker: worker.to_path_buf(),
            backend: backend.to_string(),
            model: model.to_string(),
            device: device.to_string(),
            output: None,
            force,
            ffprobe: ffprobe.to_path_buf(),
        })?;
    }
    Ok(pending.len())
}

fn validate_worker_output(
    root: &Path,
    backend: &str,
    model: &str,
    info: &video::VideoInfo,
) -> Result<()> {
    let result_path = root.join("result.json");
    let result: WorkerResult = serde_json::from_slice(
        &fs::read(&result_path)
            .with_context(|| format!("worker did not create {}", result_path.display()))?,
    )?;
    if result.schema_version != WORKER_PROTOCOL_VERSION
        || result.backend != backend
        || result.model != model
        || result.version.trim().is_empty()
        || result.frames != info.frames
        || ![result.mean_confidence, result.minimum_confidence]
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
    if generator.backend != backend
        || generator.model != model
        || generator.version != result.version
    {
        bail!("artifact generator provenance differs from result.json");
    }
    for path in artifact.referenced_paths(&artifact_path) {
        let image = image::open(&path)
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
            "refinement source {} differs from input {}",
            left.display(),
            right.display()
        );
    }
    Ok(())
}
