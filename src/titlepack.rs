use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::metadata::{
    HumanInputProvenance, LayerArtifactKind, LayerCoordinates, LayerGenerator, LayerRole,
};
use crate::model::{AnalysisConfidence, MotionSample, RectF};

pub const MANIFEST_FILE: &str = "manifest.toml";
pub const MOTION_FILE: &str = "motion.json";
pub const CONTENT_MASK_FILE: &str = "content-mask.png";
pub const STRUCTURAL_MASK_FILE: &str = "structural-mask.png";
pub const STRUCTURAL_TEMPLATE_FILE: &str = "structural-template.png";
pub const OCCLUDER_DIR: &str = "occluder";
pub const LAYERS_DIR: &str = "layers";
pub const CURRENT_FORMAT_VERSION: u32 = 6;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PackStatus {
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
pub struct TitlePackManifest {
    pub format_version: u32,
    pub status: PackStatus,
    /// Milestone 3 assumes the production source has a text-free plaque cavity.
    pub source_is_text_free: bool,
    #[serde(default = "unknown_build")]
    pub analyzer_build: String,
    pub source: SourceInfo,
    pub reference_frame: usize,
    pub canonical_width: u32,
    pub canonical_height: u32,
    pub source_plaque_rect: RectF,
    pub motion_model: String,
    pub loop_closed: bool,
    pub has_occluder: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<LayerAsset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_inputs: Option<HumanInputProvenance>,
    #[serde(default = "gate_passed_by_default")]
    pub analysis_gate_passed: bool,
    pub confidence: AnalysisConfidence,
}

fn gate_passed_by_default() -> bool {
    false
}

fn unknown_build() -> String {
    "unknown".to_string()
}

pub struct TitlePack {
    pub root: PathBuf,
    pub manifest: TitlePackManifest,
    pub motion: Vec<MotionSample>,
}

impl TitlePack {
    pub fn create(
        root: impl Into<PathBuf>,
        manifest: TitlePackManifest,
        motion: Vec<MotionSample>,
    ) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)
            .with_context(|| format!("failed to create title-pack {}", root.display()))?;
        let pack = Self {
            root,
            manifest,
            motion,
        };
        pack.save_metadata()?;
        Ok(pack)
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let manifest_path = root.join(MANIFEST_FILE);
        let motion_path = root.join(MOTION_FILE);
        let manifest_text = fs::read_to_string(&manifest_path).with_context(|| {
            format!(
                "failed to read title-pack manifest {}",
                manifest_path.display()
            )
        })?;
        let manifest: TitlePackManifest = toml::from_str(&manifest_text).with_context(|| {
            format!(
                "failed to parse title-pack manifest {}",
                manifest_path.display()
            )
        })?;
        if manifest.format_version != CURRENT_FORMAT_VERSION {
            bail!(
                "unsupported title-pack format {}; expected {}. Re-run analyze with this version",
                manifest.format_version,
                CURRENT_FORMAT_VERSION
            );
        }
        if manifest.status != PackStatus::Complete {
            bail!("title-pack is not complete: {}", root.display());
        }
        if !manifest.source_is_text_free {
            bail!(
                "title-pack was not analyzed under the text-free plaque contract; re-run analyze"
            );
        }
        let motion_bytes = fs::read(&motion_path).with_context(|| {
            format!("failed to read title-pack motion {}", motion_path.display())
        })?;
        let motion: Vec<MotionSample> =
            serde_json::from_slice(&motion_bytes).with_context(|| {
                format!(
                    "failed to parse title-pack motion {}",
                    motion_path.display()
                )
            })?;
        if motion.len() != manifest.source.frames {
            bail!("title-pack motion length does not match source frame count");
        }
        let pack = Self {
            root,
            manifest,
            motion,
        };
        pack.require_asset(CONTENT_MASK_FILE)?;
        pack.require_asset(STRUCTURAL_MASK_FILE)?;
        pack.require_asset(STRUCTURAL_TEMPLATE_FILE)?;
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

    pub fn save_metadata(&self) -> Result<()> {
        let manifest_path = self.root.join(MANIFEST_FILE);
        let motion_path = self.root.join(MOTION_FILE);
        let manifest = format!(
            "# Generated title-pack metadata. Do not edit; regenerate it with analyze.\n{}",
            toml::to_string_pretty(&self.manifest)?
        );
        fs::write(&manifest_path, manifest).with_context(|| {
            format!(
                "failed to write title-pack manifest {}",
                manifest_path.display()
            )
        })?;
        fs::write(&motion_path, serde_json::to_vec_pretty(&self.motion)?).with_context(|| {
            format!(
                "failed to write title-pack motion {}",
                motion_path.display()
            )
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
            bail!("title-pack asset path is not relative: {}", name.display());
        }
        let path = self.root.join(name);
        if !path.is_file() {
            bail!("title-pack is missing required asset {}", path.display());
        }
        Ok(path)
    }

    pub fn require_current_analyzer(&self) -> Result<()> {
        if self.manifest.analyzer_build != crate::build_info::SOURCE_FINGERPRINT {
            bail!(
                "title-pack was analyzed by source build {}; current build is {}\nhelp: re-run analyze or use replace --reanalyze",
                self.manifest.analyzer_build,
                crate::build_info::SOURCE_FINGERPRINT
            );
        }
        Ok(())
    }
}

pub fn sequence_path(pattern: &Path, frame: usize) -> PathBuf {
    PathBuf::from(
        pattern
            .to_string_lossy()
            .replace("%06d", &format!("{frame:06}")),
    )
}

pub fn is_titlepack(path: &Path) -> bool {
    let manifest_path = path.join(MANIFEST_FILE);
    let Ok(text) = fs::read_to_string(manifest_path) else {
        return false;
    };
    let Ok(manifest) = toml::from_str::<TitlePackManifest>(&text) else {
        return false;
    };
    manifest.format_version == CURRENT_FORMAT_VERSION
        && manifest.status == PackStatus::Complete
        && manifest.source_is_text_free
        && manifest.analyzer_build == crate::build_info::SOURCE_FINGERPRINT
        && path.join(MOTION_FILE).is_file()
        && path.join(CONTENT_MASK_FILE).is_file()
        && path.join(STRUCTURAL_MASK_FILE).is_file()
        && path.join(STRUCTURAL_TEMPLATE_FILE).is_file()
}

#[cfg(test)]
mod tests {
    use super::TitlePackManifest;

    #[test]
    fn format_five_manifest_defaults_new_provenance_fields() {
        let manifest: TitlePackManifest = toml::from_str(
            r#"
                format_version = 5
                status = "complete"
                source_is_text_free = true
                analyzer_build = "build"
                reference_frame = 0
                canonical_width = 10
                canonical_height = 5
                motion_model = "automatic"
                loop_closed = false
                has_occluder = false
                analysis_gate_passed = true

                [source]
                path = "clip.mp4"
                sha256 = "source"
                width = 100
                height = 50
                fps = 24.0
                frames = 2
                duration_seconds = 0.083333

                [source_plaque_rect]
                x = 0.0
                y = 0.0
                width = 10.0
                height = 5.0

                [human_inputs]
                plaque_id = "main"
                locked_keyframes = 0
                guide_keyframes = 0

                [confidence]
                plaque_detection = 0.9
                motion = 0.9
                extraction = 0.9
                occlusion = 0.9
                overall = 0.9
            "#,
        )
        .unwrap();
        let provenance = manifest.human_inputs.unwrap();

        assert_eq!(provenance.plaque_hint, None);
        assert_eq!(provenance.track_csv, None);
    }
}
