//! Persistent scene-analysis cache format.
//!
//! Analysis turns a source video into reusable plaque motion, masks, templates, and
//! provenance. Rendering consumes this data without repeating computer vision.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::model::{AnalysisConfidence, MotionSample, RectF};
use crate::portable_path::PortablePath;
use crate::scene::{
    DepthMode, LayerArtifactKind, LayerCoordinates, LayerGenerator, LayerMatte, LayerRole,
    SceneProvenance, SurfaceSpace,
};

pub const MANIFEST_FILE: &str = "manifest.toml";
pub const TRAJECTORY_FILE: &str = "trajectory.json";
pub const CONTENT_MASK_FILE: &str = "content-mask.png";
pub const STRUCTURAL_MASK_FILE: &str = "structural-mask.png";
pub const STRUCTURAL_TEMPLATE_FILE: &str = "structural-template.png";
pub const REGISTRATION_MASK_FILE: &str = "registration-mask.png";
pub const REGISTRATION_TEMPLATE_FILE: &str = "registration-template.png";
pub const INJECTED_SURFACE_FILE: &str = "injected-surface.png";
pub const OCCLUDER_DIR: &str = "occluder";
/// Analyzer-private photometric material not already explained by authored depth.
/// This channel exists only while automatic semantic refinement is running.
pub(crate) const AUTOMATIC_OCCLUDER_WORK_DIR: &str = ".occluder-automatic-work";
/// Analyzer-private frame-local changed material. Automatic semantic identity gates
/// this richer channel after discovery, retaining thin porous foreground detail.
pub(crate) const AUTOMATIC_MATERIAL_WORK_DIR: &str = ".occluder-material-work";
/// Analyzer-private frame-exact detail recovered around authored opaque foreground.
/// It is unioned back after automatic semantic refinement, then discarded.
pub(crate) const AUTHORED_OCCLUDER_WORK_DIR: &str = ".occluder-authored-work";
pub const LAYERS_DIR: &str = "layers";
pub const ANALYSIS_FORMAT: &str = "plaque-forge.analysis/1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AnalysisStatus {
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceInfo {
    pub path: PortablePath,
    pub sha256: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub frames: usize,
    pub duration_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerAsset {
    pub id: String,
    pub role: LayerRole,
    pub coordinates: LayerCoordinates,
    pub kind: LayerArtifactKind,
    pub affects_layout: bool,
    #[serde(default = "crate::scene::default_true")]
    pub affects_tracking: bool,
    #[serde(default)]
    pub matte: LayerMatte,
    pub path: PortablePath,
    pub first_frame: Option<usize>,
    pub last_frame: Option<usize>,
    pub generator: Option<LayerGenerator>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InjectedSurfaceAsset {
    pub path: PortablePath,
    pub source_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SegmentationConfig {
    pub backend: String,
    pub model: String,
    pub device: String,
    pub profile: String,
    pub precision: String,
    pub worker_sha256: String,
    pub runtime_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisManifest {
    pub format: String,
    pub status: AnalysisStatus,
    pub source_is_text_free: bool,
    pub analyzer_build: String,
    pub source: SourceInfo,
    pub reference_frame: usize,
    pub canonical_width: u32,
    pub canonical_height: u32,
    pub source_plaque_rect: RectF,
    pub surface_space: SurfaceSpace,
    pub trajectory_model: String,
    pub loop_closed: bool,
    pub has_occluder: bool,
    #[serde(default)]
    pub occlusion_mode: DepthMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segmentation: Option<SegmentationConfig>,
    #[serde(default)]
    pub automatic_ml_foreground: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub injected_surface: Option<InjectedSurfaceAsset>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<LayerAsset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenes: Option<SceneProvenance>,
    pub analysis_gate_passed: bool,
    pub confidence: AnalysisConfidence,
}

pub struct Analysis {
    pub root: PathBuf,
    pub manifest: AnalysisManifest,
    pub motion: Vec<MotionSample>,
}

impl Analysis {
    pub fn create(
        root: impl Into<PathBuf>,
        manifest: AnalysisManifest,
        motion: Vec<MotionSample>,
    ) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)
            .with_context(|| format!("failed to create analysis {}", root.display()))?;
        let pack = Self {
            root,
            manifest,
            motion,
        };
        pack.save_manifest()?;
        Ok(pack)
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let manifest_path = root.join(MANIFEST_FILE);
        let motion_path = root.join(TRAJECTORY_FILE);
        let manifest_text = fs::read_to_string(&manifest_path).with_context(|| {
            format!(
                "failed to read analysis manifest {}",
                manifest_path.display()
            )
        })?;
        let manifest: AnalysisManifest = toml::from_str(&manifest_text).with_context(|| {
            format!(
                "failed to parse analysis manifest {}",
                manifest_path.display()
            )
        })?;
        if manifest.format != ANALYSIS_FORMAT {
            bail!(
                "unsupported analysis format {:?}; expected {ANALYSIS_FORMAT:?}. Re-run analyze",
                manifest.format
            );
        }
        if manifest.status != AnalysisStatus::Complete {
            bail!("analysis is not complete: {}", root.display());
        }
        validate_portable_paths(&manifest)?;
        if !manifest.source_is_text_free {
            bail!("analysis source was not text-free; re-run analyze");
        }
        let motion_bytes = fs::read(&motion_path)
            .with_context(|| format!("failed to read analysis motion {}", motion_path.display()))?;
        let motion: Vec<MotionSample> =
            serde_json::from_slice(&motion_bytes).with_context(|| {
                format!("failed to parse analysis motion {}", motion_path.display())
            })?;
        if motion.len() != manifest.source.frames {
            bail!("analysis motion length does not match source frame count");
        }
        for (frame, sample) in motion.iter().enumerate() {
            if sample.frame != frame {
                bail!(
                    "analysis motion frame index {} is stored at position {frame}",
                    sample.frame
                );
            }
            sample
                .validate()
                .with_context(|| format!("invalid analysis motion sample {frame}"))?;
        }
        let pack = Self {
            root,
            manifest,
            motion,
        };
        pack.require_asset(CONTENT_MASK_FILE)?;
        pack.require_asset(STRUCTURAL_MASK_FILE)?;
        pack.require_asset(STRUCTURAL_TEMPLATE_FILE)?;
        pack.require_asset(REGISTRATION_MASK_FILE)?;
        pack.require_asset(REGISTRATION_TEMPLATE_FILE)?;
        if let Some(surface) = &pack.manifest.injected_surface {
            pack.require_asset_path(surface.path.as_path())?;
        }
        validate_layer_semantics(&pack.manifest.layers)?;
        for layer in &pack.manifest.layers {
            match layer.kind {
                LayerArtifactKind::AlphaImage => {
                    pack.require_asset_path(layer.path.as_path())?;
                }
                LayerArtifactKind::AlphaSequence => {
                    let first = layer
                        .first_frame
                        .context("layer sequence missing first frame")?;
                    let last = layer
                        .last_frame
                        .context("layer sequence missing last frame")?;
                    for frame in first..=last {
                        pack.require_asset_path(&sequence_path(layer.path.as_path(), frame))?;
                    }
                }
            }
        }
        Ok(pack)
    }

    pub fn save_manifest(&self) -> Result<()> {
        validate_portable_paths(&self.manifest)?;
        validate_layer_semantics(&self.manifest.layers)?;
        for (frame, sample) in self.motion.iter().enumerate() {
            if sample.frame != frame {
                bail!(
                    "analysis motion frame index {} is stored at position {frame}",
                    sample.frame
                );
            }
            sample
                .validate()
                .with_context(|| format!("invalid analysis motion sample {frame}"))?;
        }
        let manifest_path = self.root.join(MANIFEST_FILE);
        let motion_path = self.root.join(TRAJECTORY_FILE);
        let manifest = format!(
            "# Generated analysis cache. Regenerate with analyze.\n{}",
            toml::to_string_pretty(&self.manifest)?
        );
        fs::write(&manifest_path, manifest).with_context(|| {
            format!(
                "failed to write analysis manifest {}",
                manifest_path.display()
            )
        })?;
        fs::write(&motion_path, serde_json::to_vec_pretty(&self.motion)?).with_context(|| {
            format!("failed to write analysis motion {}", motion_path.display())
        })?;
        Ok(())
    }

    pub fn require_asset(&self, name: &str) -> Result<PathBuf> {
        self.require_asset_path(Path::new(name))
    }

    pub fn require_asset_path(&self, name: &Path) -> Result<PathBuf> {
        if name.is_absolute()
            || name
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            bail!("analysis asset path is not relative: {}", name.display());
        }
        let path = self.root.join(name);
        if !path.is_file() {
            bail!("analysis is missing required asset {}", path.display());
        }
        Ok(path)
    }

    pub fn require_current_analyzer(&self) -> Result<()> {
        if self.manifest.analyzer_build != crate::build_info::ANALYZER_CACHE_VERSION {
            bail!(
                "analysis was produced by analyzer {}; current analyzer is {}\nhelp: rebuild it explicitly with `plaque-forge analyze --force`",
                self.manifest.analyzer_build,
                crate::build_info::ANALYZER_CACHE_VERSION
            );
        }
        Ok(())
    }

    pub fn source_path(&self) -> PathBuf {
        self.manifest
            .source
            .path
            .resolve_from(&self.root.join(MANIFEST_FILE))
    }

    /// Identity of every analysis artifact that can affect the rendered pixels.
    /// Diagnostics and ML sidecars are intentionally excluded because the renderer
    /// never reads them.
    pub fn render_inputs_sha256(&self, use_occluder_masks: bool) -> Result<String> {
        let mut paths = vec![
            PathBuf::from(MANIFEST_FILE),
            PathBuf::from(TRAJECTORY_FILE),
            PathBuf::from(CONTENT_MASK_FILE),
        ];
        if let Some(surface) = &self.manifest.injected_surface {
            paths.push(surface.path.as_path().to_path_buf());
        }
        for layer in &self.manifest.layers {
            match layer.kind {
                LayerArtifactKind::AlphaImage => paths.push(layer.path.as_path().to_path_buf()),
                LayerArtifactKind::AlphaSequence => {
                    let first = layer
                        .first_frame
                        .context("layer sequence missing first frame")?;
                    let last = layer
                        .last_frame
                        .context("layer sequence missing last frame")?;
                    paths.extend(
                        (first..=last).map(|frame| sequence_path(layer.path.as_path(), frame)),
                    );
                }
            }
        }
        if use_occluder_masks {
            paths.extend(
                (0..self.manifest.source.frames)
                    .map(|frame| Path::new(OCCLUDER_DIR).join(format!("{frame:06}.png"))),
            );
        }
        crate::digest::relative_files_sha256(&self.root, paths.iter())
    }
}

fn validate_layer_semantics(layers: &[LayerAsset]) -> Result<()> {
    for layer in layers {
        layer
            .matte
            .validate(&format!("analysis layer {:?} matte", layer.id))?;
        if layer.matte.mode == crate::scene::LayerMatteMode::Opaque
            && layer.role != LayerRole::Foreground
        {
            bail!(
                "analysis layer {:?} uses opaque matte semantics but is not foreground",
                layer.id
            );
        }
    }
    Ok(())
}

fn validate_portable_paths(manifest: &AnalysisManifest) -> Result<()> {
    if let Some(surface) = &manifest.injected_surface
        && surface.path.has_parent_component()
    {
        bail!("injected-surface path escapes the analysis bundle");
    }
    if let Some(layer) = manifest
        .layers
        .iter()
        .find(|layer| layer.path.has_parent_component())
    {
        bail!("layer {:?} path escapes the analysis bundle", layer.id);
    }
    let provenance = manifest.scenes.as_ref().into_iter().flat_map(|identity| {
        identity
            .manifest
            .iter()
            .chain(identity.trajectory.iter())
            .chain(identity.surface_asset.iter())
            .chain(identity.layer_artifacts.iter())
    });
    for file in provenance {
        PortablePath::project(&file.path)
            .with_context(|| format!("provenance path is not portable: {}", file.path.display()))?;
    }
    Ok(())
}

pub fn sequence_path(pattern: &Path, frame: usize) -> PathBuf {
    PathBuf::from(
        pattern
            .to_string_lossy()
            .replace("%06d", &format!("{frame:06}")),
    )
}

/// Freshness of one bundled asset's analysis cache relative to the current build.
#[derive(Debug, PartialEq, Eq)]
pub enum CacheFreshness {
    Fresh,
    Missing,
    Invalid(String),
    StaleAnalyzer { found: String },
    SourceChanged,
}

/// One bundled asset's audit outcome.
pub struct CacheAuditEntry {
    pub stem: String,
    pub freshness: CacheFreshness,
}

/// Classify a cache against the expected analyzer identity and source bytes.
///
/// Pure so tests can pin every staleness reason without TOML fixtures; the IO
/// wrapper feeds it from `Analysis::open` and the actual source digest.
fn classify_cache_freshness(
    found_analyzer: Option<&str>,
    found_source_sha256: Option<&str>,
    actual_source_sha256: &str,
    current_analyzer: &str,
) -> CacheFreshness {
    let Some(analyzer) = found_analyzer else {
        return CacheFreshness::Missing;
    };
    if analyzer != current_analyzer {
        return CacheFreshness::StaleAnalyzer {
            found: analyzer.to_string(),
        };
    }
    match found_source_sha256 {
        Some(sha256) if sha256 == actual_source_sha256 => CacheFreshness::Fresh,
        _ => CacheFreshness::SourceChanged,
    }
}

/// Audit every `assets/*.mp4` stem's analysis cache without regenerating anything.
pub fn audit_bundled_caches(assets_dir: &Path) -> Result<Vec<CacheAuditEntry>> {
    let mut stems: Vec<String> = fs::read_dir(assets_dir)?
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("mp4"))
        .map(|path| path.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    if stems.is_empty() {
        bail!("no input videos found in {}", assets_dir.display());
    }
    stems.sort();

    let mut entries = Vec::new();
    for stem in stems {
        let freshness = match Analysis::open(assets_dir.join("analysis").join(&stem)) {
            Err(error) => CacheFreshness::Invalid(format!("{error:#}")),
            Ok(pack) => classify_cache_freshness(
                Some(&pack.manifest.analyzer_build),
                Some(&pack.manifest.source.sha256),
                crate::digest::file_sha256(&assets_dir.join(format!("{stem}.mp4")))?.as_str(),
                crate::build_info::ANALYZER_CACHE_VERSION,
            ),
        };
        entries.push(CacheAuditEntry { stem, freshness });
    }
    Ok(entries)
}

/// Read-only CI gate: reject any bundled asset whose analysis cache is missing,
/// invalid, stale, or was produced from different source bytes.
pub fn run_check_analysis_cache(assets_dir: &Path) -> Result<()> {
    let entries = audit_bundled_caches(assets_dir)?;
    let mut rejected = Vec::new();
    for entry in &entries {
        let verdict = match &entry.freshness {
            CacheFreshness::Fresh => continue,
            CacheFreshness::Missing => "missing".to_string(),
            CacheFreshness::Invalid(error) => format!("invalid: {error}"),
            CacheFreshness::StaleAnalyzer { found } => {
                format!(
                    "stale analyzer {} (current {})",
                    found,
                    crate::build_info::ANALYZER_CACHE_VERSION
                )
            }
            CacheFreshness::SourceChanged => {
                "source video differs from the analyzed bytes".to_string()
            }
        };
        println!("stale  {}  {}", entry.stem, verdict);
        rejected.push(entry.stem.clone());
    }
    if rejected.is_empty() {
        println!("all {} bundled analysis caches are fresh", entries.len());
        return Ok(());
    }
    bail!(
        "{} of {} bundled assets have stale or incomplete analysis caches\n\
         help: regenerate them on the canonical ML machine with:\n\
         \x20 ./scripts/analyze_assets.sh --force-ml {}",
        rejected.len(),
        entries.len(),
        rejected.join(" ")
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_info::ANALYZER_CACHE_VERSION as CURRENT;

    #[test]
    fn cache_freshness_classification_pins_every_staleness_reason() {
        const OLD: &str = "surface-analysis-v10-ancient";
        let source = "abc123";

        assert_eq!(
            classify_cache_freshness(Some(CURRENT), Some(source), source, CURRENT),
            CacheFreshness::Fresh
        );
        assert_eq!(
            classify_cache_freshness(Some(OLD), Some(source), source, CURRENT),
            CacheFreshness::StaleAnalyzer {
                found: OLD.to_string()
            }
        );
        assert_eq!(
            classify_cache_freshness(Some(CURRENT), Some("outdated"), source, CURRENT),
            CacheFreshness::SourceChanged
        );
        assert_eq!(
            classify_cache_freshness(Some(CURRENT), None, source, CURRENT),
            CacheFreshness::SourceChanged
        );
        assert_eq!(
            classify_cache_freshness(None, None, source, CURRENT),
            CacheFreshness::Missing
        );
    }
}
