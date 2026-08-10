use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SIDECAR_FORMAT_VERSION: u32 = 1;
pub const MOTION_TRACK_FORMAT_VERSION: u32 = 2;
const LEGACY_MOTION_TRACK_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy)]
pub struct PlaqueProposal {
    pub reference_frame: usize,
    pub bounds: [f64; 4],
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaqueMetadata {
    pub id: String,
    pub reference_frame: Option<usize>,
    pub bounds: Option<[f64; 4]>,
    pub motion_track: Option<PathBuf>,
    #[serde(default)]
    pub prompts: Vec<SegmentationPrompt>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SegmentationPrompt {
    pub frame: usize,
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
    Foreground,
    Background,
    Reflection,
    Shadow,
    Modulation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerMetadata {
    pub id: String,
    pub role: LayerRole,
    pub plaque: String,
    pub in_front_of: Option<String>,
    pub artifact: Option<PathBuf>,
    #[serde(default)]
    pub prompts: Vec<SegmentationPrompt>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceMetadata {
    pub schema_version: u32,
    pub source: PathBuf,
    pub default_plaque: Option<String>,
    #[serde(default)]
    pub plaques: Vec<PlaqueMetadata>,
    #[serde(default)]
    pub layers: Vec<LayerMetadata>,
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
pub struct HumanMotionTrack {
    pub schema_version: u32,
    pub plaque: String,
    pub coordinates: CoordinateSystem,
    pub source_sha256: Option<String>,
    pub keyframes: Vec<MotionKeyframe>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputFileProvenance {
    pub path: PathBuf,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct HumanInputProvenance {
    pub metadata: Option<InputFileProvenance>,
    pub plaque_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plaque_hint: Option<[f64; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plaque_frame: Option<usize>,
    pub motion_track: Option<InputFileProvenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_csv: Option<InputFileProvenance>,
    pub locked_keyframes: usize,
    pub guide_keyframes: usize,
}

impl HumanInputProvenance {
    pub fn content_matches(&self, other: &Self) -> bool {
        self.plaque_id == other.plaque_id
            && self.plaque_hint == other.plaque_hint
            && self.plaque_frame == other.plaque_frame
            && file_hash(&self.metadata) == file_hash(&other.metadata)
            && file_hash(&self.motion_track) == file_hash(&other.motion_track)
            && file_hash(&self.track_csv) == file_hash(&other.track_csv)
            && self.locked_keyframes == other.locked_keyframes
            && self.guide_keyframes == other.guide_keyframes
    }
}

fn file_hash(file: &Option<InputFileProvenance>) -> Option<&str> {
    file.as_ref().map(|file| {
        file.semantic_sha256
            .as_deref()
            .unwrap_or(file.sha256.as_str())
    })
}

#[derive(Debug, Clone)]
pub struct LoadedSourceMetadata {
    pub path: PathBuf,
    pub document: SourceMetadata,
}

impl SourceMetadata {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read source metadata {}", path.display()))?;
        let document: Self = toml::from_str(&text)
            .with_context(|| format!("failed to parse source metadata {}", path.display()))?;
        document
            .validate()
            .with_context(|| format!("invalid source metadata {}", path.display()))?;
        Ok(document)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SIDECAR_FORMAT_VERSION {
            bail!(
                "unsupported metadata schema {}; expected {}",
                self.schema_version,
                SIDECAR_FORMAT_VERSION
            );
        }
        require_relative(&self.source, "source")?;
        if self.plaques.is_empty() {
            bail!("metadata must declare at least one [[plaques]] entry");
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
            if let Some(path) = &plaque.motion_track {
                require_relative(path, &format!("plaque {:?} motion_track", plaque.id))?;
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
            for prompt in &layer.prompts {
                prompt.validate(&format!("layer {:?} prompt", layer.id))?;
            }
        }
        Ok(())
    }

    pub fn select_plaque(&self, requested: Option<&str>) -> Result<&PlaqueMetadata> {
        let id = requested.or(self.default_plaque.as_deref());
        if let Some(id) = id {
            return self
                .plaques
                .iter()
                .find(|plaque| plaque.id == id)
                .with_context(|| format!("metadata does not declare plaque {id:?}"));
        }
        if self.plaques.len() == 1 {
            return Ok(&self.plaques[0]);
        }
        bail!("metadata declares multiple plaques; select one with --plaque <id>")
    }
}

impl SegmentationPrompt {
    fn validate(&self, description: &str) -> Result<()> {
        if let Some(bounds) = self.box_bounds {
            validate_rect(bounds, &format!("{description} box_bounds"))?;
        }
        for (kind, points) in [
            ("positive_points", self.positive_points.as_slice()),
            ("negative_points", self.negative_points.as_slice()),
            ("polygon", self.polygon.as_slice()),
        ] {
            for (index, point) in points.iter().enumerate() {
                validate_point(*point, &format!("{description} {kind}[{index}]"))?;
            }
        }
        if !self.polygon.is_empty() && self.polygon.len() < 3 {
            bail!("{description} polygon must contain at least three points");
        }
        if let Some(quad) = self.quad {
            validate_quad(quad, &format!("{description} quad"))?;
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
}

impl HumanMotionTrack {
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
        if ![
            LEGACY_MOTION_TRACK_FORMAT_VERSION,
            MOTION_TRACK_FORMAT_VERSION,
        ]
        .contains(&self.schema_version)
        {
            bail!(
                "unsupported motion-track schema {}; expected {} or {}",
                self.schema_version,
                LEGACY_MOTION_TRACK_FORMAT_VERSION,
                MOTION_TRACK_FORMAT_VERSION
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
        let has_locked = self.keyframes.iter().any(|keyframe| keyframe.locked);
        let has_guides = self.keyframes.iter().any(|keyframe| !keyframe.locked);
        if self.schema_version == LEGACY_MOTION_TRACK_FORMAT_VERSION && has_locked && has_guides {
            bail!("motion-track schema version 1 cannot mix locked and guided keyframes");
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

pub fn default_sidecar_path(input: &Path) -> PathBuf {
    input.with_extension("plaque.toml")
}

pub fn find_source_metadata(
    input: &Path,
    explicit: Option<&Path>,
) -> Result<Option<LoadedSourceMetadata>> {
    let path = match explicit {
        Some(path) => path.to_path_buf(),
        None => {
            let candidate = default_sidecar_path(input);
            if !candidate.is_file() {
                return Ok(None);
            }
            candidate
        }
    };
    if !path.is_file() {
        bail!(
            "source metadata does not exist or is not a file: {}",
            path.display()
        );
    }
    let document = SourceMetadata::load(&path)?;
    Ok(Some(LoadedSourceMetadata { path, document }))
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
        sha256: sha256(path)?,
        semantic_sha256: None,
    })
}

pub fn semantic_provenance<T: Serialize>(path: &Path, value: &T) -> Result<InputFileProvenance> {
    let mut output = provenance(path)?;
    output.semantic_sha256 = Some(sha256_bytes(&serde_json::to_vec(value)?));
    Ok(output)
}

pub fn current_human_input_provenance(
    input: &Path,
    explicit_metadata: Option<&Path>,
    requested_plaque: Option<&str>,
    explicit_plaque_hint: Option<[f64; 4]>,
    explicit_plaque_frame: Option<usize>,
    explicit_motion_track: Option<&Path>,
    track_csv: Option<&Path>,
) -> Result<Option<HumanInputProvenance>> {
    let loaded = find_source_metadata(input, explicit_metadata)?;
    let mut identity = HumanInputProvenance::default();
    if let Some(bounds) = explicit_plaque_hint {
        identity.plaque_hint = Some(bounds);
        identity.plaque_frame = Some(explicit_plaque_frame.unwrap_or(0));
    } else if explicit_plaque_frame.is_some() {
        bail!("--plaque-frame requires --plaque-hint");
    }
    let mut referenced_track = None;
    if let Some(loaded) = &loaded {
        let selected = loaded.document.select_plaque(requested_plaque)?;
        identity.metadata = Some(semantic_provenance(&loaded.path, &loaded.document)?);
        identity.plaque_id = Some(selected.id.clone());
        if track_csv.is_none() {
            referenced_track = selected
                .motion_track
                .as_ref()
                .map(|path| resolve_relative(&loaded.path, path));
        }
    } else if let Some(id) = requested_plaque {
        bail!("--plaque {id:?} requires a metadata sidecar");
    }

    let track_path = if track_csv.is_some() {
        None
    } else {
        explicit_motion_track
            .map(Path::to_path_buf)
            .or(referenced_track)
    };
    if let Some(path) = track_path {
        let track = HumanMotionTrack::load(&path)?;
        if let Some(plaque_id) = &identity.plaque_id
            && track.plaque != *plaque_id
        {
            bail!(
                "motion track describes plaque {:?}, but metadata selected {:?}",
                track.plaque,
                plaque_id
            );
        }
        identity.plaque_id = Some(track.plaque.clone());
        identity.motion_track = Some(semantic_provenance(&path, &track)?);
        identity.locked_keyframes = track.locked_keyframes();
        identity.guide_keyframes = track.guide_keyframes();
    }
    if let Some(path) = track_csv {
        identity.track_csv = Some(provenance(path)?);
    }

    if identity == HumanInputProvenance::default() {
        Ok(None)
    } else {
        Ok(Some(identity))
    }
}

pub fn sha256(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(sha256_bytes(&bytes))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}

pub fn sidecar_document(
    input: &Path,
    sidecar: &Path,
    detector: &str,
    proposal: Option<PlaqueProposal>,
    alternatives: &[PlaqueProposal],
) -> Result<String> {
    let source = relative_reference(sidecar, input)?;
    let source_stem = input
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("video");
    let suggested_track = format!("{source_stem}.main.track.toml");
    let mut output = format!(
        "# Plaque Forge source metadata. This file is human-owned.\n\
         # Plaque Forge will not overwrite it without explicit --force.\n\
         schema_version = {SIDECAR_FORMAT_VERSION}\n\
         source = {}\n\
         default_plaque = \"main\"\n\n\
         [[plaques]]\n\
         id = \"main\"\n",
        toml_string(&source.to_string_lossy())?
    );
    if let Some(proposal) = proposal {
        output.push_str(&format!(
            "# Automatic {detector} proposal (confidence {:.3}). Review and edit as needed.\n\
             # Bounds are [x, y, width, height] in source pixels on reference_frame.\n\
             # Analysis still measures plaque motion on every frame.\n\
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
            "# Automatic plaque detection found no proposal. Set both fields manually.\n\
             # reference_frame = 0\n\
             # bounds = [100.0, 100.0, 400.0, 200.0]\n",
        );
    }
    output.push_str(&format!(
        "# Optional motion track, relative to this file.\n\
         # motion_track = {}\n\n",
        toml_string(&suggested_track)?
    ));
    output.push_str(
        "# Segmentation prompts are optional. Positive points belong to the target;\n\
         # negative points identify nearby pixels that must be excluded.\n\
         # [[plaques.prompts]]\n",
    );
    if let Some(proposal) = proposal {
        output.push_str(&format!(
            "# frame = {}\n\
             # box_bounds = [{:.1}, {:.1}, {:.1}, {:.1}]\n\
             # positive_points = [[{:.1}, {:.1}]]\n\
             # negative_points = []\n",
            proposal.reference_frame,
            proposal.bounds[0],
            proposal.bounds[1],
            proposal.bounds[2],
            proposal.bounds[3],
            proposal.bounds[0] + proposal.bounds[2] * 0.5,
            proposal.bounds[1] + proposal.bounds[3] * 0.5,
        ));
    } else {
        output.push_str(
            "# frame = 0\n\
             # box_bounds = [100.0, 100.0, 400.0, 200.0]\n\
             # positive_points = [[300.0, 200.0]]\n\
             # negative_points = []\n",
        );
    }
    for (index, alternative) in alternatives.iter().enumerate() {
        output.push_str(&format!(
            "\n# Alternative automatic candidate {} (confidence {:.3}).\n\
             # Replace main's geometry with this candidate, or uncomment this block\n\
             # if it is a separate plaque you want to keep.\n\
             # [[plaques]]\n\
             # id = \"candidate-{}\"\n\
             # reference_frame = {}\n\
             # bounds = [{:.1}, {:.1}, {:.1}, {:.1}]\n",
            index + 1,
            alternative.confidence.clamp(0.0, 1.0),
            index + 2,
            alternative.reference_frame,
            alternative.bounds[0],
            alternative.bounds[1],
            alternative.bounds[2],
            alternative.bounds[3],
        ));
    }
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
        "# Plaque Forge motion-track proposal. This file is human-owned after export.\n\
         # Coordinates are source-video pixels. Corner order is top-left, top-right,\n\
         # bottom-right, bottom-left. Unlocked quads guide automatic refinement.\n\
         # Set locked = true only on reviewed frames that must be exact. A locked\n\
         # entry for every frame is a fully authoritative motion track.\n\
         schema_version = {MOTION_TRACK_FORMAT_VERSION}\n\
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
    let track: HumanMotionTrack =
        toml::from_str(&output).context("generated motion-track document is not valid TOML")?;
    track
        .validate()
        .context("generated motion-track document is invalid")?;
    Ok(output)
}

pub fn write_human_file(path: &Path, contents: &str, force: bool) -> Result<()> {
    if path.exists() && !force {
        bail!(
            "refusing to overwrite human-owned file {}; use --force only after reviewing it",
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
        bail!("{description} must be relative so the metadata remains portable");
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

fn relative_reference(owner: &Path, target: &Path) -> Result<PathBuf> {
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
        bail!("sidecar and source do not share a filesystem root");
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
    fn generated_sidecar_is_commented_and_valid() {
        let text = sidecar_document(
            Path::new("example.mp4"),
            Path::new("example.plaque.toml"),
            "ensemble",
            None,
            &[],
        )
        .unwrap();
        assert!(text.contains("human-owned"));
        let metadata: SourceMetadata = toml::from_str(&text).unwrap();
        metadata.validate().unwrap();
    }

    #[test]
    fn detected_proposal_is_active_and_alternatives_are_comments() {
        let text = sidecar_document(
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
        let metadata: SourceMetadata = toml::from_str(&text).unwrap();

        metadata.validate().unwrap();
        assert_eq!(metadata.plaques.len(), 1);
        assert_eq!(metadata.plaques[0].reference_frame, Some(51));
        assert_eq!(metadata.plaques[0].bounds, Some([65.0, 6.0, 905.0, 487.0]));
        assert!(text.contains("Alternative automatic candidate 1"));
        assert!(text.contains("# [[plaques]]"));
    }

    #[test]
    fn metadata_selects_an_explicit_plaque() {
        let metadata: SourceMetadata = toml::from_str(
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
        metadata.validate().unwrap();
        assert_eq!(metadata.select_plaque(None).unwrap().id, "right");
        assert_eq!(metadata.select_plaque(Some("left")).unwrap().id, "left");
    }

    #[test]
    fn track_rejects_mixed_authority() {
        let track: HumanMotionTrack = toml::from_str(
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
        assert!(
            track
                .validate()
                .unwrap_err()
                .to_string()
                .contains("cannot mix")
        );
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
        let track: HumanMotionTrack = toml::from_str(&text).unwrap();
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
    fn schema_two_accepts_mixed_authority() {
        let track: HumanMotionTrack = toml::from_str(
            r#"
                schema_version = 2
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
    fn cache_identity_uses_semantics_instead_of_comments() {
        let file = |path: &str, raw: &str| InputFileProvenance {
            path: path.into(),
            sha256: raw.into(),
            semantic_sha256: Some("same-semantics".into()),
        };
        let a = HumanInputProvenance {
            metadata: Some(file("a.toml", "raw-a")),
            plaque_id: Some("main".into()),
            ..HumanInputProvenance::default()
        };
        let b = HumanInputProvenance {
            metadata: Some(file("b.toml", "raw-b")),
            plaque_id: Some("main".into()),
            ..HumanInputProvenance::default()
        };
        assert!(a.content_matches(&b));
    }

    #[test]
    fn cache_identity_includes_bounds_and_legacy_track_contents() {
        let file = |hash: &str| InputFileProvenance {
            path: "track.csv".into(),
            sha256: hash.into(),
            semantic_sha256: None,
        };
        let baseline = HumanInputProvenance {
            plaque_hint: Some([10.0, 20.0, 30.0, 40.0]),
            plaque_frame: Some(0),
            track_csv: Some(file("csv-a")),
            ..HumanInputProvenance::default()
        };
        let changed_bounds = HumanInputProvenance {
            plaque_hint: Some([11.0, 20.0, 30.0, 40.0]),
            ..baseline.clone()
        };
        let changed_csv = HumanInputProvenance {
            track_csv: Some(file("csv-b")),
            ..baseline.clone()
        };

        assert!(!baseline.content_matches(&changed_bounds));
        assert!(!baseline.content_matches(&changed_csv));
    }

    #[test]
    fn cache_identity_normalizes_the_default_plaque_frame() {
        let implicit = current_human_input_provenance(
            Path::new("missing-test-video.mp4"),
            None,
            None,
            Some([10.0, 20.0, 30.0, 40.0]),
            None,
            None,
            None,
        )
        .unwrap()
        .unwrap();
        let explicit = current_human_input_provenance(
            Path::new("missing-test-video.mp4"),
            None,
            None,
            Some([10.0, 20.0, 30.0, 40.0]),
            Some(0),
            None,
            None,
        )
        .unwrap()
        .unwrap();

        assert!(implicit.content_matches(&explicit));
    }

    #[test]
    fn existing_format_four_provenance_defaults_new_fields() {
        let provenance: HumanInputProvenance = toml::from_str(
            r#"
                plaque_id = "main"
                locked_keyframes = 0
                guide_keyframes = 0
            "#,
        )
        .unwrap();

        assert_eq!(provenance.plaque_hint, None);
        assert_eq!(provenance.plaque_frame, None);
        assert_eq!(provenance.track_csv, None);
    }
}
