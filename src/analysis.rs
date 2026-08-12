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
use crate::refinement::{
    InjectedMotion, LayerArtifactKind, LayerCoordinates, LayerGenerator, LayerRole,
    RefinementProvenance,
};

pub const MANIFEST_FILE: &str = "manifest.toml";
pub const MOTION_FILE: &str = "motion.json";
pub const CONTENT_MASK_FILE: &str = "content-mask.png";
pub const STRUCTURAL_MASK_FILE: &str = "structural-mask.png";
pub const STRUCTURAL_TEMPLATE_FILE: &str = "structural-template.png";
pub const INJECTED_SURFACE_FILE: &str = "injected-surface.png";
pub const OCCLUDER_DIR: &str = "occluder";
pub const LAYERS_DIR: &str = "layers";
pub const ANALYSIS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AnalysisStatus {
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceInfo {
    pub path: PathBuf,
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
    pub path: PathBuf,
    pub first_frame: Option<usize>,
    pub last_frame: Option<usize>,
    pub generator: Option<LayerGenerator>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InjectedSurfaceAsset {
    pub path: PathBuf,
    pub source_sha256: String,
    pub motion: InjectedMotion,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SegmentationConfig {
    pub backend: String,
    pub model: String,
    pub device: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisManifest {
    pub schema_version: u32,
    pub status: AnalysisStatus,
    pub source_is_text_free: bool,
    pub analyzer_build: String,
    pub source: SourceInfo,
    pub reference_frame: usize,
    pub canonical_width: u32,
    pub canonical_height: u32,
    pub source_plaque_rect: RectF,
    pub motion_model: String,
    pub loop_closed: bool,
    pub has_occluder: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segmentation: Option<SegmentationConfig>,
    #[serde(default)]
    pub automatic_ml_foreground: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub injected_surface: Option<InjectedSurfaceAsset>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<LayerAsset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refinements: Option<RefinementProvenance>,
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
        let motion_path = root.join(MOTION_FILE);
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
        if manifest.schema_version != ANALYSIS_SCHEMA_VERSION {
            bail!(
                "unsupported analysis schema {}; expected {}. Re-run analyze",
                manifest.schema_version,
                ANALYSIS_SCHEMA_VERSION
            );
        }
        if manifest.status != AnalysisStatus::Complete {
            bail!("analysis is not complete: {}", root.display());
        }
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
        let pack = Self {
            root,
            manifest,
            motion,
        };
        pack.require_asset(CONTENT_MASK_FILE)?;
        pack.require_asset(STRUCTURAL_MASK_FILE)?;
        pack.require_asset(STRUCTURAL_TEMPLATE_FILE)?;
        if let Some(surface) = &pack.manifest.injected_surface {
            pack.require_asset_path(&surface.path)?;
        }
        for layer in &pack.manifest.layers {
            match layer.kind {
                LayerArtifactKind::AlphaImage => {
                    pack.require_asset_path(&layer.path)?;
                }
                LayerArtifactKind::AlphaSequence => {
                    let first = layer
                        .first_frame
                        .context("layer sequence missing first frame")?;
                    let last = layer
                        .last_frame
                        .context("layer sequence missing last frame")?;
                    for frame in first..=last {
                        pack.require_asset_path(&sequence_path(&layer.path, frame))?;
                    }
                }
            }
        }
        Ok(pack)
    }

    pub fn save_manifest(&self) -> Result<()> {
        let manifest_path = self.root.join(MANIFEST_FILE);
        let motion_path = self.root.join(MOTION_FILE);
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
        if self.manifest.source.path.is_absolute() {
            self.manifest.source.path.clone()
        } else {
            self.root.join(&self.manifest.source.path)
        }
    }
}

pub fn sequence_path(pattern: &Path, frame: usize) -> PathBuf {
    PathBuf::from(
        pattern
            .to_string_lossy()
            .replace("%06d", &format!("{frame:06}")),
    )
}
