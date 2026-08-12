//! Human-reviewed scene corrections and generated refinement-artifact schemas.
//!
//! `refinement.toml` selects plaques and layers. Motion tracks and alpha sequences may
//! be generated artifacts that a person reviews rather than data they type frame by frame.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::writable_region::WritableRegion;

pub const REFINEMENT_SCHEMA_VERSION: u32 = 2;
pub const LEGACY_REFINEMENT_SCHEMA_VERSION: u32 = 1;
pub const MOTION_TRACK_SCHEMA_VERSION: u32 = 1;
pub const LAYER_ARTIFACT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy)]
pub struct PlaqueProposal {
    pub reference_frame: usize,
    pub bounds: [f64; 4],
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaqueRefinement {
    pub id: String,
    pub reference_frame: Option<usize>,
    /// Legacy rectangular tracking/writing hint. Prefer `writable_region` for non-rectangular surfaces.
    pub bounds: Option<[f64; 4]>,
    #[serde(default)]
    pub writable_region: Option<WritableRegion>,
    /// Optional source for the visual plaque. Omit for a surface already present in video.
    #[serde(default)]
    pub surface: Option<PlaqueSurface>,
    pub motion_track: Option<PathBuf>,
    /// Sparse human-authored corrections. Prefer these to editing a dense generated motion track.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub motion: Vec<MotionAnchor>,
    #[serde(default)]
    pub prompts: Vec<SegmentationPrompt>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum PlaqueSurface {
    Source,
    Injected {
        image: PathBuf,
        #[serde(default)]
        motion: InjectedMotion,
        /// [left, top, right, bottom] fractional inset used when writable_region is omitted.
        #[serde(default = "default_injected_inset")]
        inset: [f64; 4],
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum InjectedMotion {
    #[default]
    Auto,
    Screen,
    Scene,
}

fn default_injected_inset() -> [f64; 4] {
    [0.08, 0.12, 0.08, 0.12]
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SpatialCoordinates {
    SourcePixels,
    Normalized,
}

impl Default for SpatialCoordinates {
    fn default() -> Self {
        Self::SourcePixels
    }
}

fn is_source_pixels(value: &SpatialCoordinates) -> bool {
    *value == SpatialCoordinates::SourcePixels
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionAnchor {
    pub frame: usize,
    #[serde(default = "default_normalized_coordinates")]
    pub coordinates: SpatialCoordinates,
    pub quad: [[f64; 2]; 4],
    #[serde(default = "default_locked")]
    pub locked: bool,
    pub visibility: Option<f64>,
}

fn default_normalized_coordinates() -> SpatialCoordinates {
    SpatialCoordinates::Normalized
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SegmentationPrompt {
    pub frame: usize,
    /// Existing files default to source pixels; new human-authored prompts should prefer normalized.
    #[serde(default, skip_serializing_if = "is_source_pixels")]
    pub coordinates: SpatialCoordinates,
    pub object: Option<String>,
    pub box_bounds: Option<[f64; 4]>,
    #[serde(default)]
    pub positive_points: Vec<[f64; 2]>,
    #[serde(default)]
    pub negative_points: Vec<[f64; 2]>,
    #[serde(default)]
    pub polygon: Vec<[f64; 2]>,
    pub quad: Option<[[f64; 2]; 4]>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LayerRole {
    WritingSurface,
    Foreground,
    Background,
    Reflection,
    Shadow,
    Modulation,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LayerArtifactKind {
    AlphaImage,
    AlphaSequence,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LayerCoordinates {
    PlaqueCanonical,
    SourcePixels,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LayerGenerator {
    pub backend: String,
    pub model: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerArtifact {
    pub schema_version: u32,
    pub kind: LayerArtifactKind,
    pub coordinates: LayerCoordinates,
    pub path: Option<PathBuf>,
    pub pattern: Option<PathBuf>,
    pub first_frame: Option<usize>,
    pub last_frame: Option<usize>,
    #[serde(default = "default_true")]
    pub affects_layout: bool,
    pub generator: Option<LayerGenerator>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefinementLayer {
    pub id: String,
    pub role: LayerRole,
    pub plaque: String,
    pub in_front_of: Option<String>,
    pub artifact: Option<PathBuf>,
    pub active_frames: Option<[usize; 2]>,
    #[serde(default = "default_true")]
    pub affects_layout: bool,
    #[serde(default)]
    pub prompts: Vec<SegmentationPrompt>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Refinement {
    pub schema_version: u32,
    pub source: PathBuf,
    pub default_plaque: Option<String>,
    #[serde(default)]
    pub plaques: Vec<PlaqueRefinement>,
    #[serde(default)]
    pub layers: Vec<RefinementLayer>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CoordinateSystem {
    SourcePixels,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionKeyframe {
    pub frame: usize,
    pub quad: [[f64; 2]; 4],
    #[serde(default = "default_locked")]
    pub locked: bool,
    pub visibility: Option<f64>,
}

fn default_locked() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionRefinement {
    pub schema_version: u32,
    pub plaque: String,
    pub coordinates: CoordinateSystem,
    pub source_sha256: Option<String>,
    pub keyframes: Vec<MotionKeyframe>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InputFileProvenance {
    pub path: PathBuf,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct RefinementProvenance {
    pub manifest: Option<InputFileProvenance>,
    pub plaque_id: Option<String>,
    pub motion_track: Option<InputFileProvenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_asset: Option<InputFileProvenance>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layer_artifacts: Vec<InputFileProvenance>,
    pub locked_keyframes: usize,
    pub guide_keyframes: usize,
}

impl RefinementProvenance {
    pub fn content_matches(&self, other: &Self) -> bool {
        self.plaque_id == other.plaque_id
            && file_hash(&self.manifest) == file_hash(&other.manifest)
            && file_hash(&self.motion_track) == file_hash(&other.motion_track)
            && file_hash(&self.surface_asset) == file_hash(&other.surface_asset)
            && file_hash_list(&self.layer_artifacts) == file_hash_list(&other.layer_artifacts)
            && self.locked_keyframes == other.locked_keyframes
            && self.guide_keyframes == other.guide_keyframes
    }
}

fn file_hash_list(files: &[InputFileProvenance]) -> Vec<&str> {
    files
        .iter()
        .map(|file| {
            file.semantic_sha256
                .as_deref()
                .unwrap_or(file.sha256.as_str())
        })
        .collect()
}

fn file_hash(file: &Option<InputFileProvenance>) -> Option<&str> {
    file.as_ref().map(|file| {
        file.semantic_sha256
            .as_deref()
            .unwrap_or(file.sha256.as_str())
    })
}

#[derive(Debug, Clone)]
pub struct LoadedRefinement {
    pub path: PathBuf,
    pub document: Refinement,
}

impl Refinement {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read refinement {}", path.display()))?;
        let document: Self = toml::from_str(&text)
            .with_context(|| format!("failed to parse refinement {}", path.display()))?;
        document
            .validate()
            .with_context(|| format!("invalid refinement {}", path.display()))?;
        Ok(document)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != LEGACY_REFINEMENT_SCHEMA_VERSION
            && self.schema_version != REFINEMENT_SCHEMA_VERSION
        {
            bail!(
                "unsupported refinement schema {}; expected {} or legacy {}",
                self.schema_version,
                REFINEMENT_SCHEMA_VERSION,
                LEGACY_REFINEMENT_SCHEMA_VERSION
            );
        }
        require_relative(&self.source, "source")?;
        if self.plaques.is_empty() {
            bail!("refinement must declare at least one [[plaques]] entry");
        }

        let mut plaque_ids = HashSet::new();
        for plaque in &self.plaques {
            validate_id(&plaque.id, "plaque")?;
            if !plaque_ids.insert(plaque.id.as_str()) {
                bail!("duplicate plaque id {:?}", plaque.id);
            }
            if let Some(bounds) = plaque.bounds {
                validate_rect(bounds, &format!("plaque {:?} bounds", plaque.id))?;
            }
            if let Some(region) = &plaque.writable_region {
                region.validate(&format!("plaque {:?} writable_region", plaque.id))?;
            }
            if let Some(surface) = &plaque.surface {
                surface.validate(&plaque.id)?;
                if matches!(surface, PlaqueSurface::Injected { .. })
                    && plaque.tracking_bounds().is_none()
                {
                    bail!(
                        "injected plaque {:?} needs bounds or writable_region to declare its placement",
                        plaque.id
                    );
                }
            }
            if let Some(path) = &plaque.motion_track {
                require_relative(path, &format!("plaque {:?} motion_track", plaque.id))?;
            }
            if plaque.motion_track.is_some() && !plaque.motion.is_empty() {
                bail!(
                    "plaque {:?} declares both motion_track and sparse motion anchors; use one source of motion authority",
                    plaque.id
                );
            }
            for (index, anchor) in plaque.motion.iter().enumerate() {
                anchor.validate(&format!("plaque {:?} motion[{index}]", plaque.id))?;
            }
            for prompt in &plaque.prompts {
                prompt.validate(&format!("plaque {:?} prompt", plaque.id))?;
            }
        }

        if let Some(default) = &self.default_plaque
            && !plaque_ids.contains(default.as_str())
        {
            bail!(
                "default_plaque {:?} does not name a declared plaque",
                default
            );
        }

        let mut layer_ids = HashSet::new();
        for layer in &self.layers {
            validate_id(&layer.id, "layer")?;
            if !layer_ids.insert(layer.id.as_str()) {
                bail!("duplicate layer id {:?}", layer.id);
            }
            if !plaque_ids.contains(layer.plaque.as_str()) {
                bail!(
                    "layer {:?} refers to unknown plaque {:?}",
                    layer.id,
                    layer.plaque
                );
            }
            if let Some(path) = &layer.artifact {
                require_relative(path, &format!("layer {:?} artifact", layer.id))?;
            }
            if let Some([first, last]) = layer.active_frames {
                if first > last {
                    bail!("layer {:?} active_frames are reversed", layer.id);
                }
                if layer
                    .prompts
                    .iter()
                    .any(|prompt| !(first..=last).contains(&prompt.frame))
                {
                    bail!("layer {:?} has a prompt outside active_frames", layer.id);
                }
            }
            for prompt in &layer.prompts {
                prompt.validate(&format!("layer {:?} prompt", layer.id))?;
            }
        }
        Ok(())
    }

    pub fn select_plaque(&self, requested: Option<&str>) -> Result<&PlaqueRefinement> {
        let id = requested.or(self.default_plaque.as_deref());
        if let Some(id) = id {
            return self
                .plaques
                .iter()
                .find(|plaque| plaque.id == id)
                .with_context(|| format!("refinement does not declare plaque {id:?}"));
        }
        if self.plaques.len() == 1 {
            return Ok(&self.plaques[0]);
        }
        bail!("refinement declares multiple plaques; select one with --plaque <id>")
    }
}

impl PlaqueSurface {
    fn validate(&self, plaque_id: &str) -> Result<()> {
        match self {
            Self::Source => Ok(()),
            Self::Injected { image, inset, .. } => {
                require_relative(image, &format!("plaque {:?} injected image", plaque_id))?;
                if inset
                    .iter()
                    .any(|value| !value.is_finite() || !(0.0..=0.45).contains(value))
                {
                    bail!(
                        "plaque {:?} injected inset values must be finite fractions between 0 and 0.45",
                        plaque_id
                    );
                }
                if inset[0] + inset[2] >= 0.95 || inset[1] + inset[3] >= 0.95 {
                    bail!("plaque {:?} injected inset leaves no writable area", plaque_id);
                }
                Ok(())
            }
        }
    }

    pub fn injected(&self) -> Option<(&Path, InjectedMotion, [f64; 4])> {
        match self {
            Self::Injected { image, motion, inset } => Some((image.as_path(), *motion, *inset)),
            Self::Source => None,
        }
    }
}

impl PlaqueRefinement {
    /// Enclosing source-pixel rectangle used by the planar tracker. A non-rectangular
    /// writable region still tracks through its enclosing rectangle.
    pub fn tracking_bounds(&self) -> Option<[f64; 4]> {
        self.bounds
            .or_else(|| self.writable_region.as_ref().map(WritableRegion::bounds))
    }

    pub fn sparse_motion_track(
        &self,
        width: u32,
        height: u32,
        source_sha256: &str,
    ) -> Result<Option<MotionRefinement>> {
        if self.motion.is_empty() {
            return Ok(None);
        }
        let mut keyframes = self
            .motion
            .iter()
            .map(|anchor| anchor.to_keyframe(width, height))
            .collect::<Result<Vec<_>>>()?;
        keyframes.sort_by_key(|frame| frame.frame);
        if keyframes.windows(2).any(|pair| pair[0].frame == pair[1].frame) {
            bail!("plaque {:?} has duplicate sparse motion-anchor frames", self.id);
        }
        Ok(Some(MotionRefinement {
            schema_version: MOTION_TRACK_SCHEMA_VERSION,
            plaque: self.id.clone(),
            coordinates: CoordinateSystem::SourcePixels,
            source_sha256: Some(source_sha256.to_string()),
            keyframes,
        }))
    }
}

impl MotionAnchor {
    fn validate(&self, description: &str) -> Result<()> {
        if let Some(visibility) = self.visibility
            && (!visibility.is_finite() || !(0.0..=1.0).contains(&visibility))
        {
            bail!("{description} visibility must be between 0 and 1");
        }
        match self.coordinates {
            SpatialCoordinates::SourcePixels => validate_quad(self.quad, description),
            SpatialCoordinates::Normalized => {
                for (index, point) in self.quad.iter().enumerate() {
                    validate_normalized_point(*point, &format!("{description} quad[{index}]"))?;
                }
                validate_quad(self.quad, description)
            }
        }
    }

    fn to_keyframe(&self, width: u32, height: u32) -> Result<MotionKeyframe> {
        self.validate("motion anchor")?;
        let quad = match self.coordinates {
            SpatialCoordinates::SourcePixels => self.quad,
            SpatialCoordinates::Normalized => self
                .quad
                .map(|point| [point[0] * width as f64, point[1] * height as f64]),
        };
        Ok(MotionKeyframe {
            frame: self.frame,
            quad,
            locked: self.locked,
            visibility: self.visibility,
        })
    }
}

impl SegmentationPrompt {
    fn validate(&self, description: &str) -> Result<()> {
        if let Some(object) = &self.object {
            validate_id(object, &format!("{description} object"))?;
        }
        if let Some(bounds) = self.box_bounds {
            match self.coordinates {
                SpatialCoordinates::SourcePixels => {
                    validate_rect(bounds, &format!("{description} box_bounds"))?
                }
                SpatialCoordinates::Normalized => {
                    validate_normalized_rect(bounds, &format!("{description} box_bounds"))?
                }
            }
        }
        for (kind, points) in [
            ("positive_points", self.positive_points.as_slice()),
            ("negative_points", self.negative_points.as_slice()),
            ("polygon", self.polygon.as_slice()),
        ] {
            for (index, point) in points.iter().enumerate() {
                match self.coordinates {
                    SpatialCoordinates::SourcePixels => {
                        validate_point(*point, &format!("{description} {kind}[{index}]"))?
                    }
                    SpatialCoordinates::Normalized => validate_normalized_point(
                        *point,
                        &format!("{description} {kind}[{index}]"),
                    )?,
                }
            }
        }
        if !self.polygon.is_empty() && self.polygon.len() < 3 {
            bail!("{description} polygon must contain at least three points");
        }
        if let Some(quad) = self.quad {
            match self.coordinates {
                SpatialCoordinates::SourcePixels => {
                    validate_quad(quad, &format!("{description} quad"))?
                }
                SpatialCoordinates::Normalized => {
                    for (index, point) in quad.iter().enumerate() {
                        validate_normalized_point(
                            *point,
                            &format!("{description} quad[{index}]"),
                        )?;
                    }
                    validate_quad(quad, &format!("{description} quad"))?;
                }
            }
        }
        if self.box_bounds.is_none()
            && self.positive_points.is_empty()
            && self.negative_points.is_empty()
            && self.polygon.is_empty()
            && self.quad.is_none()
        {
            bail!("{description} does not contain a box, point, polygon, or quad");
        }
        Ok(())
    }

    pub fn source_pixels(&self, width: u32, height: u32) -> Result<Self> {
        self.validate("segmentation prompt")?;
        if self.coordinates == SpatialCoordinates::SourcePixels {
            return Ok(self.clone());
        }
        let point_width = width.saturating_sub(1) as f64;
        let point_height = height.saturating_sub(1) as f64;
        let scale_point = |point: [f64; 2]| [
            point[0] * point_width,
            point[1] * point_height,
        ];
        let scale_rect = |rect: [f64; 4]| [
            rect[0] * width as f64,
            rect[1] * height as f64,
            rect[2] * width as f64,
            rect[3] * height as f64,
        ];
        Ok(Self {
            frame: self.frame,
            coordinates: SpatialCoordinates::SourcePixels,
            object: self.object.clone(),
            box_bounds: self.box_bounds.map(scale_rect),
            positive_points: self
                .positive_points
                .iter()
                .copied()
                .map(scale_point)
                .collect(),
            negative_points: self
                .negative_points
                .iter()
                .copied()
                .map(scale_point)
                .collect(),
            polygon: self.polygon.iter().copied().map(scale_point).collect(),
            quad: self.quad.map(|quad| quad.map(scale_point)),
        })
    }
}

impl LayerArtifact {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read layer artifact {}", path.display()))?;
        let artifact: Self = toml::from_str(&text)
            .with_context(|| format!("failed to parse layer artifact {}", path.display()))?;
        artifact
            .validate()
            .with_context(|| format!("invalid layer artifact {}", path.display()))?;
        Ok(artifact)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != LAYER_ARTIFACT_SCHEMA_VERSION {
            bail!(
                "unsupported layer-artifact schema {}; expected {}",
                self.schema_version,
                LAYER_ARTIFACT_SCHEMA_VERSION
            );
        }
        match self.kind {
            LayerArtifactKind::AlphaImage => {
                let path = self
                    .path
                    .as_ref()
                    .context("alpha-image artifact requires path")?;
                require_relative(path, "layer artifact path")?;
                if self.pattern.is_some() || self.first_frame.is_some() || self.last_frame.is_some()
                {
                    bail!("alpha-image artifact cannot declare pattern or frame range");
                }
            }
            LayerArtifactKind::AlphaSequence => {
                if self.coordinates != LayerCoordinates::SourcePixels {
                    bail!("alpha-sequence artifacts require source-pixels coordinates");
                }
                let pattern = self
                    .pattern
                    .as_ref()
                    .context("alpha-sequence artifact requires pattern")?;
                require_relative(pattern, "layer artifact pattern")?;
                let pattern = pattern.to_string_lossy();
                if pattern.matches("%06d").count() != 1 {
                    bail!("alpha-sequence pattern must contain one %06d placeholder");
                }
                let first = self
                    .first_frame
                    .context("alpha-sequence artifact requires first_frame")?;
                let last = self
                    .last_frame
                    .context("alpha-sequence artifact requires last_frame")?;
                if first > last {
                    bail!("alpha-sequence first_frame must not exceed last_frame");
                }
                if self.path.is_some() {
                    bail!("alpha-sequence artifact cannot declare path");
                }
            }
        }
        if let Some(generator) = &self.generator
            && [
                generator.backend.as_str(),
                generator.model.as_str(),
                generator.version.as_str(),
            ]
            .iter()
            .any(|value| value.trim().is_empty())
        {
            bail!("layer artifact generator fields cannot be empty");
        }
        Ok(())
    }

    pub fn referenced_paths(&self, owner: &Path) -> Vec<PathBuf> {
        match self.kind {
            LayerArtifactKind::AlphaImage => self
                .path
                .as_ref()
                .map(|path| vec![resolve_relative(owner, path)])
                .unwrap_or_default(),
            LayerArtifactKind::AlphaSequence => {
                let Some(pattern) = &self.pattern else {
                    return Vec::new();
                };
                let pattern = pattern.to_string_lossy();
                let first = self.first_frame.unwrap_or(0);
                let last = self.last_frame.unwrap_or(first);
                (first..=last)
                    .map(|frame| {
                        resolve_relative(
                            owner,
                            Path::new(&pattern.replace("%06d", &format!("{frame:06}"))),
                        )
                    })
                    .collect()
            }
        }
    }
}

impl MotionRefinement {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read motion track {}", path.display()))?;
        let track: Self = toml::from_str(&text)
            .with_context(|| format!("failed to parse motion track {}", path.display()))?;
        track
            .validate()
            .with_context(|| format!("invalid motion track {}", path.display()))?;
        Ok(track)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != MOTION_TRACK_SCHEMA_VERSION {
            bail!(
                "unsupported motion-track schema {}; expected {}",
                self.schema_version,
                MOTION_TRACK_SCHEMA_VERSION
            );
        }
        validate_id(&self.plaque, "motion-track plaque")?;
        if let Some(hash) = &self.source_sha256
            && (hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            bail!("source_sha256 must contain 64 hexadecimal characters");
        }
        if self.keyframes.is_empty() {
            bail!("motion track contains no [[keyframes]] entries");
        }
        let mut frames = HashSet::new();
        let mut orientation = 0.0_f64;
        for keyframe in &self.keyframes {
            if !frames.insert(keyframe.frame) {
                bail!("duplicate motion keyframe {}", keyframe.frame);
            }
            validate_quad(
                keyframe.quad,
                &format!("motion keyframe {} quad", keyframe.frame),
            )?;
            let current = signed_area(keyframe.quad).signum();
            if orientation == 0.0 {
                orientation = current;
            } else if current != orientation {
                bail!(
                    "motion track changes corner winding at frame {}",
                    keyframe.frame
                );
            }
            if let Some(visibility) = keyframe.visibility
                && (!(0.0..=1.0).contains(&visibility) || !visibility.is_finite())
            {
                bail!(
                    "motion keyframe {} visibility must be in [0, 1]",
                    keyframe.frame
                );
            }
        }
        Ok(())
    }

    pub fn sorted_keyframes(&self) -> Vec<&MotionKeyframe> {
        let mut keyframes = self.keyframes.iter().collect::<Vec<_>>();
        keyframes.sort_by_key(|keyframe| keyframe.frame);
        keyframes
    }

    pub fn locked_keyframes(&self) -> usize {
        self.keyframes
            .iter()
            .filter(|keyframe| keyframe.locked)
            .count()
    }

    pub fn guide_keyframes(&self) -> usize {
        self.keyframes.len() - self.locked_keyframes()
    }

    pub fn is_dense_locked(&self, frame_count: usize) -> bool {
        self.locked_keyframes() == frame_count
            && self
                .keyframes
                .iter()
                .all(|keyframe| keyframe.frame < frame_count)
    }
}

pub fn find_refinement(input: &Path, explicit: Option<&Path>) -> Result<Option<LoadedRefinement>> {
    let path = match explicit {
        Some(path) => path.to_path_buf(),
        None => {
            let candidate = crate::workspace::refinement_path(input)?;
            if !candidate.is_file() {
                return Ok(None);
            }
            candidate
        }
    };
    if !path.is_file() {
        bail!(
            "refinement does not exist or is not a file: {}",
            path.display()
        );
    }
    let document = Refinement::load(&path)?;
    Ok(Some(LoadedRefinement { path, document }))
}

pub fn resolve_relative(owner: &Path, referenced: &Path) -> PathBuf {
    owner
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(referenced)
}

pub fn provenance(path: &Path) -> Result<InputFileProvenance> {
    Ok(InputFileProvenance {
        path: path.canonicalize().unwrap_or_else(|_| path.to_path_buf()),
        sha256: crate::digest::file_sha256(path)?,
        semantic_sha256: None,
    })
}

pub fn semantic_provenance<T: Serialize>(path: &Path, value: &T) -> Result<InputFileProvenance> {
    let mut output = provenance(path)?;
    output.semantic_sha256 = Some(crate::digest::bytes_sha256(&serde_json::to_vec(value)?));
    Ok(output)
}

pub fn layer_artifact_provenance(
    path: &Path,
    artifact: &LayerArtifact,
) -> Result<InputFileProvenance> {
    let mut output = provenance(path)?;
    let mut digest = Sha256::new();
    digest.update(serde_json::to_vec(artifact)?);
    for asset in artifact.referenced_paths(path) {
        let bytes = fs::read(&asset)
            .with_context(|| format!("failed to read layer asset {}", asset.display()))?;
        digest.update(asset.file_name().unwrap_or_default().as_encoded_bytes());
        digest.update(bytes);
    }
    output.semantic_sha256 = Some(format!("{:x}", digest.finalize()));
    Ok(output)
}

pub fn layer_artifact_path(refinement_path: &Path, layer: &RefinementLayer) -> Option<PathBuf> {
    if let Some(artifact) = &layer.artifact {
        return Some(resolve_relative(refinement_path, artifact));
    }
    if layer.prompts.is_empty() {
        return None;
    }

    // Schema-1 conventions placed generated prompted layers directly beside
    // refinement.toml. Reuse those artifacts when present, while routing new generated
    // state under artifacts/layers/ so the human-editable directory stays readable.
    let legacy = refinement_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&layer.id)
        .join("artifact.toml");
    if legacy.is_file() {
        return Some(legacy);
    }
    Some(crate::workspace::layer_path(refinement_path, &layer.id).join("artifact.toml"))
}

pub fn selected_layer_artifacts(
    refinement: &LoadedRefinement,
    plaque_id: &str,
) -> Result<Vec<(RefinementLayer, PathBuf, LayerArtifact)>> {
    refinement
        .document
        .layers
        .iter()
        .filter(|layer| layer.plaque == plaque_id)
        .filter_map(|layer| {
            layer_artifact_path(&refinement.path, layer).map(|path| {
                LayerArtifact::load(&path).map(|document| (layer.clone(), path, document))
            })
        })
        .collect()
}

pub fn current_refinement_provenance(
    input: &Path,
    explicit_refinement: Option<&Path>,
    requested_plaque: Option<&str>,
) -> Result<Option<RefinementProvenance>> {
    let loaded = find_refinement(input, explicit_refinement)?;
    let mut identity = RefinementProvenance::default();
    if let Some(loaded) = &loaded {
        let selected = loaded.document.select_plaque(requested_plaque)?;
        identity.manifest = Some(semantic_provenance(&loaded.path, &loaded.document)?);
        identity.plaque_id = Some(selected.id.clone());
        if let Some(PlaqueSurface::Injected { image, .. }) = &selected.surface {
            let path = resolve_relative(&loaded.path, image);
            identity.surface_asset = Some(provenance(&path).with_context(|| {
                format!("failed to hash injected plaque image {}", path.display())
            })?);
        }
        for (_, path, artifact) in selected_layer_artifacts(loaded, &selected.id)? {
            identity
                .layer_artifacts
                .push(layer_artifact_provenance(&path, &artifact)?);
        }
        if let Some(track) = &selected.motion_track {
            let path = resolve_relative(&loaded.path, track);
            let track = MotionRefinement::load(&path)?;
            if track.plaque != selected.id {
                bail!(
                    "motion track describes plaque {:?}, but refinement selected {:?}",
                    track.plaque,
                    selected.id
                );
            }
            identity.motion_track = Some(semantic_provenance(&path, &track)?);
            identity.locked_keyframes = track.locked_keyframes();
            identity.guide_keyframes = track.guide_keyframes();
        } else if !selected.motion.is_empty() {
            identity.locked_keyframes = selected.motion.iter().filter(|anchor| anchor.locked).count();
            identity.guide_keyframes = selected.motion.len() - identity.locked_keyframes;
        }
    } else if let Some(id) = requested_plaque {
        bail!("--plaque {id:?} requires a refinement manifest");
    }

    if identity == RefinementProvenance::default() {
        Ok(None)
    } else {
        Ok(Some(identity))
    }
}

pub fn refinement_document(
    input: &Path,
    refinement: &Path,
    detector: &str,
    proposal: Option<PlaqueProposal>,
    _alternatives: &[PlaqueProposal],
) -> Result<String> {
    let source = relative_reference(refinement, input)?;
    let mut output = format!(
        "# Human-editable Plaque Forge intent. Keep this file sparse; generated tracks/masks live elsewhere.\n\
         schema_version = {REFINEMENT_SCHEMA_VERSION}\n\
         source = {}\n\
         default_plaque = \"main\"\n\n\
         [[plaques]]\n\
         id = \"main\"\n",
        toml_string(&source.to_string_lossy())?
    );
    if let Some(proposal) = proposal {
        output.push_str(&format!(
            "# Automatic {detector} proposal, confidence {:.3}. Edit only if the preview/diagnostics are wrong.\n\
             reference_frame = {}\n\
             bounds = [{:.1}, {:.1}, {:.1}, {:.1}]\n",
            proposal.confidence.clamp(0.0, 1.0),
            proposal.reference_frame,
            proposal.bounds[0],
            proposal.bounds[1],
            proposal.bounds[2],
            proposal.bounds[3],
        ));
    } else {
        output.push_str(
            "# Automatic selection was inconclusive. Set a reference frame and enclosing bounds.\n\
             # reference_frame = 0\n\
             # bounds = [100.0, 100.0, 400.0, 200.0]\n",
        );
    }
    output.push_str(
        "\n# Sparse motion corrections may be embedded only for frames the tracker gets wrong.\n\
         # [[plaques.motion]]\n\
         # frame = 120\n\
         # coordinates = \"normalized\"\n\
         # quad = [[0.20, 0.30], [0.80, 0.30], [0.80, 0.60], [0.20, 0.60]]\n\
         # locked = true\n\n\
         # Non-rectangular writing surfaces use [plaques.writable_region]; see docs/REFINEMENTS.md.\n",
    );
    Ok(output)
}

pub fn motion_track_document(
    plaque: &str,
    source_sha256: &str,
    frames: &[(usize, [[f64; 2]; 4], f64)],
    locked: bool,
) -> Result<String> {
    validate_id(plaque, "motion-track plaque")?;
    let mut output = format!(
        "# Editable motion refinement. Quads use TL, TR, BR, BL source pixels.\n\
         schema_version = {MOTION_TRACK_SCHEMA_VERSION}\n\
         plaque = {plaque:?}\n\
         coordinates = \"source-pixels\"\n\
         source_sha256 = {source_sha256:?}\n"
    );
    for (frame, quad, visibility) in frames {
        output.push_str(&format!(
            "\n[[keyframes]]\nframe = {frame}\nquad = [\n  [{:.6}, {:.6}],\n  [{:.6}, {:.6}],\n  [{:.6}, {:.6}],\n  [{:.6}, {:.6}],\n]\nlocked = {locked}\nvisibility = {:.6}\n",
            quad[0][0],
            quad[0][1],
            quad[1][0],
            quad[1][1],
            quad[2][0],
            quad[2][1],
            quad[3][0],
            quad[3][1],
            visibility.clamp(0.0, 1.0),
        ));
    }
    let track: MotionRefinement =
        toml::from_str(&output).context("generated motion-track document is not valid TOML")?;
    track
        .validate()
        .context("generated motion-track document is invalid")?;
    Ok(output)
}

pub fn write_refinement(path: &Path, contents: &str, force: bool) -> Result<()> {
    if path.exists() && !force {
        bail!(
            "refusing to overwrite refinement {}; use --force to replace it",
            path.display()
        );
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
}

fn validate_id(id: &str, kind: &str) -> Result<()> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("{kind} id {id:?} must use letters, numbers, '-' or '_'");
    }
    Ok(())
}

fn require_relative(path: &Path, description: &str) -> Result<()> {
    if path.is_absolute() {
        bail!("{description} must be relative");
    }
    Ok(())
}

fn validate_rect(rect: [f64; 4], description: &str) -> Result<()> {
    if rect.iter().any(|value| !value.is_finite()) {
        bail!("{description} contains a non-finite coordinate");
    }
    if rect[2] <= 0.0 || rect[3] <= 0.0 {
        bail!("{description} width and height must be positive");
    }
    Ok(())
}

fn validate_point(point: [f64; 2], description: &str) -> Result<()> {
    if point.iter().any(|value| !value.is_finite()) {
        bail!("{description} contains a non-finite coordinate");
    }
    Ok(())
}

fn validate_normalized_point(point: [f64; 2], description: &str) -> Result<()> {
    if point
        .iter()
        .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    {
        bail!("{description} must contain normalized coordinates between 0 and 1");
    }
    Ok(())
}

fn validate_normalized_rect(rect: [f64; 4], description: &str) -> Result<()> {
    if rect.iter().any(|value| !value.is_finite())
        || rect[0] < 0.0
        || rect[1] < 0.0
        || rect[2] <= 0.0
        || rect[3] <= 0.0
        || rect[0] + rect[2] > 1.0 + 1.0e-9
        || rect[1] + rect[3] > 1.0 + 1.0e-9
    {
        bail!("{description} must be [x,y,width,height] normalized inside 0..1");
    }
    Ok(())
}

fn validate_quad(quad: [[f64; 2]; 4], description: &str) -> Result<()> {
    for (index, point) in quad.iter().enumerate() {
        validate_point(*point, &format!("{description}[{index}]"))?;
    }
    let mut sign = 0.0_f64;
    for index in 0..4 {
        let a = quad[index];
        let b = quad[(index + 1) % 4];
        let c = quad[(index + 2) % 4];
        let ab = [b[0] - a[0], b[1] - a[1]];
        let bc = [c[0] - b[0], c[1] - b[1]];
        if ab[0].hypot(ab[1]) < 1.0e-9 {
            bail!("{description} has a zero-length edge");
        }
        let cross = ab[0] * bc[1] - ab[1] * bc[0];
        if cross.abs() < 1.0e-12 {
            bail!("{description} has three collinear consecutive corners");
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            bail!("{description} is concave or self-intersecting");
        }
    }
    if signed_area(quad).abs() < 1.0e-12 {
        bail!("{description} has zero area");
    }
    Ok(())
}

fn signed_area(quad: [[f64; 2]; 4]) -> f64 {
    quad.iter()
        .zip(quad.iter().cycle().skip(1))
        .take(4)
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum::<f64>()
        * 0.5
}

fn toml_string(value: &str) -> Result<String> {
    Ok(toml::Value::String(value.to_string()).to_string())
}

pub(crate) fn relative_reference(owner: &Path, target: &Path) -> Result<PathBuf> {
    let owner_parent = owner.parent().unwrap_or_else(|| Path::new("."));
    if owner_parent == target.parent().unwrap_or_else(|| Path::new(".")) {
        return target
            .file_name()
            .map(PathBuf::from)
            .context("input path has no file name");
    }

    let current = std::env::current_dir().context("failed to resolve current directory")?;
    let absolute = |path: &Path| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            current.join(path)
        }
    };
    let owner = absolute(owner_parent);
    let target = target.canonicalize().unwrap_or_else(|_| absolute(target));
    let owner_components = owner.components().collect::<Vec<_>>();
    let target_components = target.components().collect::<Vec<_>>();
    let common = owner_components
        .iter()
        .zip(&target_components)
        .take_while(|(a, b)| a == b)
        .count();
    if common == 0 {
        bail!("refinement and source do not share a filesystem root");
    }
    let mut relative = PathBuf::new();
    for _ in common..owner_components.len() {
        relative.push("..");
    }
    for component in &target_components[common..] {
        relative.push(component.as_os_str());
    }
    Ok(relative)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_refinement_is_valid() {
        let text = refinement_document(
            Path::new("example.mp4"),
            Path::new("example.plaque.toml"),
            "ensemble",
            None,
            &[],
        )
        .unwrap();
        let refinement: Refinement = toml::from_str(&text).unwrap();
        refinement.validate().unwrap();
    }

    #[test]
    fn detected_proposal_keeps_the_human_manifest_short() {
        let text = refinement_document(
            Path::new("example.mp4"),
            Path::new("example.plaque.toml"),
            "ensemble",
            Some(PlaqueProposal {
                reference_frame: 51,
                bounds: [65.0, 6.0, 905.0, 487.0],
                confidence: 0.776,
            }),
            &[PlaqueProposal {
                reference_frame: 51,
                bounds: [700.0, 40.0, 300.0, 150.0],
                confidence: 0.63,
            }],
        )
        .unwrap();
        let refinement: Refinement = toml::from_str(&text).unwrap();

        refinement.validate().unwrap();
        assert_eq!(refinement.plaques.len(), 1);
        assert_eq!(refinement.plaques[0].reference_frame, Some(51));
        assert_eq!(
            refinement.plaques[0].bounds,
            Some([65.0, 6.0, 905.0, 487.0])
        );
        assert!(!text.contains("Alternative automatic candidate"));
        assert!(text.contains("[[plaques.motion]]"));
    }

    #[test]
    fn injected_surface_accepts_outer_bounds_and_inner_writable_region() {
        let refinement: Refinement = toml::from_str(
            r#"
                schema_version = 1
                source = "clip.mp4"
                default_plaque = "main"

                [[plaques]]
                id = "main"
                reference_frame = 0
                bounds = [100.0, 40.0, 500.0, 180.0]

                [plaques.writable_region]
                shape = "ellipse"
                center = [350.0, 130.0]
                radii = [210.0, 65.0]

                [plaques.surface]
                type = "injected"
                image = "plaque.png"
                motion = "screen"
            "#,
        )
        .unwrap();

        refinement.validate().unwrap();
        assert_eq!(
            refinement.plaques[0].tracking_bounds(),
            Some([100.0, 40.0, 500.0, 180.0])
        );
    }

    #[test]
    fn refinement_selects_an_explicit_plaque() {
        let refinement: Refinement = toml::from_str(
            r#"
                schema_version = 1
                source = "clip.mp4"
                default_plaque = "right"

                [[plaques]]
                id = "left"

                [[plaques]]
                id = "right"
            "#,
        )
        .unwrap();
        refinement.validate().unwrap();
        assert_eq!(refinement.select_plaque(None).unwrap().id, "right");
        assert_eq!(refinement.select_plaque(Some("left")).unwrap().id, "left");
    }

    #[test]
    fn layer_artifacts_distinguish_canonical_images_and_source_sequences() {
        let image: LayerArtifact = toml::from_str(
            r#"
                schema_version = 1
                kind = "alpha-image"
                coordinates = "plaque-canonical"
                path = "moss.png"
            "#,
        )
        .unwrap();
        let sequence: LayerArtifact = toml::from_str(
            r#"
                schema_version = 1
                kind = "alpha-sequence"
                coordinates = "source-pixels"
                pattern = "masks/%06d.png"
                first_frame = 0
                last_frame = 9
            "#,
        )
        .unwrap();

        image.validate().unwrap();
        sequence.validate().unwrap();
        assert_eq!(
            sequence.referenced_paths(Path::new("artifact.toml")).len(),
            10
        );
    }

    #[test]
    fn track_accepts_mixed_authority() {
        let track: MotionRefinement = toml::from_str(
            r#"
                schema_version = 1
                plaque = "main"
                coordinates = "source-pixels"

                [[keyframes]]
                frame = 0
                quad = [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [0.0, 1.0]]
                locked = true

                [[keyframes]]
                frame = 1
                quad = [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [0.0, 1.0]]
                locked = false
            "#,
        )
        .unwrap();
        track.validate().unwrap();
    }

    #[test]
    fn generated_motion_track_round_trips() {
        let text = motion_track_document(
            "main",
            &"a".repeat(64),
            &[(0, [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [0.0, 1.0]], 1.0)],
            false,
        )
        .unwrap();
        let track: MotionRefinement = toml::from_str(&text).unwrap();
        track.validate().unwrap();
        assert!(!track.keyframes[0].locked);
    }

    #[test]
    fn generated_motion_track_rejects_an_invalid_plaque_id() {
        let result = motion_track_document(
            "not a plaque",
            &"a".repeat(64),
            &[(0, [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [0.0, 1.0]], 1.0)],
            false,
        );

        assert!(result.is_err());
    }

    #[test]
    fn mixed_track_counts_authority() {
        let track: MotionRefinement = toml::from_str(
            r#"
                schema_version = 1
                plaque = "main"
                coordinates = "source-pixels"

                [[keyframes]]
                frame = 0
                quad = [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [0.0, 1.0]]
                locked = false

                [[keyframes]]
                frame = 1
                quad = [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [0.0, 1.0]]
                locked = true
            "#,
        )
        .unwrap();

        track.validate().unwrap();
        assert_eq!(track.guide_keyframes(), 1);
        assert_eq!(track.locked_keyframes(), 1);
    }


    #[test]
    fn schema_two_sparse_normalized_motion_resolves_to_source_pixels() {
        let refinement: Refinement = toml::from_str(
            r#"
                schema_version = 2
                source = "clip.mp4"
                default_plaque = "main"

                [[plaques]]
                id = "main"
                bounds = [10.0, 20.0, 100.0, 50.0]

                [[plaques.motion]]
                frame = 7
                coordinates = "normalized"
                quad = [[0.1, 0.2], [0.8, 0.2], [0.8, 0.7], [0.1, 0.7]]
            "#,
        )
        .unwrap();
        refinement.validate().unwrap();
        let track = refinement.plaques[0]
            .sparse_motion_track(1000, 500, &"a".repeat(64))
            .unwrap()
            .unwrap();
        assert_eq!(track.keyframes[0].frame, 7);
        assert_eq!(track.keyframes[0].quad[0], [100.0, 100.0]);
        assert_eq!(track.keyframes[0].quad[2], [800.0, 350.0]);
    }

    #[test]
    fn normalized_prompt_converts_at_worker_boundary() {
        let prompt: SegmentationPrompt = toml::from_str(
            r#"
                frame = 3
                coordinates = "normalized"
                box_bounds = [0.1, 0.2, 0.4, 0.5]
                positive_points = [[0.5, 0.25]]
            "#,
        )
        .unwrap();
        let prompt = prompt.source_pixels(1000, 400).unwrap();
        assert_eq!(prompt.box_bounds, Some([100.0, 80.0, 400.0, 200.0]));
        assert_eq!(prompt.positive_points, vec![[499.5, 99.75]]);
        assert_eq!(prompt.coordinates, SpatialCoordinates::SourcePixels);
    }

    #[test]
    fn schema_one_prompts_still_default_to_source_pixels() {
        let prompt: SegmentationPrompt = toml::from_str(
            r#"
                frame = 3
                positive_points = [[500.0, 100.0]]
            "#,
        )
        .unwrap();
        assert_eq!(prompt.coordinates, SpatialCoordinates::SourcePixels);
        prompt.validate("legacy prompt").unwrap();
    }

    #[test]
    fn cache_identity_uses_semantics_instead_of_comments() {
        let file = |path: &str, raw: &str| InputFileProvenance {
            path: path.into(),
            sha256: raw.into(),
            semantic_sha256: Some("same-semantics".into()),
        };
        let a = RefinementProvenance {
            manifest: Some(file("a.toml", "raw-a")),
            plaque_id: Some("main".into()),
            ..RefinementProvenance::default()
        };
        let b = RefinementProvenance {
            manifest: Some(file("b.toml", "raw-b")),
            plaque_id: Some("main".into()),
            ..RefinementProvenance::default()
        };
        assert!(a.content_matches(&b));
    }
}
