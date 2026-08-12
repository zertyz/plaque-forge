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
use crate::refinement::{
    InjectedMotion, LayerArtifactKind, LayerCoordinates, LayerGenerator, LayerRole, OcclusionMode,
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
pub const ANALYSIS_SCHEMA_VERSION: u32 = 2;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migrated_from_analyzer: Option<String>,
    pub source: SourceInfo,
    pub reference_frame: usize,
    pub canonical_width: u32,
    pub canonical_height: u32,
    pub source_plaque_rect: RectF,
    pub motion_model: String,
    pub loop_closed: bool,
    pub has_occluder: bool,
    #[serde(default)]
    pub occlusion_mode: OcclusionMode,
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

impl AnalysisManifest {
    /// Plaque geometry and semantic controls that a serialization-only migration
    /// must never change. This intentionally excludes portable provenance paths.
    fn semantic_signature(&self) -> Result<String> {
        Ok(crate::digest::bytes_sha256(&serde_json::to_vec(
            &serde_json::json!({
                "analyzer_build": self.analyzer_build,
                "reference_frame": self.reference_frame,
                "canonical_width": self.canonical_width,
                "canonical_height": self.canonical_height,
                "source_plaque_rect": self.source_plaque_rect,
                "motion_model": self.motion_model,
                "loop_closed": self.loop_closed,
                "has_occluder": self.has_occluder,
                "occlusion_mode": self.occlusion_mode,
                "automatic_ml_foreground": self.automatic_ml_foreground,
                "injected_surface": self.injected_surface,
                "layers": self.layers,
            }),
        )?))
    }
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
        let pack = Self {
            root,
            manifest,
            motion,
        };
        pack.require_asset(CONTENT_MASK_FILE)?;
        pack.require_asset(STRUCTURAL_MASK_FILE)?;
        pack.require_asset(STRUCTURAL_TEMPLATE_FILE)?;
        if let Some(surface) = &pack.manifest.injected_surface {
            pack.require_asset_path(surface.path.as_path())?;
        }
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
        self.manifest
            .source
            .path
            .resolve_from(&self.root.join(MANIFEST_FILE))
    }
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
    let provenance = manifest
        .refinements
        .as_ref()
        .into_iter()
        .flat_map(|identity| {
            identity
                .manifest
                .iter()
                .chain(identity.motion_track.iter())
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

/// Upgrade legacy generated caches in place without repeating scene analysis or ML.
///
/// Migration is deliberately serialization-only. It may rewrite equivalent paths and
/// schema tags, but it must never claim a new analyzer identity or attach current
/// refinement provenance to geometry produced with older semantics.
pub fn migrate_tree(root: &Path, apply: bool) -> Result<()> {
    if !root.is_dir() {
        bail!("analysis root does not exist: {}", root.display());
    }
    let mut manifests = fs::read_dir(root)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path().join(MANIFEST_FILE))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    manifests.sort();
    let mut pending = 0usize;
    for path in manifests {
        if manifest_needs_migration(&path)? {
            pending += 1;
            if apply {
                migrate_manifest(&path)?;
                println!("migrated: {}", path.display());
            } else {
                println!("would migrate: {}", path.display());
            }
        } else {
            Analysis::open(path.parent().context("analysis manifest has no parent")?)
                .with_context(|| format!("analysis cache failed validation: {}", path.display()))?;
        }
    }
    if apply {
        println!("migrated {pending} analysis manifest(s)");
    } else {
        println!("{pending} analysis manifest(s) require migration; add --apply to update them");
    }
    Ok(())
}

fn manifest_needs_migration(path: &Path) -> Result<bool> {
    let source = fs::read_to_string(path)?;
    let value: toml::Value = toml::from_str(&source)?;
    let schema = value
        .get("schema_version")
        .and_then(toml::Value::as_integer)
        .unwrap_or_default();
    let build = value
        .get("analyzer_build")
        .and_then(toml::Value::as_str)
        .unwrap_or_default();
    Ok(schema != i64::from(ANALYSIS_SCHEMA_VERSION)
        || build != crate::build_info::ANALYZER_CACHE_VERSION
        || contains_absolute_path(&value))
}

fn contains_absolute_path(value: &toml::Value) -> bool {
    match value {
        toml::Value::Table(table) => table.iter().any(|(key, value)| {
            (key == "path" && value.as_str().is_some_and(is_nonportable_legacy_path))
                || contains_absolute_path(value)
        }),
        toml::Value::Array(values) => values.iter().any(contains_absolute_path),
        _ => false,
    }
}

fn is_nonportable_legacy_path(path: &str) -> bool {
    Path::new(path).is_absolute()
        || path.contains('\\')
        || path.starts_with("//")
        || path
            .as_bytes()
            .get(1)
            .is_some_and(|character| *character == b':')
}

fn migrate_manifest(path: &Path) -> Result<()> {
    let source = fs::read_to_string(path)?;
    let mut value: toml::Value = toml::from_str(&source)?;
    let original_build = value
        .get("analyzer_build")
        .and_then(toml::Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    if original_build != crate::build_info::ANALYZER_CACHE_VERSION {
        bail!(
            "analysis {} was produced by {}; current semantic analyzer is {}. Re-run analyze: migration cannot validate or bless old geometry",
            path.display(),
            original_build,
            crate::build_info::ANALYZER_CACHE_VERSION
        );
    }
    make_toml_paths_portable(&mut value, path)?;
    let table = value
        .as_table_mut()
        .context("analysis manifest root is not a TOML table")?;
    table.insert(
        "schema_version".to_string(),
        toml::Value::Integer(i64::from(ANALYSIS_SCHEMA_VERSION)),
    );
    // Preserve semantic identity exactly. A migration is not an analysis run.
    let manifest: AnalysisManifest = value
        .try_into()
        .context("legacy analysis cannot be represented by the current schema")?;
    validate_portable_paths(&manifest)?;
    let semantic_before = manifest.semantic_signature()?;

    let root = path.parent().context("analysis manifest has no parent")?;
    validate_portable_paths(&manifest)?;
    anyhow::ensure!(
        manifest.semantic_signature()? == semantic_before,
        "analysis migration attempted to change semantic geometry or depth state"
    );
    let serialized = format!(
        "# Generated analysis cache. Regenerate with analyze.\n{}",
        toml::to_string_pretty(&manifest)?
    );
    let staged_manifest = root.join(".manifest.migrate.toml");
    fs::write(&staged_manifest, serialized)?;
    fs::rename(&staged_manifest, path)?;
    Analysis::open(root).context("migrated analysis failed validation")?;
    Ok(())
}

fn make_toml_paths_portable(value: &mut toml::Value, owner: &Path) -> Result<()> {
    match value {
        toml::Value::Table(table) => {
            for (key, value) in table {
                if key == "path"
                    && let Some(path) = value.as_str()
                {
                    if Path::new(path).is_absolute() {
                        *value = toml::Value::String(
                            crate::portable_path::relative_reference(owner, Path::new(path))?
                                .to_string(),
                        );
                    } else if is_nonportable_legacy_path(path) {
                        bail!(
                            "cannot migrate a non-native legacy path automatically: {path:?}; move this cache on its original platform or rebuild analysis"
                        );
                    }
                } else {
                    make_toml_paths_portable(value, owner)?;
                }
            }
        }
        toml::Value::Array(values) => {
            for value in values {
                make_toml_paths_portable(value, owner)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_path_detection_is_platform_independent() {
        assert!(is_nonportable_legacy_path("/home/user/source.mp4"));
        assert!(is_nonportable_legacy_path(r"C:\work\source.mp4"));
        assert!(is_nonportable_legacy_path("C:/work/source.mp4"));
        assert!(is_nonportable_legacy_path(r"assets\source.mp4"));
        assert!(!is_nonportable_legacy_path("../../source.mp4"));
    }
}
