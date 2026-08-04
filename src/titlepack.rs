use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::model::{AnalysisConfidence, MotionSample, RectF};

pub const MANIFEST_FILE: &str = "manifest.toml";
pub const MOTION_FILE: &str = "motion.json";
pub const CONTENT_MASK_FILE: &str = "content-mask.png";
pub const STRUCTURAL_MASK_FILE: &str = "structural-mask.png";
pub const STRUCTURAL_TEMPLATE_FILE: &str = "structural-template.png";
pub const OCCLUDER_DIR: &str = "occluder";
pub const CURRENT_FORMAT_VERSION: u32 = 3;

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
        Ok(pack)
    }

    pub fn save_metadata(&self) -> Result<()> {
        let manifest_path = self.root.join(MANIFEST_FILE);
        let motion_path = self.root.join(MOTION_FILE);
        fs::write(&manifest_path, toml::to_string_pretty(&self.manifest)?).with_context(|| {
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

    pub fn asset(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    pub fn require_asset(&self, name: &str) -> Result<PathBuf> {
        let path = self.asset(name);
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
