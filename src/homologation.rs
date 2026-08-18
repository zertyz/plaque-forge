//! Executable acceptance contracts for human-homologated rendered behavior.
//!
//! A homologation contract intentionally sits above implementation details. It records
//! stable scene geometry, typography constraints, and sparse source-preservation witnesses
//! for visually important foreground crossings. Rendering and segmentation algorithms may
//! change freely as long as the observable contract continues to hold.

pub mod matrix;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::{
    analysis::{Analysis, MANIFEST_FILE},
    application::HomologateRequest,
    render::{RENDER_MANIFEST_SCHEMA_VERSION, RenderManifest},
    scene::{Scene, resolve_relative},
    video::{self, Decoder},
};

/// Audit behavioral homologation coverage without rendering assets.
pub fn audit_capabilities(matrix_path: &Path) -> Result<matrix::CapabilityCoverageReport> {
    let matrix = matrix::CapabilityMatrix::load(matrix_path)?;
    Ok(matrix.report())
}

pub const HOMOLOGATION_FORMAT: &str = "plaque-forge.homologation/1";
pub const HOMOLOGATION_REPORT_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HomologationContract {
    pub format: String,
    pub asset: String,
    pub source: PathBuf,
    pub scene: PathBuf,
    pub analysis: PathBuf,
    pub surface: String,
    pub source_sha256: String,
    pub geometry: GeometryContract,
    pub render: RenderContract,
    pub typography: TypographyContract,
    #[serde(default)]
    pub source_preservation: Vec<SourcePreservationContract>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeometryContract {
    pub tracking_bounds: [f64; 4],
    pub writable_bounds: [f64; 4],
    /// Optional identity of an authored writable-region mask. This keeps a homologated
    /// irregular writing surface byte-stable without forcing it into a lossy rectangle.
    #[serde(default)]
    pub writable_mask_sha256: Option<String>,
    /// Optional identity of an authored reviewed trajectory. Automatic trackers remain
    /// implementation details; only explicitly promoted trajectory assets are pinned.
    #[serde(default)]
    pub trajectory_sha256: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderContract {
    pub text: String,
    pub style: PathBuf,
    pub style_sha256: String,
    pub font_file: String,
    pub fit_mode: String,
    pub resolved_text: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypographyContract {
    pub lines: usize,
    pub maximum_font_size: f32,
    pub maximum_clipped_pixels: u64,
    pub maximum_missing_glyphs: usize,
    pub maximum_fallback_glyphs: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcePreservationContract {
    pub frame: usize,
    pub mask: PathBuf,
    pub mask_sha256: String,
    pub minimum_selected_pixels: usize,
    pub maximum_mean_absolute_error: f64,
    pub maximum_p95_absolute_error: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourcePreservationResult {
    pub frame: usize,
    pub mask: String,
    pub mask_sha256: String,
    pub selected_pixels: usize,
    pub mean_absolute_error: f64,
    pub p95_absolute_error: f64,
    pub maximum_mean_absolute_error: f64,
    pub maximum_p95_absolute_error: f64,
    pub passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HomologationReport {
    pub schema_version: u32,
    pub contract_sha256: String,
    pub source_sha256: String,
    pub analysis_manifest_sha256: String,
    pub render_manifest_sha256: String,
    pub rendered_sha256: String,
    pub passed: bool,
    pub source_preservation: Vec<SourcePreservationResult>,
    pub failures: Vec<String>,
}

impl HomologationContract {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read homologation contract {}", path.display()))?;
        let contract: Self = toml::from_str(&text)
            .with_context(|| format!("failed to parse homologation contract {}", path.display()))?;
        contract.validate(path)?;
        Ok(contract)
    }

    fn validate(&self, path: &Path) -> Result<()> {
        ensure!(
            self.format == HOMOLOGATION_FORMAT,
            "unsupported homologation format {:?}; expected {HOMOLOGATION_FORMAT:?}",
            self.format
        );
        ensure!(
            !self.asset.trim().is_empty(),
            "homologation asset name is empty"
        );
        ensure!(
            !self.asset.contains('/') && !self.asset.contains('\\'),
            "homologation asset name must not contain path separators"
        );
        ensure!(
            !self.surface.trim().is_empty(),
            "homologation surface id is empty"
        );
        validate_sha256(&self.source_sha256, "source_sha256")?;
        validate_sha256(&self.render.style_sha256, "render.style_sha256")?;
        ensure!(
            !self.render.font_file.trim().is_empty(),
            "render.font_file must not be empty"
        );
        validate_rect(self.geometry.tracking_bounds, "geometry.tracking_bounds")?;
        validate_rect(self.geometry.writable_bounds, "geometry.writable_bounds")?;
        ensure!(
            rect_contains(self.geometry.tracking_bounds, self.geometry.writable_bounds),
            "homologated writable bounds must be contained inside tracking bounds"
        );
        if let Some(sha256) = self.geometry.writable_mask_sha256.as_deref() {
            validate_sha256(sha256, "geometry.writable_mask_sha256")?;
        }
        if let Some(sha256) = self.geometry.trajectory_sha256.as_deref() {
            validate_sha256(sha256, "geometry.trajectory_sha256")?;
        }
        ensure!(
            self.typography.lines > 0,
            "typography.lines must be positive"
        );
        ensure!(
            self.typography.maximum_font_size.is_finite()
                && self.typography.maximum_font_size > 0.0,
            "typography.maximum_font_size must be finite and positive"
        );
        let mut frames = BTreeSet::new();
        for witness in &self.source_preservation {
            validate_sha256(&witness.mask_sha256, "source_preservation.mask_sha256")?;
            ensure!(
                witness.minimum_selected_pixels > 0,
                "source-preservation frame {} must select at least one pixel",
                witness.frame
            );
            ensure!(
                witness.maximum_mean_absolute_error.is_finite()
                    && witness.maximum_mean_absolute_error >= 0.0,
                "source-preservation frame {} has invalid mean-error threshold",
                witness.frame
            );
            ensure!(
                witness.maximum_p95_absolute_error.is_finite()
                    && witness.maximum_p95_absolute_error >= 0.0,
                "source-preservation frame {} has invalid p95-error threshold",
                witness.frame
            );
            ensure!(
                frames.insert(witness.frame),
                "source-preservation frame {} is declared more than once",
                witness.frame
            );
            let mask = resolve_relative(path, &witness.mask);
            ensure!(
                mask.is_file(),
                "homologation mask is missing: {}",
                mask.display()
            );
            let mask_sha256 = crate::digest::file_sha256(&mask)?;
            ensure!(
                mask_sha256 == witness.mask_sha256,
                "homologation mask identity changed: {}",
                mask.display()
            );
            let selected_pixels = image::open(&mask)
                .with_context(|| format!("failed to load homologation mask {}", mask.display()))?
                .to_luma8()
                .as_raw()
                .iter()
                .filter(|&&alpha| alpha != 0)
                .count();
            ensure!(
                selected_pixels >= witness.minimum_selected_pixels,
                "homologation mask {} selects {selected_pixels} pixels; contract requires at least {}",
                mask.display(),
                witness.minimum_selected_pixels
            );
        }
        Ok(())
    }
}

pub(crate) fn run(
    args: HomologateRequest,
    commands: &dyn crate::infrastructure::CommandExecutor,
) -> Result<()> {
    let contract = HomologationContract::load(&args.contract)?;
    if let Some(root) = args.diagnostics.as_ref() {
        let previous = root.join(&contract.asset);
        if previous.exists() {
            fs::remove_dir_all(&previous).with_context(|| {
                format!(
                    "failed to clear stale homologation diagnostics {}",
                    previous.display()
                )
            })?;
        }
    }
    let contract_sha256 = crate::digest::file_sha256(&args.contract)?;
    let source = resolve_relative(&args.contract, &contract.source);
    let scene_path = resolve_relative(&args.contract, &contract.scene);
    let analysis_path = resolve_relative(&args.contract, &contract.analysis);
    let style_path = resolve_relative(&args.contract, &contract.render.style);

    let mut failures = Vec::new();

    let source_sha256 = crate::digest::file_sha256(&source)?;
    check_equal(
        "source SHA-256",
        &source_sha256,
        &contract.source_sha256,
        &mut failures,
    );

    let style_sha256 = crate::digest::file_sha256(&style_path)?;
    check_equal(
        "style SHA-256",
        &style_sha256,
        &contract.render.style_sha256,
        &mut failures,
    );

    check_scene_geometry(&contract, &scene_path, &mut failures)?;

    let analysis = Analysis::open(&analysis_path)?;
    check_equal(
        "analysis source SHA-256",
        &analysis.manifest.source.sha256,
        &source_sha256,
        &mut failures,
    );
    let analysis_manifest_path = analysis_path.join(MANIFEST_FILE);
    let analysis_manifest_sha256 = crate::digest::file_sha256(&analysis_manifest_path)?;

    let manifest_path = args.rendered.with_extension("render-manifest.json");
    let manifest_bytes = fs::read(&manifest_path)
        .with_context(|| format!("failed to read render manifest {}", manifest_path.display()))?;
    let manifest: RenderManifest = serde_json::from_slice(&manifest_bytes).with_context(|| {
        format!(
            "failed to parse render manifest {}",
            manifest_path.display()
        )
    })?;
    ensure!(
        manifest.schema_version == RENDER_MANIFEST_SCHEMA_VERSION,
        "unsupported render manifest schema {}; expected {}",
        manifest.schema_version,
        RENDER_MANIFEST_SCHEMA_VERSION
    );
    let render_manifest_sha256 = crate::digest::bytes_sha256(&manifest_bytes);
    let rendered_sha256 = crate::digest::file_sha256(&args.rendered)?;

    check_equal(
        "render manifest rendered SHA-256",
        &manifest.rendered_sha256,
        &rendered_sha256,
        &mut failures,
    );
    check_equal(
        "render manifest source SHA-256",
        &manifest.source_sha256,
        &source_sha256,
        &mut failures,
    );
    check_equal(
        "render manifest analysis SHA-256",
        &manifest.analysis_manifest_sha256,
        &analysis_manifest_sha256,
        &mut failures,
    );
    if let Err(error) = crate::render::load_decision_trace(&manifest_path, &manifest) {
        failures.push(format!("render decision trace is invalid: {error:#}"));
    }
    check_equal(
        "title text",
        &manifest.title_text,
        &contract.render.text,
        &mut failures,
    );
    check_equal(
        "fit mode",
        &manifest.typography.fit_mode,
        &contract.render.fit_mode,
        &mut failures,
    );
    check_equal(
        "resolved title layout",
        &manifest.typography.resolved_text,
        &contract.render.resolved_text,
        &mut failures,
    );
    let expected_style_file = style_path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    check_equal(
        "style file",
        manifest.style_file.as_deref().unwrap_or("<none>"),
        expected_style_file.as_str(),
        &mut failures,
    );
    check_equal(
        "render manifest style SHA-256",
        manifest.style_sha256.as_deref().unwrap_or("<none>"),
        style_sha256.as_str(),
        &mut failures,
    );
    check_equal(
        "font file",
        manifest.font_file.as_str(),
        contract.render.font_file.as_str(),
        &mut failures,
    );

    let metrics = &manifest.typography;
    check_limit(
        "typography line count",
        metrics.lines as f64,
        contract.typography.lines as f64,
        &mut failures,
    );
    check_maximum(
        "font size",
        metrics.font_size as f64,
        contract.typography.maximum_font_size as f64,
        &mut failures,
    );
    check_maximum(
        "clipped pixels",
        metrics.clipped_pixels as f64,
        contract.typography.maximum_clipped_pixels as f64,
        &mut failures,
    );
    check_maximum(
        "missing glyphs",
        metrics.missing_glyphs as f64,
        contract.typography.maximum_missing_glyphs as f64,
        &mut failures,
    );
    check_maximum(
        "fallback glyphs",
        metrics.fallback_glyphs as f64,
        contract.typography.maximum_fallback_glyphs as f64,
        &mut failures,
    );

    let source_preservation =
        check_source_preservation(&args, &contract, &source, &mut failures, commands)?;

    let passed = failures.is_empty();
    let report = HomologationReport {
        schema_version: HOMOLOGATION_REPORT_SCHEMA_VERSION,
        contract_sha256,
        source_sha256,
        analysis_manifest_sha256,
        render_manifest_sha256,
        rendered_sha256,
        passed,
        source_preservation,
        failures,
    };
    let json = serde_json::to_string_pretty(&report)?;
    if let Some(path) = args.report {
        crate::staged_output::write_file(&path, json.as_bytes(), true)
            .with_context(|| format!("failed to write homologation report {}", path.display()))?;
    }
    println!("{json}");
    if !report.passed {
        bail!(
            "homologation contract failed with {} requirement violation(s)",
            report.failures.len()
        );
    }
    Ok(())
}

fn check_scene_geometry(
    contract: &HomologationContract,
    scene_path: &Path,
    failures: &mut Vec<String>,
) -> Result<()> {
    let scene = Scene::load(scene_path)?;
    let Some(surface) = scene
        .surfaces
        .iter()
        .find(|surface| surface.id == contract.surface)
    else {
        failures.push(format!(
            "scene {} does not contain homologated surface {:?}",
            scene_path.display(),
            contract.surface
        ));
        return Ok(());
    };
    match surface.bounds {
        Some(bounds) if rect_approximately_equal(bounds, contract.geometry.tracking_bounds) => {}
        Some(bounds) => failures.push(format!(
            "tracking bounds changed: expected {:?}, got {:?}",
            contract.geometry.tracking_bounds, bounds
        )),
        None => failures.push("homologated surface no longer declares tracking bounds".to_string()),
    }
    match surface
        .writable_region
        .as_ref()
        .map(|region| region.bounds())
    {
        Some(bounds) if rect_approximately_equal(bounds, contract.geometry.writable_bounds) => {}
        Some(bounds) => failures.push(format!(
            "writable bounds changed: expected {:?}, got {:?}",
            contract.geometry.writable_bounds, bounds
        )),
        None => {
            failures.push("homologated surface no longer declares a writable region".to_string())
        }
    }
    if let Some(expected) = contract.geometry.writable_mask_sha256.as_deref() {
        match surface.writable_region.as_ref() {
            Some(crate::writable_region::WritableRegion::Mask { path, .. }) => {
                let path = resolve_relative(scene_path, path);
                match crate::digest::file_sha256(&path) {
                    Ok(actual) => {
                        check_equal("writable mask SHA-256", actual.as_str(), expected, failures)
                    }
                    Err(error) => failures.push(format!(
                        "failed to hash homologated writable mask {}: {error:#}",
                        path.display()
                    )),
                }
            }
            Some(_) => failures.push(
                "homologation pins a writable mask but the surface no longer uses one".to_string(),
            ),
            None => {}
        }
    }
    if let Some(expected) = contract.geometry.trajectory_sha256.as_deref() {
        match surface.trajectory.as_ref() {
            Some(path) => {
                let path = resolve_relative(scene_path, path);
                match crate::digest::file_sha256(&path) {
                    Ok(actual) => {
                        check_equal("trajectory SHA-256", actual.as_str(), expected, failures)
                    }
                    Err(error) => failures.push(format!(
                        "failed to hash homologated trajectory {}: {error:#}",
                        path.display()
                    )),
                }
            }
            None => failures.push(
                "homologation pins a reviewed trajectory but the surface no longer declares one"
                    .to_string(),
            ),
        }
    }
    Ok(())
}

fn check_source_preservation(
    args: &HomologateRequest,
    contract: &HomologationContract,
    source: &Path,
    failures: &mut Vec<String>,
    commands: &dyn crate::infrastructure::CommandExecutor,
) -> Result<Vec<SourcePreservationResult>> {
    if contract.source_preservation.is_empty() {
        return Ok(Vec::new());
    }
    let source_info = video::probe_with(commands, &args.ffprobe, source)?;
    let rendered_info = video::probe_with(commands, &args.ffprobe, &args.rendered)?;
    ensure!(
        source_info.width == rendered_info.width && source_info.height == rendered_info.height,
        "homologated render dimensions differ from the source"
    );
    ensure!(
        source_info.frames == rendered_info.frames
            && (source_info.fps - rendered_info.fps).abs() <= 1.0e-6,
        "homologated render timing differs from the source"
    );

    let mut witnesses = BTreeMap::<usize, &SourcePreservationContract>::new();
    for witness in &contract.source_preservation {
        ensure!(
            witness.frame < source_info.frames,
            "homologation frame {} lies outside {} source frames",
            witness.frame,
            source_info.frames
        );
        witnesses.insert(witness.frame, witness);
    }

    let mut source_decoder = Decoder::spawn(&args.ffmpeg, source, &source_info)?;
    let mut rendered_decoder = Decoder::spawn(&args.ffmpeg, &args.rendered, &rendered_info)?;
    let mut results = Vec::with_capacity(witnesses.len());
    for frame_index in 0..source_info.frames {
        let source_frame = source_decoder
            .next_frame()?
            .with_context(|| format!("source ended before frame {frame_index}"))?;
        let rendered_frame = rendered_decoder
            .next_frame()?
            .with_context(|| format!("render ended before frame {frame_index}"))?;
        let Some(witness) = witnesses.get(&frame_index) else {
            continue;
        };
        let mask_path = resolve_relative(&args.contract, &witness.mask);
        let mask = image::open(&mask_path)
            .with_context(|| format!("failed to load homologation mask {}", mask_path.display()))?
            .to_luma8();
        ensure!(
            mask.width() == source_info.width && mask.height() == source_info.height,
            "homologation mask {} dimensions {}x{} differ from source {}x{}",
            mask_path.display(),
            mask.width(),
            mask.height(),
            source_info.width,
            source_info.height
        );
        let mut errors = Vec::<u8>::new();
        for (pixel_index, &mask_alpha) in mask.as_raw().iter().enumerate() {
            if mask_alpha == 0 {
                continue;
            }
            let offset = pixel_index * 4;
            for channel in 0..3 {
                errors.push(
                    source_frame.pixels()[offset + channel]
                        .abs_diff(rendered_frame.pixels()[offset + channel]),
                );
            }
        }
        ensure!(
            !errors.is_empty(),
            "homologation mask {} selects no pixels",
            mask_path.display()
        );
        let selected_pixels = errors.len() / 3;
        ensure!(
            selected_pixels >= witness.minimum_selected_pixels,
            "homologation mask {} selects {selected_pixels} pixels; contract requires at least {}",
            mask_path.display(),
            witness.minimum_selected_pixels
        );
        let mean = errors.iter().map(|&value| value as f64).sum::<f64>() / errors.len() as f64;
        let p95 = percentile_u8(&mut errors, 0.95);
        let passed = mean <= witness.maximum_mean_absolute_error + f64::EPSILON
            && p95 <= witness.maximum_p95_absolute_error + f64::EPSILON;
        let diagnostics = if !passed {
            failures.push(format!(
                "frame {} source-preservation regression: mean error {:.2} (max {:.2}), p95 {:.2} (max {:.2})",
                frame_index,
                mean,
                witness.maximum_mean_absolute_error,
                p95,
                witness.maximum_p95_absolute_error
            ));
            args.diagnostics
                .as_ref()
                .map(|root| {
                    write_witness_diagnostics(
                        root,
                        &contract.asset,
                        frame_index,
                        &source_frame,
                        &rendered_frame,
                        &mask,
                        &mask_path,
                    )
                })
                .transpose()?
                .map(|path| path.to_string_lossy().into_owned())
        } else {
            None
        };
        results.push(SourcePreservationResult {
            frame: frame_index,
            mask: witness.mask.to_string_lossy().into_owned(),
            mask_sha256: witness.mask_sha256.clone(),
            selected_pixels,
            mean_absolute_error: mean,
            p95_absolute_error: p95,
            maximum_mean_absolute_error: witness.maximum_mean_absolute_error,
            maximum_p95_absolute_error: witness.maximum_p95_absolute_error,
            passed,
            diagnostics,
        });
    }
    source_decoder.finish()?;
    ensure!(
        rendered_decoder.next_frame()?.is_none(),
        "render contains frames after the source ended"
    );
    rendered_decoder.finish()?;
    ensure!(
        results.len() == witnesses.len(),
        "only {} of {} source-preservation witnesses were evaluated",
        results.len(),
        witnesses.len()
    );
    Ok(results)
}

fn write_witness_diagnostics(
    root: &Path,
    asset: &str,
    frame: usize,
    source: &crate::surface::Surface,
    rendered: &crate::surface::Surface,
    mask: &image::GrayImage,
    mask_path: &Path,
) -> Result<PathBuf> {
    let directory = root.join(asset).join(format!("frame-{frame:06}"));
    fs::create_dir_all(&directory).with_context(|| {
        format!(
            "failed to create regression diagnostics {}",
            directory.display()
        )
    })?;
    let width = source.width();
    let height = source.height();
    let source_bytes = source.pixels();
    let rendered_bytes = rendered.pixels();
    let source_image = image::RgbaImage::from_raw(width, height, source_bytes.to_vec())
        .context("invalid source diagnostic frame")?;
    let rendered_image = image::RgbaImage::from_raw(width, height, rendered_bytes.to_vec())
        .context("invalid rendered diagnostic frame")?;
    source_image.save(directory.join("source.png"))?;
    rendered_image.save(directory.join("rendered.png"))?;

    let mut diff = image::RgbaImage::new(width, height);
    let mut witness = rendered_image.clone();
    for (index, pixel) in diff.pixels_mut().enumerate() {
        let offset = index * 4;
        *pixel = image::Rgba([
            source_bytes[offset]
                .abs_diff(rendered_bytes[offset])
                .saturating_mul(3),
            source_bytes[offset + 1]
                .abs_diff(rendered_bytes[offset + 1])
                .saturating_mul(3),
            source_bytes[offset + 2]
                .abs_diff(rendered_bytes[offset + 2])
                .saturating_mul(3),
            255,
        ]);
        if mask.as_raw()[index] != 0 {
            let target = witness.get_pixel_mut(index as u32 % width, index as u32 / width);
            target.0[0] = (target.0[0] / 2).saturating_add(127);
            target.0[1] /= 2;
            target.0[2] /= 2;
        }
    }
    diff.save(directory.join("diff-3x.png"))?;
    witness.save(directory.join("witness-overlay.png"))?;
    fs::copy(mask_path, directory.join("witness-mask.png"))
        .with_context(|| format!("failed to copy witness mask {}", mask_path.display()))?;
    Ok(directory)
}

fn check_equal<T>(description: &str, actual: &T, expected: &T, failures: &mut Vec<String>)
where
    T: PartialEq + std::fmt::Display + ?Sized,
{
    if actual != expected {
        failures.push(format!(
            "{description} changed: expected {expected}, got {actual}"
        ));
    }
}

fn check_limit(description: &str, actual: f64, expected: f64, failures: &mut Vec<String>) {
    if (actual - expected).abs() > f64::EPSILON {
        failures.push(format!(
            "{description} changed: expected {expected:.3}, got {actual:.3}"
        ));
    }
}

fn check_maximum(description: &str, actual: f64, maximum: f64, failures: &mut Vec<String>) {
    if actual > maximum + f64::EPSILON {
        failures.push(format!(
            "{description} {actual:.3} exceeds homologated maximum {maximum:.3}"
        ));
    }
}

fn percentile_u8(values: &mut [u8], percentile: f64) -> f64 {
    values.sort_unstable();
    let index = ((values.len().saturating_sub(1)) as f64 * percentile)
        .round()
        .clamp(0.0, values.len().saturating_sub(1) as f64) as usize;
    values[index] as f64
}

fn validate_sha256(value: &str, description: &str) -> Result<()> {
    ensure!(
        value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "{description} must contain 64 hexadecimal characters"
    );
    Ok(())
}

fn validate_rect(rect: [f64; 4], description: &str) -> Result<()> {
    ensure!(
        rect.iter().all(|value| value.is_finite()),
        "{description} contains a non-finite coordinate"
    );
    ensure!(
        rect[2] > 0.0 && rect[3] > 0.0,
        "{description} width and height must be positive"
    );
    Ok(())
}

fn rect_contains(outer: [f64; 4], inner: [f64; 4]) -> bool {
    const EPSILON: f64 = 1.0e-6;
    inner[0] + EPSILON >= outer[0]
        && inner[1] + EPSILON >= outer[1]
        && inner[0] + inner[2] <= outer[0] + outer[2] + EPSILON
        && inner[1] + inner[3] <= outer[1] + outer[3] + EPSILON
}

fn rect_approximately_equal(left: [f64; 4], right: [f64; 4]) -> bool {
    left.into_iter()
        .zip(right)
        .all(|(left, right)| (left - right).abs() <= 1.0e-6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_uses_the_requested_rank() {
        let mut values = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        assert_eq!(percentile_u8(&mut values, 0.95), 10.0);
    }

    #[test]
    fn rectangle_containment_is_inclusive() {
        assert!(rect_contains(
            [10.0, 20.0, 100.0, 50.0],
            [10.0, 20.0, 100.0, 50.0]
        ));
        assert!(!rect_contains(
            [10.0, 20.0, 100.0, 50.0],
            [9.0, 20.0, 100.0, 50.0]
        ));
    }

    #[test]
    fn failed_witness_diagnostics_are_self_contained() {
        let root = std::env::temp_dir().join(format!(
            "plaque-forge-homologation-diagnostics-{}",
            std::process::id()
        ));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(&root).unwrap();

        let source =
            crate::surface::Surface::from_rgba(2, 1, vec![10, 20, 30, 255, 40, 50, 60, 255])
                .unwrap();
        let rendered =
            crate::surface::Surface::from_rgba(2, 1, vec![20, 10, 30, 255, 40, 80, 60, 255])
                .unwrap();
        let mask = image::GrayImage::from_raw(2, 1, vec![255, 0]).unwrap();
        let mask_path = root.join("mask.png");
        mask.save(&mask_path).unwrap();

        let directory =
            write_witness_diagnostics(&root, "asset", 7, &source, &rendered, &mask, &mask_path)
                .unwrap();
        for name in [
            "source.png",
            "rendered.png",
            "diff-3x.png",
            "witness-overlay.png",
            "witness-mask.png",
        ] {
            assert!(directory.join(name).is_file(), "missing diagnostic {name}");
        }
        fs::remove_dir_all(&root).unwrap();
    }
}
