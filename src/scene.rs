//! Authored scene intent and generated scene-artifact schemas.
//!
//! `scene.toml` describes artistic intent. Generated trajectories, masks, and
//! verification evidence are separate artifacts and can never certify themselves.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::writable_region::WritableRegion;

pub const SCENE_FORMAT: &str = "plaque-forge.scene/1";
pub const TRAJECTORY_FORMAT: &str = "plaque-forge.trajectory/1";
pub const LAYER_ARTIFACT_FORMAT: &str = "plaque-forge.layer/1";

#[derive(Debug, Clone, Copy)]
pub struct SurfaceProposal {
    pub reference_frame: usize,
    pub bounds: [f64; 4],
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TitleSurface {
    pub id: String,
    /// Coordinate space in which the title exists. A physical surface is never
    /// allowed to become screen-fixed merely because measurement is difficult.
    pub space: SurfaceSpace,
    pub reference_frame: Option<usize>,
    /// Enclosing source-pixel tracking hint. The writable shape remains separate.
    pub bounds: Option<[f64; 4]>,
    #[serde(default)]
    pub writable_region: Option<WritableRegion>,
    #[serde(default)]
    pub appearance: SurfaceAppearance,
    pub trajectory: Option<PathBuf>,
    /// Sparse reviewed measurements. They constrain the solver but do not bypass
    /// independent verification.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<SurfaceAnchor>,
    #[serde(default)]
    pub prompts: Vec<SegmentationPrompt>,
    #[serde(default)]
    pub depth: DepthMode,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DepthMode {
    #[default]
    Automatic,
    /// A deliberately flat canvas with no scene depth.
    Flat,
    /// Use only explicitly declared layers.
    DeclaredOnly,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum SurfaceAppearance {
    #[default]
    Observed,
    Image {
        image: PathBuf,
        /// [left, top, right, bottom] fractional inset used when writable_region is omitted.
        #[serde(default = "default_injected_inset")]
        inset: [f64; 4],
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SurfaceSpace {
    #[default]
    ScenePlane,
    SceneMesh,
    ScreenCanvas,
}

fn default_injected_inset() -> [f64; 4] {
    [0.08, 0.12, 0.08, 0.12]
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SpatialCoordinates {
    #[default]
    SourcePixels,
    Normalized,
}

fn is_source_pixels(value: &SpatialCoordinates) -> bool {
    *value == SpatialCoordinates::SourcePixels
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceAnchor {
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

/// How a layer mask should be interpreted by compositing and geometric consumers.
///
/// `Optical` preserves measured alpha. `Opaque` treats the mask as semantic
/// confidence for an opaque foreground and calibrates it into solid occlusion
/// with a narrow soft edge.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LayerMatteMode {
    #[default]
    Optical,
    Opaque,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct LayerMatte {
    pub mode: LayerMatteMode,
    pub support_threshold: f64,
    pub solid_threshold: f64,
}

impl Default for LayerMatte {
    fn default() -> Self {
        Self {
            mode: LayerMatteMode::Optical,
            support_threshold: 0.03,
            solid_threshold: 0.20,
        }
    }
}

impl LayerMatte {
    pub(crate) fn validate(&self, description: &str) -> Result<()> {
        if !self.support_threshold.is_finite()
            || !self.solid_threshold.is_finite()
            || !(0.0..1.0).contains(&self.support_threshold)
            || !(0.0..=1.0).contains(&self.solid_threshold)
            || self.support_threshold >= self.solid_threshold
        {
            bail!(
                "{description} thresholds must satisfy 0 <= support_threshold < solid_threshold <= 1"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LayerGenerator {
    pub backend: String,
    pub model: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerArtifact {
    pub format: String,
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

pub(crate) fn default_true() -> bool {
    true
}

fn validate_optional_sha256(value: Option<&str>, name: &str, required: bool) -> Result<()> {
    let Some(value) = value else {
        if required {
            bail!("generated layer artifact requires {name} provenance");
        }
        return Ok(());
    };
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("{name} must contain 64 hexadecimal characters");
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneLayer {
    pub id: String,
    pub role: LayerRole,
    pub surface: String,
    pub in_front_of: Option<String>,
    pub artifact: Option<PathBuf>,
    pub active_frames: Option<[usize; 2]>,
    #[serde(default = "default_true")]
    pub affects_layout: bool,
    /// Whether this layer may influence plaque tracking. Render-only foreground
    /// evidence can disable this so compositing improvements cannot perturb an
    /// already homologated motion solution.
    #[serde(default = "default_true")]
    pub affects_tracking: bool,
    #[serde(default)]
    pub matte: LayerMatte,
    #[serde(default)]
    pub prompts: Vec<SegmentationPrompt>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub format: String,
    pub source: PathBuf,
    pub default_surface: Option<String>,
    #[serde(default)]
    pub surfaces: Vec<TitleSurface>,
    #[serde(default)]
    pub layers: Vec<SceneLayer>,
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
pub struct SurfaceTrajectory {
    pub format: String,
    pub surface: String,
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
pub struct SceneProvenance {
    pub manifest: Option<InputFileProvenance>,
    pub surface_id: Option<String>,
    pub trajectory: Option<InputFileProvenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_asset: Option<InputFileProvenance>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layer_artifacts: Vec<InputFileProvenance>,
    pub locked_keyframes: usize,
    pub guide_keyframes: usize,
}

impl SceneProvenance {
    pub fn content_matches(&self, other: &Self) -> bool {
        self.surface_id == other.surface_id
            && optional_file_matches(&self.manifest, &other.manifest)
            && optional_file_matches(&self.trajectory, &other.trajectory)
            && optional_file_matches(&self.surface_asset, &other.surface_asset)
            && file_list_matches(&self.layer_artifacts, &other.layer_artifacts)
            && self.locked_keyframes == other.locked_keyframes
            && self.guide_keyframes == other.guide_keyframes
    }

    pub fn portable_for(&self, owner: &Path) -> Result<Self> {
        fn portable(
            file: &Option<InputFileProvenance>,
            owner: &Path,
        ) -> Result<Option<InputFileProvenance>> {
            file.as_ref()
                .map(|file| {
                    let mut output = file.clone();
                    output.path = PathBuf::from(
                        crate::portable_path::relative_reference(owner, &file.path)?.to_string(),
                    );
                    Ok(output)
                })
                .transpose()
        }

        Ok(Self {
            manifest: portable(&self.manifest, owner)?,
            surface_id: self.surface_id.clone(),
            trajectory: portable(&self.trajectory, owner)?,
            surface_asset: portable(&self.surface_asset, owner)?,
            layer_artifacts: self
                .layer_artifacts
                .iter()
                .map(|file| {
                    let mut output = file.clone();
                    output.path = PathBuf::from(
                        crate::portable_path::relative_reference(owner, &file.path)?.to_string(),
                    );
                    Ok(output)
                })
                .collect::<Result<Vec<_>>>()?,
            locked_keyframes: self.locked_keyframes,
            guide_keyframes: self.guide_keyframes,
        })
    }
}

fn file_matches(a: &InputFileProvenance, b: &InputFileProvenance) -> bool {
    a.sha256 == b.sha256
        || a.semantic_sha256
            .as_ref()
            .zip(b.semantic_sha256.as_ref())
            .is_some_and(|(a, b)| a == b)
}

fn optional_file_matches(
    a: &Option<InputFileProvenance>,
    b: &Option<InputFileProvenance>,
) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => file_matches(a, b),
        _ => false,
    }
}

fn file_list_matches(a: &[InputFileProvenance], b: &[InputFileProvenance]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| file_matches(a, b))
}

#[derive(Debug, Clone)]
pub struct LoadedScene {
    pub path: PathBuf,
    pub document: Scene,
}

impl Scene {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read scene {}", path.display()))?;
        let document: Self = toml::from_str(&text)
            .with_context(|| format!("failed to parse scene {}", path.display()))?;
        document
            .validate()
            .with_context(|| format!("invalid scene {}", path.display()))?;
        Ok(document)
    }

    pub fn validate(&self) -> Result<()> {
        if self.format != SCENE_FORMAT {
            bail!(
                "unsupported scene format {:?}; expected {SCENE_FORMAT:?}",
                self.format
            );
        }
        require_relative(&self.source, "source")?;
        if self.surfaces.is_empty() {
            bail!("scene must declare at least one [[surfaces]] entry");
        }

        let mut surface_ids = HashSet::new();
        for surface in &self.surfaces {
            validate_id(&surface.id, "surface")?;
            if !surface_ids.insert(surface.id.as_str()) {
                bail!("duplicate surface id {:?}", surface.id);
            }
            if let Some(bounds) = surface.bounds {
                validate_rect(bounds, &format!("surface {:?} bounds", surface.id))?;
            }
            if let Some(region) = &surface.writable_region {
                region.validate(&format!("surface {:?} writable_region", surface.id))?;
                if let Some(bounds) = surface.bounds {
                    let writable_bounds = region.bounds();
                    if !rect_contains(bounds, writable_bounds) {
                        bail!(
                            "surface {:?} writable_region {:?} must be fully contained inside surface bounds {:?}",
                            surface.id,
                            writable_bounds,
                            bounds
                        );
                    }
                }
            }
            surface.appearance.validate(&surface.id)?;
            if matches!(surface.appearance, SurfaceAppearance::Image { .. })
                && surface.tracking_bounds().is_none()
            {
                bail!(
                    "image surface {:?} needs bounds or writable_region",
                    surface.id
                );
            }
            if surface.space == SurfaceSpace::ScreenCanvas && surface.depth != DepthMode::Flat {
                bail!(
                    "screen-canvas surface {:?} must use depth = \"flat\"",
                    surface.id
                );
            }
            if surface.space == SurfaceSpace::SceneMesh {
                bail!(
                    "scene-mesh surface {:?} is not supported until a deformable mesh solver and independent verifier exist",
                    surface.id
                );
            }
            if let Some(path) = &surface.trajectory {
                require_relative(path, &format!("surface {:?} trajectory", surface.id))?;
            }
            if surface.trajectory.is_some() && !surface.anchors.is_empty() {
                bail!(
                    "surface {:?} declares both trajectory and sparse anchors",
                    surface.id
                );
            }
            for (index, anchor) in surface.anchors.iter().enumerate() {
                anchor.validate(&format!("surface {:?} anchors[{index}]", surface.id))?;
            }
            for prompt in &surface.prompts {
                prompt.validate(&format!("surface {:?} prompt", surface.id))?;
            }
        }

        if let Some(default) = &self.default_surface
            && !surface_ids.contains(default.as_str())
        {
            bail!(
                "default_surface {:?} does not name a declared plaque",
                default
            );
        }

        let mut layer_ids = HashSet::new();
        for layer in &self.layers {
            validate_id(&layer.id, "layer")?;
            if !layer_ids.insert(layer.id.as_str()) {
                bail!("duplicate layer id {:?}", layer.id);
            }
            if !surface_ids.contains(layer.surface.as_str()) {
                bail!(
                    "layer {:?} refers to unknown surface {:?}",
                    layer.id,
                    layer.surface
                );
            }
            if let Some(target) = &layer.in_front_of
                && !surface_ids.contains(target.as_str())
            {
                bail!(
                    "layer {:?} in_front_of refers to unknown plaque {:?}",
                    layer.id,
                    target
                );
            }
            if let Some(path) = &layer.artifact {
                require_relative(path, &format!("layer {:?} artifact", layer.id))?;
            }
            layer
                .matte
                .validate(&format!("layer {:?} matte", layer.id))?;
            if layer.matte.mode == LayerMatteMode::Opaque && layer.role != LayerRole::Foreground {
                bail!(
                    "layer {:?} uses matte mode opaque, which is only valid for foreground layers",
                    layer.id
                );
            }
            if layer.artifact.is_some() && !layer.prompts.is_empty() {
                bail!(
                    "layer {:?} cannot declare both a generated prompt and an artifact",
                    layer.id
                );
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

    pub fn select_surface(&self, requested: Option<&str>) -> Result<&TitleSurface> {
        let id = requested.or(self.default_surface.as_deref());
        if let Some(id) = id {
            return self
                .surfaces
                .iter()
                .find(|surface| surface.id == id)
                .with_context(|| format!("scene does not declare surface {id:?}"));
        }
        if self.surfaces.len() == 1 {
            return Ok(&self.surfaces[0]);
        }
        bail!("scene declares multiple surfaces; select one with --surface <id>")
    }
}

impl SurfaceAppearance {
    fn validate(&self, surface_id: &str) -> Result<()> {
        match self {
            Self::Observed => Ok(()),
            Self::Image { image, inset } => {
                require_relative(image, &format!("surface {:?} image", surface_id))?;
                if inset
                    .iter()
                    .any(|value| !value.is_finite() || !(0.0..=0.45).contains(value))
                {
                    bail!(
                        "plaque {:?} injected inset values must be finite fractions between 0 and 0.45",
                        surface_id
                    );
                }
                if inset[0] + inset[2] >= 0.95 || inset[1] + inset[3] >= 0.95 {
                    bail!(
                        "plaque {:?} injected inset leaves no writable area",
                        surface_id
                    );
                }
                Ok(())
            }
        }
    }

    pub fn image(&self) -> Option<(&Path, [f64; 4])> {
        match self {
            Self::Image { image, inset } => Some((image.as_path(), *inset)),
            Self::Observed => None,
        }
    }
}

impl TitleSurface {
    /// Enclosing source-pixel rectangle used by the planar tracker. A non-rectangular
    /// writable region still tracks through its enclosing rectangle.
    pub fn tracking_bounds(&self) -> Option<[f64; 4]> {
        self.bounds
            .or_else(|| self.writable_region.as_ref().map(WritableRegion::bounds))
    }

    pub fn sparse_trajectory(
        &self,
        width: u32,
        height: u32,
        source_sha256: &str,
    ) -> Result<Option<SurfaceTrajectory>> {
        if self.anchors.is_empty() {
            return Ok(None);
        }
        let mut keyframes = self
            .anchors
            .iter()
            .map(|anchor| anchor.to_keyframe(width, height))
            .collect::<Result<Vec<_>>>()?;
        keyframes.sort_by_key(|frame| frame.frame);
        if keyframes
            .windows(2)
            .any(|pair| pair[0].frame == pair[1].frame)
        {
            bail!(
                "plaque {:?} has duplicate sparse motion-anchor frames",
                self.id
            );
        }
        Ok(Some(SurfaceTrajectory {
            format: TRAJECTORY_FORMAT.to_string(),
            surface: self.id.clone(),
            coordinates: CoordinateSystem::SourcePixels,
            source_sha256: Some(source_sha256.to_string()),
            keyframes,
        }))
    }
}

impl SurfaceAnchor {
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
                        validate_normalized_point(*point, &format!("{description} quad[{index}]"))?;
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
        let scale_point = |point: [f64; 2]| [point[0] * point_width, point[1] * point_height];
        let scale_rect = |rect: [f64; 4]| {
            [
                rect[0] * width as f64,
                rect[1] * height as f64,
                rect[2] * width as f64,
                rect[3] * height as f64,
            ]
        };
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
        if self.format != LAYER_ARTIFACT_FORMAT {
            bail!(
                "unsupported layer-artifact format {:?}; expected {LAYER_ARTIFACT_FORMAT:?}",
                self.format
            );
        }
        match self.kind {
            LayerArtifactKind::AlphaImage => {
                let path = self
                    .path
                    .as_ref()
                    .context("alpha-image artifact requires path")?;
                crate::portable_path::PortablePath::bundle(path)
                    .context("invalid layer artifact path")?;
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
                crate::portable_path::PortablePath::bundle(pattern)
                    .context("invalid layer artifact pattern")?;
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
        if let Some(generator) = &self.generator {
            // LayerArtifact also represents reviewed/authored artifacts whose
            // generator metadata may describe provenance without being a live ML
            // cache record.  Strict worker/cache provenance (device plus sealed
            // source/prompt/worker/runtime/request hashes) is enforced by the
            // segmentation boundary when worker output is accepted or reused.
            if generator
                .requested_device
                .as_deref()
                .is_some_and(|device| device.trim().is_empty())
            {
                bail!("layer artifact requested_device cannot be empty");
            }
            for (name, value) in [
                ("source_sha256", generator.source_sha256.as_deref()),
                ("prompt_sha256", generator.prompt_sha256.as_deref()),
                ("worker_sha256", generator.worker_sha256.as_deref()),
                ("request_sha256", generator.request_sha256.as_deref()),
            ] {
                validate_optional_sha256(value, name, false)?;
            }
            validate_optional_sha256(generator.runtime_sha256.as_deref(), "runtime_sha256", false)?;
        }
        Ok(())
    }

    /// Validate the sealed identity required when this artifact is a reusable
    /// machine-generated cache rather than reviewed/static scene data.
    pub fn validate_generated_provenance(&self) -> Result<()> {
        let generator = self
            .generator
            .as_ref()
            .context("generated layer artifact is missing generator provenance")?;
        if generator
            .requested_device
            .as_deref()
            .is_none_or(|device| device.trim().is_empty())
        {
            bail!("generated layer artifact requires requested_device provenance");
        }
        for (name, value) in [
            ("source_sha256", generator.source_sha256.as_deref()),
            ("prompt_sha256", generator.prompt_sha256.as_deref()),
            ("worker_sha256", generator.worker_sha256.as_deref()),
            ("request_sha256", generator.request_sha256.as_deref()),
        ] {
            validate_optional_sha256(value, name, true)?;
        }
        validate_optional_sha256(generator.runtime_sha256.as_deref(), "runtime_sha256", false)?;
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

impl SurfaceTrajectory {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read trajectory {}", path.display()))?;
        let track: Self = toml::from_str(&text)
            .with_context(|| format!("failed to parse trajectory {}", path.display()))?;
        track
            .validate()
            .with_context(|| format!("invalid trajectory {}", path.display()))?;
        Ok(track)
    }

    pub fn validate(&self) -> Result<()> {
        if self.format != TRAJECTORY_FORMAT {
            bail!(
                "unsupported trajectory format {:?}; expected {TRAJECTORY_FORMAT:?}",
                self.format
            );
        }
        validate_id(&self.surface, "trajectory surface")?;
        if let Some(hash) = &self.source_sha256
            && (hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            bail!("source_sha256 must contain 64 hexadecimal characters");
        }
        if self.keyframes.is_empty() {
            bail!("trajectory contains no [[keyframes]] entries");
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
                    "trajectory changes corner winding at frame {}",
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

pub fn find_scene(input: &Path, explicit: Option<&Path>) -> Result<Option<LoadedScene>> {
    let path = match explicit {
        Some(path) => path.to_path_buf(),
        None => {
            let candidate = crate::workspace::scene_path(input)?;
            if !candidate.is_file() {
                return Ok(None);
            }
            candidate
        }
    };
    if !path.is_file() {
        bail!("scene does not exist or is not a file: {}", path.display());
    }
    let document = Scene::load(&path)?;
    Ok(Some(LoadedScene { path, document }))
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
    output.semantic_sha256 = Some(digest
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>());
    Ok(output)
}

pub fn layer_artifact_path(scene_path: &Path, layer: &SceneLayer) -> Option<PathBuf> {
    if let Some(artifact) = &layer.artifact {
        return Some(resolve_relative(scene_path, artifact));
    }
    if layer.prompts.is_empty() {
        return None;
    }

    Some(crate::workspace::layer_path(scene_path, &layer.id).join("artifact.toml"))
}

pub fn selected_layer_artifacts(
    scene: &LoadedScene,
    surface_id: &str,
) -> Result<Vec<(SceneLayer, PathBuf, LayerArtifact)>> {
    scene
        .document
        .layers
        .iter()
        .filter(|layer| layer.surface == surface_id)
        .filter_map(|layer| {
            layer_artifact_path(&scene.path, layer).map(|path| {
                LayerArtifact::load(&path).map(|document| (layer.clone(), path, document))
            })
        })
        .collect()
}

pub fn current_scene_provenance(
    input: &Path,
    explicit_scene: Option<&Path>,
    requested_surface: Option<&str>,
) -> Result<Option<SceneProvenance>> {
    let loaded = find_scene(input, explicit_scene)?;
    let mut identity = SceneProvenance::default();
    if let Some(loaded) = &loaded {
        let selected = loaded.document.select_surface(requested_surface)?;
        identity.manifest = Some(semantic_provenance(&loaded.path, &loaded.document)?);
        identity.surface_id = Some(selected.id.clone());
        if let SurfaceAppearance::Image { image, .. } = &selected.appearance {
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
        if let Some(track) = &selected.trajectory {
            let path = resolve_relative(&loaded.path, track);
            let track = SurfaceTrajectory::load(&path)?;
            if track.surface != selected.id {
                bail!(
                    "trajectory describes surface {:?}, but scene selected {:?}",
                    track.surface,
                    selected.id
                );
            }
            identity.trajectory = Some(semantic_provenance(&path, &track)?);
            identity.locked_keyframes = track.locked_keyframes();
            identity.guide_keyframes = track.guide_keyframes();
        } else if !selected.anchors.is_empty() {
            identity.locked_keyframes = selected
                .anchors
                .iter()
                .filter(|anchor| anchor.locked)
                .count();
            identity.guide_keyframes = selected.anchors.len() - identity.locked_keyframes;
        }
    } else if let Some(id) = requested_surface {
        bail!("--surface {id:?} requires a scene manifest");
    }

    if identity == SceneProvenance::default() {
        Ok(None)
    } else {
        Ok(Some(identity))
    }
}

pub fn scene_document(
    input: &Path,
    scene: &Path,
    detector: &str,
    proposal: Option<SurfaceProposal>,
    _alternatives: &[SurfaceProposal],
) -> Result<String> {
    let source = relative_reference(scene, input)?;
    let mut output = format!(
        "# Plaque Forge scene intent. Generated evidence lives in the analysis cache.\n\
         format = {SCENE_FORMAT:?}\n\
         source = {}\n\
         default_surface = \"main\"\n\n\
         [[surfaces]]\n\
         id = \"main\"\n\
         space = \"scene-plane\"\n\
         depth = \"automatic\"\n",
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
        "\n# Sparse reviewed measurements constrain analysis but never bypass verification.\n\
         # [[surfaces.anchors]]\n\
         # frame = 120\n\
         # coordinates = \"normalized\"\n\
         # quad = [[0.20, 0.30], [0.80, 0.30], [0.80, 0.60], [0.20, 0.60]]\n\
         # locked = true\n\n\
         # Non-rectangular writing surfaces use [surfaces.writable_region].\n",
    );
    Ok(output)
}

pub fn trajectory_document(
    surface: &str,
    source_sha256: &str,
    frames: &[(usize, [[f64; 2]; 4], f64)],
    locked: bool,
) -> Result<String> {
    validate_id(surface, "trajectory surface")?;
    let mut output = format!(
        "# Reviewed surface trajectory. Quads use TL, TR, BR, BL source pixels.\n\
         format = {TRAJECTORY_FORMAT:?}\n\
         surface = {surface:?}\n\
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
    let track: SurfaceTrajectory =
        toml::from_str(&output).context("generated trajectory document is not valid TOML")?;
    track
        .validate()
        .context("generated trajectory document is invalid")?;
    Ok(output)
}

pub fn write_scene(path: &Path, contents: &str, force: bool) -> Result<()> {
    if path.exists() && !force {
        bail!(
            "refusing to overwrite scene {}; use --force to replace it",
            path.display()
        );
    }
    crate::staged_output::write_file(path, contents.as_bytes(), force)
        .with_context(|| format!("failed to write {}", path.display()))
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
    crate::portable_path::PortablePath::project(path)
        .with_context(|| format!("{description} must be a portable relative path"))?;
    Ok(())
}

fn rect_contains(outer: [f64; 4], inner: [f64; 4]) -> bool {
    const EPSILON: f64 = 1.0e-6;
    let outer_right = outer[0] + outer[2];
    let outer_bottom = outer[1] + outer[3];
    let inner_right = inner[0] + inner[2];
    let inner_bottom = inner[1] + inner[3];
    inner[0] + EPSILON >= outer[0]
        && inner[1] + EPSILON >= outer[1]
        && inner_right <= outer_right + EPSILON
        && inner_bottom <= outer_bottom + EPSILON
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
    Ok(PathBuf::from(
        crate::portable_path::relative_reference(owner, target)?.to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_scene_is_valid() {
        let text = scene_document(
            Path::new("example.mp4"),
            Path::new("example.plaque.toml"),
            "ensemble",
            None,
            &[],
        )
        .unwrap();
        let scene: Scene = toml::from_str(&text).unwrap();
        scene.validate().unwrap();
    }

    #[test]
    fn detected_proposal_keeps_the_human_manifest_short() {
        let text = scene_document(
            Path::new("example.mp4"),
            Path::new("example.plaque.toml"),
            "ensemble",
            Some(SurfaceProposal {
                reference_frame: 51,
                bounds: [65.0, 6.0, 905.0, 487.0],
                confidence: 0.776,
            }),
            &[SurfaceProposal {
                reference_frame: 51,
                bounds: [700.0, 40.0, 300.0, 150.0],
                confidence: 0.63,
            }],
        )
        .unwrap();
        let scene: Scene = toml::from_str(&text).unwrap();

        scene.validate().unwrap();
        assert_eq!(scene.surfaces.len(), 1);
        assert_eq!(scene.surfaces[0].reference_frame, Some(51));
        assert_eq!(scene.surfaces[0].bounds, Some([65.0, 6.0, 905.0, 487.0]));
        assert!(!text.contains("Alternative automatic candidate"));
        assert!(text.contains("[[surfaces.anchors]]"));
    }

    #[test]
    fn injected_surface_accepts_outer_bounds_and_inner_writable_region() {
        let scene: Scene = toml::from_str(
            r#"
                format = "plaque-forge.scene/1"
                source = "clip.mp4"
                default_surface = "main"

                [[surfaces]]
                id = "main"
                space = "scene-plane"
                reference_frame = 0
                bounds = [100.0, 40.0, 500.0, 180.0]

                [surfaces.writable_region]
                shape = "ellipse"
                center = [350.0, 130.0]
                radii = [210.0, 65.0]

                [surfaces.appearance]
                kind = "image"
                image = "plaque.png"
            "#,
        )
        .unwrap();

        scene.validate().unwrap();
        assert_eq!(
            scene.surfaces[0].tracking_bounds(),
            Some([100.0, 40.0, 500.0, 180.0])
        );
    }

    #[test]
    fn writable_region_must_be_contained_by_the_tracking_surface() {
        let scene: Scene = toml::from_str(
            r#"
                format = "plaque-forge.scene/1"
                source = "clip.mp4"
                default_surface = "main"

                [[surfaces]]
                id = "main"
                space = "scene-plane"
                reference_frame = 0
                bounds = [100.0, 40.0, 500.0, 180.0]

                [surfaces.writable_region]
                shape = "rect"
                bounds = [120.0, 60.0, 490.0, 150.0]
            "#,
        )
        .unwrap();

        let error = scene.validate().unwrap_err().to_string();
        assert!(
            error.contains("must be fully contained inside surface bounds"),
            "unexpected validation error: {error}"
        );
    }

    #[test]
    fn writable_region_may_touch_the_tracking_surface_boundary() {
        let scene: Scene = toml::from_str(
            r#"
                format = "plaque-forge.scene/1"
                source = "clip.mp4"
                default_surface = "main"

                [[surfaces]]
                id = "main"
                space = "scene-plane"
                reference_frame = 0
                bounds = [100.0, 40.0, 500.0, 180.0]

                [surfaces.writable_region]
                shape = "rect"
                bounds = [100.0, 40.0, 500.0, 180.0]
            "#,
        )
        .unwrap();

        scene.validate().unwrap();
    }

    #[test]
    fn scene_selects_an_explicit_plaque() {
        let scene: Scene = toml::from_str(
            r#"
                format = "plaque-forge.scene/1"
                source = "clip.mp4"
                default_surface = "right"

                [[surfaces]]
                id = "left"
                space = "scene-plane"

                [[surfaces]]
                id = "right"
                space = "scene-plane"
            "#,
        )
        .unwrap();
        scene.validate().unwrap();
        assert_eq!(scene.select_surface(None).unwrap().id, "right");
        assert_eq!(scene.select_surface(Some("left")).unwrap().id, "left");
    }

    #[test]
    fn layer_artifacts_distinguish_canonical_images_and_source_sequences() {
        let image: LayerArtifact = toml::from_str(
            r#"
                format = "plaque-forge.layer/1"
                kind = "alpha-image"
                coordinates = "plaque-canonical"
                path = "moss.png"
            "#,
        )
        .unwrap();
        let sequence: LayerArtifact = toml::from_str(
            r#"
                format = "plaque-forge.layer/1"
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
    fn authored_layer_generator_metadata_does_not_require_worker_cache_identity() {
        let artifact: LayerArtifact = toml::from_str(
            r#"
                format = "plaque-forge.layer/1"
                kind = "alpha-image"
                coordinates = "plaque-canonical"
                path = "moss.png"

                [generator]
                backend = "color-refinement"
                model = "moss-alpha"
                version = "1"
            "#,
        )
        .unwrap();

        artifact.validate().unwrap();
        assert!(artifact.validate_generated_provenance().is_err());
    }

    #[test]
    fn track_accepts_mixed_authority() {
        let track: SurfaceTrajectory = toml::from_str(
            r#"
                format = "plaque-forge.trajectory/1"
                surface = "main"
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
    fn generated_trajectory_round_trips() {
        let text = trajectory_document(
            "main",
            &"a".repeat(64),
            &[(0, [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [0.0, 1.0]], 1.0)],
            false,
        )
        .unwrap();
        let track: SurfaceTrajectory = toml::from_str(&text).unwrap();
        track.validate().unwrap();
        assert!(!track.keyframes[0].locked);
    }

    #[test]
    fn generated_trajectory_rejects_an_invalid_surface_id() {
        let result = trajectory_document(
            "not a plaque",
            &"a".repeat(64),
            &[(0, [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [0.0, 1.0]], 1.0)],
            false,
        );

        assert!(result.is_err());
    }

    #[test]
    fn mixed_track_counts_authority() {
        let track: SurfaceTrajectory = toml::from_str(
            r#"
                format = "plaque-forge.trajectory/1"
                surface = "main"
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
    fn screen_canvas_is_an_explicit_flat_space() {
        let scene: Scene = toml::from_str(
            r#"
                format = "plaque-forge.scene/1"
                source = "clip.mp4"
                default_surface = "main"

                [[surfaces]]
                id = "main"
                space = "screen-canvas"
                depth = "flat"
                bounds = [20.0, 30.0, 400.0, 200.0]
            "#,
        )
        .unwrap();
        scene.validate().unwrap();
        assert_eq!(scene.surfaces[0].space, SurfaceSpace::ScreenCanvas);
    }

    #[test]
    fn schema_two_sparse_normalized_motion_resolves_to_source_pixels() {
        let scene: Scene = toml::from_str(
            r#"
                format = "plaque-forge.scene/1"
                source = "clip.mp4"
                default_surface = "main"

                [[surfaces]]
                id = "main"
                space = "scene-plane"
                bounds = [10.0, 20.0, 100.0, 50.0]

                [[surfaces.anchors]]
                frame = 7
                coordinates = "normalized"
                quad = [[0.1, 0.2], [0.8, 0.2], [0.8, 0.7], [0.1, 0.7]]
            "#,
        )
        .unwrap();
        scene.validate().unwrap();
        let track = scene.surfaces[0]
            .sparse_trajectory(1000, 500, &"a".repeat(64))
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
    fn prompts_default_to_source_pixels() {
        let prompt: SegmentationPrompt = toml::from_str(
            r#"
                frame = 3
                positive_points = [[500.0, 100.0]]
            "#,
        )
        .unwrap();
        assert_eq!(prompt.coordinates, SpatialCoordinates::SourcePixels);
        prompt.validate("prompt").unwrap();
    }

    #[test]
    fn opaque_matte_is_only_valid_for_foreground_layers() {
        let scene: Scene = toml::from_str(
            r#"
                format = "plaque-forge.scene/1"
                source = "clip.mp4"
                default_surface = "main"

                [[surfaces]]
                id = "main"
                space = "scene-plane"
                bounds = [10.0, 20.0, 100.0, 50.0]

                [[layers]]
                id = "shadow"
                role = "shadow"
                surface = "main"
                matte = { mode = "opaque" }
            "#,
        )
        .unwrap();
        assert!(scene.validate().is_err());
    }

    #[test]
    fn cache_identity_accepts_exact_bytes_even_when_cached_semantic_hash_is_stale() {
        let a = SceneProvenance {
            manifest: Some(InputFileProvenance {
                path: "a.toml".into(),
                sha256: "same-raw".into(),
                semantic_sha256: Some("old-semantics".into()),
            }),
            surface_id: Some("main".into()),
            ..SceneProvenance::default()
        };
        let b = SceneProvenance {
            manifest: Some(InputFileProvenance {
                path: "b.toml".into(),
                sha256: "same-raw".into(),
                semantic_sha256: Some("new-semantics".into()),
            }),
            surface_id: Some("main".into()),
            ..SceneProvenance::default()
        };
        assert!(a.content_matches(&b));
    }

    #[test]
    fn cache_identity_uses_semantics_instead_of_comments() {
        let file = |path: &str, raw: &str| InputFileProvenance {
            path: path.into(),
            sha256: raw.into(),
            semantic_sha256: Some("same-semantics".into()),
        };
        let a = SceneProvenance {
            manifest: Some(file("a.toml", "raw-a")),
            surface_id: Some("main".into()),
            ..SceneProvenance::default()
        };
        let b = SceneProvenance {
            manifest: Some(file("b.toml", "raw-b")),
            surface_id: Some("main".into()),
            ..SceneProvenance::default()
        };
        assert!(a.content_matches(&b));
    }
}
