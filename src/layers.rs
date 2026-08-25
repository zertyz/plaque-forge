//! Import and compose declared scene layers into analysis data.
//!
//! Layers describe material that constrains text placement or must appear in front of
//! rendered typography, such as vines, chains, shadows, and writing-surface masks.

use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use image::{GrayImage, ImageBuffer, Luma};

use crate::{
    analysis::{Analysis, CONTENT_MASK_FILE, LAYERS_DIR, LayerAsset, sequence_path},
    analyze::extraction::{rectify, transformed_rect},
    color::Rgba,
    model::{MotionSample, RectF},
    scene::{
        LayerArtifact, LayerArtifactKind, LayerCoordinates, LayerMatte, LayerMatteMode, LayerRole,
        SceneLayer, resolve_relative,
    },
    surface::Surface,
};

pub struct LayerInput {
    pub scene: SceneLayer,
    pub artifact_path: std::path::PathBuf,
    pub artifact: LayerArtifact,
}

pub fn has_authored_foreground(inputs: &[LayerInput]) -> bool {
    inputs
        .iter()
        .any(|input| input.scene.role == LayerRole::Foreground)
}

pub fn has_authored_opaque_source_foreground(inputs: &[LayerInput]) -> bool {
    inputs.iter().any(|input| {
        input.scene.role == LayerRole::Foreground
            && input.scene.matte.mode == LayerMatteMode::Opaque
            && input.artifact.coordinates == LayerCoordinates::SourcePixels
    })
}

/// Return the calibration policy that may safely be applied after authored opaque
/// masks have been projected into source coordinates. Different policies cannot be
/// collapsed into one without changing at least one layer's declared semantics.
pub fn shared_authored_opaque_source_matte(inputs: &[LayerInput]) -> Option<LayerMatte> {
    let mut mattes = inputs
        .iter()
        .filter(|input| {
            input.scene.role == LayerRole::Foreground
                && input.scene.matte.mode == LayerMatteMode::Opaque
                && input.artifact.coordinates == LayerCoordinates::SourcePixels
        })
        .map(|input| input.scene.matte);
    let first = mattes.next()?;
    mattes.all(|matte| matte == first).then_some(first)
}

pub(crate) fn source_opaque_foreground_mask(
    inputs: &[LayerInput],
    frame: usize,
    width: u32,
    height: u32,
) -> Result<Option<Vec<u8>>> {
    let mut combined = vec![0_u8; width as usize * height as usize];
    let mut found = false;
    for input in inputs.iter().filter(|input| {
        input.scene.role == LayerRole::Foreground
            && input.scene.matte.mode == LayerMatteMode::Opaque
            && input.artifact.coordinates == LayerCoordinates::SourcePixels
    }) {
        if let Some(path) = artifact_frame_path(input, frame) {
            let mut mask = load_mask(&path, width, height)?;
            apply_matte_policy(&mut mask, input.scene.matte);
            alpha_over(&mut combined, &mask);
            found = true;
        }
    }
    Ok(found.then_some(combined))
}

pub fn build_tracking_exclusions(
    inputs: &[LayerInput],
    automatic_root: &Path,
    output_root: &Path,
    width: u32,
    height: u32,
    frames: usize,
    tracked_surface: Option<(&[MotionSample], RectF)>,
) -> Result<bool> {
    let foregrounds = inputs
        .iter()
        .filter(|input| {
            input.scene.affects_tracking
                && input.scene.role == LayerRole::Foreground
                && input.artifact.coordinates == LayerCoordinates::SourcePixels
        })
        .collect::<Vec<_>>();
    let backgrounds = inputs
        .iter()
        .filter(|input| {
            input.scene.affects_tracking
                && input.scene.role == LayerRole::Background
                && input.artifact.coordinates == LayerCoordinates::SourcePixels
        })
        .collect::<Vec<_>>();
    let writing_surfaces = inputs
        .iter()
        .filter(|input| {
            input.scene.affects_tracking
                && input.scene.role == LayerRole::WritingSurface
                && input.artifact.coordinates == LayerCoordinates::SourcePixels
        })
        .collect::<Vec<_>>();
    // Background declarations only subtract false-positive foreground. By
    // themselves they cannot exclude tracking evidence and do not warrant a
    // directory full of zero masks.
    if foregrounds.is_empty()
        && (writing_surfaces.is_empty() || tracked_surface.is_none())
        && !automatic_root.is_dir()
    {
        return Ok(false);
    }
    for input in foregrounds.iter().chain(&backgrounds).chain(
        tracked_surface
            .is_some()
            .then_some(&writing_surfaces)
            .into_iter()
            .flatten(),
    ) {
        validate_frame_range(&input.artifact, frames, &input.scene.id)?;
    }
    fs::create_dir_all(output_root)?;
    for frame in 0..frames {
        let automatic = automatic_root.join(format!("{frame:06}.png"));
        let mut combined = if automatic.is_file() {
            load_mask(&automatic, width, height)?
        } else {
            vec![0; width as usize * height as usize]
        };
        // Tracking treats every non-zero mask pixel as excluded. Automatic masks
        // carry calibrated confidence for rendering, so using their faint support
        // edge here would remove evidence the semantic model did not confidently
        // classify as foreground. Keep the historic categorical p50 contract for
        // pose estimation while preserving soft confidence for compositing.
        for alpha in &mut combined {
            *alpha = u8::from(*alpha >= 128) * 255;
        }
        for input in &foregrounds {
            if let Some(path) = artifact_frame_path(input, frame) {
                let mut mask = load_mask(&path, width, height)?;
                apply_matte_policy(&mut mask, input.scene.matte);
                alpha_over(&mut combined, &mask);
            }
        }
        // A declared background layer is negative depth evidence: it may resemble
        // or move through the title plane, but it cannot occlude typography or pull
        // the plaque tracker. Subtract it from both automatic and authored masks.
        for input in &backgrounds {
            if let Some(path) = artifact_frame_path(input, frame) {
                let mut mask = load_mask(&path, width, height)?;
                apply_matte_policy(&mut mask, input.scene.matte);
                subtract_alpha(&mut combined, &mask);
            }
        }
        // A writing-surface matte answers a membership/depth question, not a
        // rigid-pose question. Use it to keep all point/descriptor evidence on
        // actual surface material (and out of foreground holes), while the
        // homography itself is estimated from persistent material reference
        // points. Multiple writing-surface parts form one allowed union.
        if let Some((motion, plaque)) = tracked_surface
            && !writing_surfaces.is_empty()
        {
            let mut support = vec![0; width as usize * height as usize];
            let mut has_support = false;
            for input in &writing_surfaces {
                if let Some(path) = artifact_frame_path(input, frame) {
                    let mut mask = load_mask(&path, width, height)?;
                    apply_matte_policy(&mut mask, input.scene.matte);
                    max_union(&mut support, &mask);
                    has_support = true;
                }
            }
            // Semantic propagation can report no object or confidently switch to
            // another dark object between strong prompt frames. It becomes a
            // tracking prior only after the independently estimated rigid plane
            // confirms both overlap and area. An unavailable/rejected prior never
            // means the plaque vanished; foreground exclusions remain authoritative.
            if has_support
                && motion.get(frame).is_some_and(|sample| {
                    writing_surface_support_is_plausible(
                        &support,
                        width,
                        height,
                        plaque,
                        sample.transform,
                    )
                })
            {
                exclude_outside_surface_support(&mut combined, &support);
            }
        }
        save_mask(
            width,
            height,
            &combined,
            &output_root.join(format!("{frame:06}.png")),
        )?;
    }
    Ok(true)
}

pub(crate) fn writing_surface_support_is_plausible(
    support: &[u8],
    width: u32,
    height: u32,
    plaque: RectF,
    transform: crate::model::Mat3,
) -> bool {
    if support.len() != width as usize * height as usize {
        return false;
    }
    let Some(inverse) = transform.inverse() else {
        return false;
    };
    let mut supported = 0usize;
    let mut expected = 0usize;
    let mut overlap = 0usize;
    // A four-pixel grid is stable at both project aspect ratios and bounds this
    // validation to roughly 1/16 of a frame without changing mask composition.
    for y in (0..height).step_by(4) {
        for x in (0..width).step_by(4) {
            let material = inverse.transform(crate::model::PointF {
                x: f64::from(x),
                y: f64::from(y),
            });
            let on_plane = material.x >= plaque.x
                && material.y >= plaque.y
                && material.x <= plaque.x + plaque.width
                && material.y <= plaque.y + plaque.height;
            let is_supported = support[y as usize * width as usize + x as usize] >= 64;
            expected += usize::from(on_plane);
            supported += usize::from(is_supported);
            overlap += usize::from(on_plane && is_supported);
        }
    }
    if expected < 64 || supported < 64 {
        return false;
    }
    let precision = overlap as f64 / supported as f64;
    let plane_coverage = overlap as f64 / expected as f64;
    precision >= 0.75 && plane_coverage >= 0.25
}

pub(crate) fn exclude_outside_surface_support(exclusion: &mut [u8], support: &[u8]) {
    for (excluded, &supported) in exclusion.iter_mut().zip(support) {
        if supported < 64 {
            *excluded = 255;
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn package(
    inputs: &[LayerInput],
    output_root: &Path,
    canonical_width: u32,
    canonical_height: u32,
    source_width: u32,
    source_height: u32,
    plaque: RectF,
    motion: &[MotionSample],
    content_mask: &mut [u8],
) -> Result<Vec<LayerAsset>> {
    let mut packed = Vec::with_capacity(inputs.len());
    for input in inputs {
        let artifact = &input.artifact;
        validate_frame_range(artifact, motion.len(), &input.scene.id)?;
        let directory = Path::new(LAYERS_DIR).join(&input.scene.id);
        fs::create_dir_all(output_root.join(&directory))?;
        let packed_path = match artifact.kind {
            LayerArtifactKind::AlphaImage => directory.join("mask.png"),
            LayerArtifactKind::AlphaSequence => directory.join("%06d.png"),
        };

        for (frame, source) in artifact_files(artifact, &input.artifact_path) {
            let (expected_width, expected_height) = match artifact.coordinates {
                LayerCoordinates::PlaqueCanonical => (canonical_width, canonical_height),
                LayerCoordinates::SourcePixels => (source_width, source_height),
            };
            // Decode once to validate geometry and channel semantics, then preserve
            // the original lossless PNG bytes (including 16-bit soft alpha).
            let _ = load_mask(&source, expected_width, expected_height)?;
            let destination = match frame {
                Some(frame) => output_root.join(sequence_path(&packed_path, frame)),
                None => output_root.join(&packed_path),
            };
            fs::copy(&source, &destination).with_context(|| {
                format!(
                    "failed to import layer mask {} as {}",
                    source.display(),
                    destination.display()
                )
            })?;
        }
        if !input.scene.prompts.is_empty() {
            let mut published = artifact.clone();
            match published.kind {
                LayerArtifactKind::AlphaImage => {
                    published.path = Some(std::path::PathBuf::from("mask.png"));
                }
                LayerArtifactKind::AlphaSequence => {
                    published.pattern = Some(std::path::PathBuf::from("%06d.png"));
                }
            }
            fs::write(
                output_root.join(&directory).join("artifact.toml"),
                format!(
                    "# Generated layer cache. Regenerate with analyze.\n{}",
                    toml::to_string_pretty(&published)?
                ),
            )?;
            copy_generated_sidecars(input, &output_root.join(&directory))?;
        }

        let layer = LayerAsset {
            id: input.scene.id.clone(),
            role: input.scene.role,
            coordinates: artifact.coordinates,
            kind: artifact.kind,
            affects_layout: artifact.affects_layout,
            affects_tracking: input.scene.affects_tracking,
            matte: input.scene.matte,
            path: crate::portable_path::PortablePath::bundle(packed_path)?,
            first_frame: artifact.first_frame,
            last_frame: artifact.last_frame,
            generator: artifact.generator.clone(),
        };
        apply_layout_layer(
            &layer,
            output_root,
            plaque,
            motion,
            canonical_width,
            canonical_height,
            source_width,
            source_height,
            content_mask,
        )?;
        packed.push(layer);
    }
    if !inputs.is_empty() {
        save_mask(
            canonical_width,
            canonical_height,
            content_mask,
            &output_root.join(CONTENT_MASK_FILE),
        )?;
    }
    Ok(packed)
}

fn copy_generated_sidecars(input: &LayerInput, destination: &Path) -> Result<()> {
    let owner = input
        .artifact_path
        .parent()
        .context("generated layer artifact has no parent directory")?;
    for name in ["result.json", "strategy-selection.json"] {
        let source = owner.join(name);
        if source.is_file() {
            fs::copy(&source, destination.join(name)).with_context(|| {
                format!(
                    "failed to retain generated layer provenance {}",
                    source.display()
                )
            })?;
        }
    }
    Ok(())
}

pub struct ForegroundReader<'a> {
    pack: &'a Analysis,
    canonical: Vec<(Surface, &'a LayerAsset)>,
    source_static: Vec<(Vec<u8>, &'a LayerAsset)>,
    sequences: Vec<&'a LayerAsset>,
}

impl<'a> ForegroundReader<'a> {
    pub fn open(pack: &'a Analysis, fused_source_material: bool) -> Result<Self> {
        let mut canonical = Vec::new();
        let mut source_static = Vec::new();
        let mut sequences = Vec::new();
        for layer in pack
            .manifest
            .layers
            .iter()
            .filter(|layer| directly_restores_layer(layer, fused_source_material))
        {
            match (layer.coordinates, layer.kind) {
                (LayerCoordinates::PlaqueCanonical, LayerArtifactKind::AlphaImage) => {
                    let mut mask = load_mask(
                        &pack.require_asset_path(layer.path.as_path())?,
                        pack.manifest.canonical_width,
                        pack.manifest.canonical_height,
                    )?;
                    apply_matte_policy(&mut mask, layer.matte);
                    canonical.push((
                        Surface::from_alpha_mask(
                            pack.manifest.canonical_width,
                            pack.manifest.canonical_height,
                            &mask,
                            Rgba::new(255, 255, 255, 255),
                        )?,
                        layer,
                    ));
                }
                (LayerCoordinates::SourcePixels, LayerArtifactKind::AlphaImage) => {
                    let mut mask = load_mask(
                        &pack.require_asset_path(layer.path.as_path())?,
                        pack.manifest.source.width,
                        pack.manifest.source.height,
                    )?;
                    apply_matte_policy(&mut mask, layer.matte);
                    source_static.push((mask, layer));
                }
                (LayerCoordinates::SourcePixels, LayerArtifactKind::AlphaSequence) => {
                    sequences.push(layer);
                }
                _ => bail!("unsupported packed foreground layer {:?}", layer.id),
            }
        }
        Ok(Self {
            pack,
            canonical,
            source_static,
            sequences,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.canonical.is_empty() && self.source_static.is_empty() && self.sequences.is_empty()
    }

    pub fn frame_mask(
        &self,
        frame: usize,
        transform: crate::model::Mat3,
    ) -> Result<Option<Vec<u8>>> {
        if self.is_empty() {
            return Ok(None);
        }
        let mut combined = vec![
            0_u8;
            self.pack.manifest.source.width as usize
                * self.pack.manifest.source.height as usize
        ];
        for (canonical, _) in &self.canonical {
            let mut full = Surface::new(
                self.pack.manifest.source.width,
                self.pack.manifest.source.height,
            );
            full.warp_blend(
                canonical,
                transformed_rect(self.pack.manifest.source_plaque_rect, transform),
                1.0,
            )?;
            alpha_over(&mut combined, &full.alpha_mask());
        }
        for (mask, _) in &self.source_static {
            alpha_over(&mut combined, mask);
        }
        for layer in &self.sequences {
            if frame_in_layer(layer, frame) {
                let path = self
                    .pack
                    .require_asset_path(&sequence_path(layer.path.as_path(), frame))?;
                let mut mask = load_mask(
                    &path,
                    self.pack.manifest.source.width,
                    self.pack.manifest.source.height,
                )?;
                apply_matte_policy(&mut mask, layer.matte);
                // ML alpha sequences flicker on thin structures: small per-frame
                // gaps would let typography bleed through and blink. Grayscale
                // closing fills such gaps without moving the outer boundary.
                let radius = (self
                    .pack
                    .manifest
                    .source
                    .width
                    .min(self.pack.manifest.source.height)
                    / 360)
                    .clamp(1, 4) as usize;
                close_small_gaps(
                    &mut mask,
                    self.pack.manifest.source.width as usize,
                    self.pack.manifest.source.height as usize,
                    radius,
                );
                alpha_over(&mut combined, &mask);
            }
        }
        Ok(combined.iter().any(|&alpha| alpha > 0).then_some(combined))
    }
}

fn directly_restores_layer(layer: &LayerAsset, fused_source_material: bool) -> bool {
    let composited_role = matches!(
        layer.role,
        LayerRole::Foreground | LayerRole::Shadow | LayerRole::Reflection | LayerRole::Modulation
    );
    composited_role
        && !(fused_source_material
            && layer.role == LayerRole::Foreground
            && layer.coordinates == LayerCoordinates::SourcePixels
            && layer.matte.mode == LayerMatteMode::Opaque)
}

#[allow(clippy::too_many_arguments)]
fn apply_layout_layer(
    layer: &LayerAsset,
    root: &Path,
    plaque: RectF,
    motion: &[MotionSample],
    canonical_width: u32,
    canonical_height: u32,
    source_width: u32,
    source_height: u32,
    content: &mut [u8],
) -> Result<()> {
    if !layer.affects_layout
        || !matches!(
            layer.role,
            LayerRole::WritingSurface | LayerRole::Foreground
        )
    {
        return Ok(());
    }
    let aggregate = match (layer.coordinates, layer.kind) {
        (LayerCoordinates::PlaqueCanonical, LayerArtifactKind::AlphaImage) => {
            let mut mask = load_mask(
                &root.join(layer.path.as_path()),
                canonical_width,
                canonical_height,
            )?;
            apply_matte_policy(&mut mask, layer.matte);
            mask
        }
        (LayerCoordinates::SourcePixels, _) => aggregate_source_layer(
            layer,
            root,
            plaque,
            motion,
            canonical_width,
            canonical_height,
            source_width,
            source_height,
        )?,
        _ => bail!("unsupported layout layer {:?}", layer.id),
    };
    for (value, alpha) in content.iter_mut().zip(aggregate) {
        let factor = if layer.role == LayerRole::WritingSurface {
            alpha
        } else {
            255 - alpha
        };
        *value = ((*value as u16 * factor as u16 + 127) / 255) as u8;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn aggregate_source_layer(
    layer: &LayerAsset,
    root: &Path,
    plaque: RectF,
    motion: &[MotionSample],
    canonical_width: u32,
    canonical_height: u32,
    source_width: u32,
    source_height: u32,
) -> Result<Vec<u8>> {
    let writing_surface = layer.role == LayerRole::WritingSurface;
    let mut aggregate = vec![
        if writing_surface { 255 } else { 0 };
        canonical_width as usize * canonical_height as usize
    ];
    let frames: Box<dyn Iterator<Item = usize>> = match layer.kind {
        LayerArtifactKind::AlphaImage => Box::new(0..motion.len()),
        LayerArtifactKind::AlphaSequence => {
            Box::new(layer.first_frame.unwrap_or(0)..=layer.last_frame.unwrap_or(0))
        }
    };
    for frame in frames {
        let source = match layer.kind {
            LayerArtifactKind::AlphaImage => root.join(layer.path.as_path()),
            LayerArtifactKind::AlphaSequence => {
                root.join(sequence_path(layer.path.as_path(), frame))
            }
        };
        let mut mask = load_mask(&source, source_width, source_height)?;
        apply_matte_policy(&mut mask, layer.matte);
        let surface = Surface::from_alpha_mask(
            source_width,
            source_height,
            &mask,
            Rgba::new(255, 255, 255, 255),
        )?;
        let canonical = rectify(
            &surface,
            plaque,
            motion
                .get(frame)
                .with_context(|| format!("layer frame {frame} lacks motion"))?
                .transform,
        )?
        .alpha_mask();
        if writing_surface {
            intersect(&mut aggregate, &canonical);
        } else {
            max_union(&mut aggregate, &canonical);
        }
    }
    Ok(aggregate)
}

fn artifact_files(
    artifact: &LayerArtifact,
    owner: &Path,
) -> Vec<(Option<usize>, std::path::PathBuf)> {
    match artifact.kind {
        LayerArtifactKind::AlphaImage => artifact
            .path
            .as_ref()
            .map(|path| vec![(None, resolve_relative(owner, path))])
            .unwrap_or_default(),
        LayerArtifactKind::AlphaSequence => artifact
            .referenced_paths(owner)
            .into_iter()
            .zip(artifact.first_frame.unwrap_or(0)..)
            .map(|(path, frame)| (Some(frame), path))
            .collect(),
    }
}

fn artifact_frame_path(input: &LayerInput, frame: usize) -> Option<std::path::PathBuf> {
    match input.artifact.kind {
        LayerArtifactKind::AlphaImage => input
            .artifact
            .path
            .as_ref()
            .map(|path| resolve_relative(&input.artifact_path, path)),
        LayerArtifactKind::AlphaSequence => {
            let first = input.artifact.first_frame?;
            let last = input.artifact.last_frame?;
            if !(first..=last).contains(&frame) {
                return None;
            }
            input.artifact.pattern.as_ref().map(|pattern| {
                resolve_relative(
                    &input.artifact_path,
                    Path::new(
                        &pattern
                            .to_string_lossy()
                            .replace("%06d", &format!("{frame:06}")),
                    ),
                )
            })
        }
    }
}

fn validate_frame_range(artifact: &LayerArtifact, frames: usize, id: &str) -> Result<()> {
    if artifact.kind == LayerArtifactKind::AlphaSequence
        && artifact.last_frame.unwrap_or(usize::MAX) >= frames
    {
        bail!("layer {id:?} frame range exceeds the {frames}-frame source");
    }
    Ok(())
}

fn frame_in_layer(layer: &LayerAsset, frame: usize) -> bool {
    layer.first_frame.is_some_and(|first| frame >= first)
        && layer.last_frame.is_some_and(|last| frame <= last)
}

fn load_mask(path: &Path, width: u32, height: u32) -> Result<Vec<u8>> {
    let image = image::open(path)
        .with_context(|| format!("failed to load layer mask {}", path.display()))?;
    if image.width() != width || image.height() != height {
        bail!(
            "layer mask {} is {}x{}, expected {}x{}",
            path.display(),
            image.width(),
            image.height(),
            width,
            height
        );
    }
    if image.color().has_alpha() {
        Ok(image.to_rgba8().pixels().map(|pixel| pixel.0[3]).collect())
    } else {
        Ok(match image {
            image::DynamicImage::ImageLuma16(mask) => mask
                .into_raw()
                .into_iter()
                .map(|value| ((u32::from(value) * 255 + 32_767) / 65_535) as u8)
                .collect(),
            other => other.to_luma8().into_raw(),
        })
    }
}

/// Grayscale morphological closing: dilation followed by erosion.
///
/// Fills small gaps and pinholes inside foreground alpha (per-frame ML jitter on
/// thin structures) while leaving the outer boundary and soft ramps unchanged.
fn close_small_gaps(mask: &mut [u8], width: usize, height: usize, radius: usize) {
    if mask.len() != width * height || radius == 0 {
        return;
    }
    let horizontal_max = |src: &[u8]| -> Vec<u8> {
        let mut out = vec![0_u8; src.len()];
        for y in 0..height {
            for x in 0..width {
                let mut value = 0_u8;
                for xx in x.saturating_sub(radius)..=(x + radius).min(width - 1) {
                    value = value.max(src[y * width + xx]);
                }
                out[y * width + x] = value;
            }
        }
        out
    };
    let vertical_max = |src: &[u8]| -> Vec<u8> {
        let mut out = vec![0_u8; src.len()];
        for y in 0..height {
            for x in 0..width {
                let mut value = 0_u8;
                for yy in y.saturating_sub(radius)..=(y + radius).min(height - 1) {
                    value = value.max(src[yy * width + x]);
                }
                out[y * width + x] = value;
            }
        }
        out
    };
    let horizontal_min = |src: &[u8]| -> Vec<u8> {
        let mut out = vec![0_u8; src.len()];
        for y in 0..height {
            for x in 0..width {
                let mut value = 255_u8;
                for xx in x.saturating_sub(radius)..=(x + radius).min(width - 1) {
                    value = value.min(src[y * width + xx]);
                }
                out[y * width + x] = value;
            }
        }
        out
    };
    let vertical_min = |src: &[u8]| -> Vec<u8> {
        let mut out = vec![0_u8; src.len()];
        for y in 0..height {
            for x in 0..width {
                let mut value = 255_u8;
                for yy in y.saturating_sub(radius)..=(y + radius).min(height - 1) {
                    value = value.min(src[yy * width + x]);
                }
                out[y * width + x] = value;
            }
        }
        out
    };
    let dilated = vertical_max(&horizontal_max(mask));
    let eroded = vertical_min(&horizontal_min(&dilated));
    mask.copy_from_slice(&eroded);
}

pub(crate) fn apply_matte_policy(mask: &mut [u8], matte: LayerMatte) {
    if matte.mode == LayerMatteMode::Optical {
        return;
    }
    let support = matte.support_threshold;
    let span = matte.solid_threshold - support;
    for alpha in mask {
        let value = f64::from(*alpha) / 255.0;
        *alpha = if value <= support {
            0
        } else if value >= matte.solid_threshold {
            255
        } else {
            let t = (value - support) / span;
            let smooth = t * t * (3.0 - 2.0 * t);
            (smooth * 255.0).round() as u8
        };
    }
}

fn save_mask(width: u32, height: u32, data: &[u8], path: &Path) -> Result<()> {
    let image: GrayImage = ImageBuffer::<Luma<u8>, _>::from_raw(width, height, data.to_vec())
        .context("invalid layer mask")?;
    image
        .save(path)
        .with_context(|| format!("failed to save layer mask {}", path.display()))
}

fn max_union(output: &mut [u8], input: &[u8]) {
    for (output, &input) in output.iter_mut().zip(input) {
        *output = (*output).max(input);
    }
}

fn alpha_over(output: &mut [u8], input: &[u8]) {
    for (output, &input) in output.iter_mut().zip(input) {
        let remaining = (255 - *output as u16) * (255 - input as u16);
        *output = (255 - (remaining + 127) / 255) as u8;
    }
}

fn subtract_alpha(output: &mut [u8], input: &[u8]) {
    for (output, &input) in output.iter_mut().zip(input) {
        *output = ((*output as u16 * (255 - input as u16) + 127) / 255) as u8;
    }
}

pub fn merge_mask(output: &mut Vec<u8>, input: &[u8]) {
    if output.is_empty() {
        output.extend_from_slice(input);
    } else {
        alpha_over(output, input);
    }
}

fn intersect(output: &mut [u8], input: &[u8]) {
    for (output, &input) in output.iter_mut().zip(input) {
        *output = (*output).min(input);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LayerInput, alpha_over, apply_matte_policy, build_tracking_exclusions, close_small_gaps,
        directly_restores_layer, exclude_outside_surface_support, has_authored_foreground,
        intersect, package, shared_authored_opaque_source_matte,
    };
    use crate::{
        model::{Mat3, MotionSample, RectF},
        scene::{
            LAYER_ARTIFACT_FORMAT, LayerArtifact, LayerArtifactKind, LayerCoordinates, LayerMatte,
            LayerMatteMode, LayerRole, SceneLayer, SegmentationPrompt, SpatialCoordinates,
        },
    };
    use image::{GrayImage, ImageBuffer, Luma};
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    #[test]
    fn fused_material_replaces_only_the_opaque_source_semantic_layer() {
        let make = |role, coordinates, mode| crate::analysis::LayerAsset {
            id: "test".into(),
            role,
            coordinates,
            kind: LayerArtifactKind::AlphaImage,
            affects_layout: false,
            affects_tracking: false,
            matte: LayerMatte {
                mode,
                ..LayerMatte::default()
            },
            path: crate::portable_path::PortablePath::bundle("mask.png").unwrap(),
            first_frame: None,
            last_frame: None,
            generator: None,
        };
        let semantic = make(
            LayerRole::Foreground,
            LayerCoordinates::SourcePixels,
            LayerMatteMode::Opaque,
        );
        let optical = make(
            LayerRole::Foreground,
            LayerCoordinates::SourcePixels,
            LayerMatteMode::Optical,
        );
        let shadow = make(
            LayerRole::Shadow,
            LayerCoordinates::SourcePixels,
            LayerMatteMode::Optical,
        );

        assert!(directly_restores_layer(&semantic, false));
        assert!(!directly_restores_layer(&semantic, true));
        assert!(directly_restores_layer(&optical, true));
        assert!(directly_restores_layer(&shadow, true));
    }

    #[test]
    fn foreground_layer_is_authoritative() {
        let input = layer_input(
            LayerRole::Foreground,
            LayerCoordinates::PlaqueCanonical,
            LayerArtifactKind::AlphaImage,
            None,
            None,
        );

        assert!(has_authored_foreground(&[input]));
    }

    #[test]
    fn shadow_or_writing_surface_is_not_authoritative_foreground() {
        let shadow = layer_input(
            LayerRole::Shadow,
            LayerCoordinates::PlaqueCanonical,
            LayerArtifactKind::AlphaImage,
            None,
            None,
        );
        let writing_surface = layer_input(
            LayerRole::WritingSurface,
            LayerCoordinates::PlaqueCanonical,
            LayerArtifactKind::AlphaImage,
            None,
            None,
        );

        assert!(!has_authored_foreground(&[shadow, writing_surface]));
    }

    #[test]
    fn projected_authored_detail_uses_only_a_shared_opaque_matte_policy() {
        let policy = LayerMatte {
            mode: LayerMatteMode::Opaque,
            support_threshold: 0.03,
            solid_threshold: 0.20,
        };
        let mut inputs = vec![
            layer_input(
                LayerRole::Foreground,
                LayerCoordinates::SourcePixels,
                LayerArtifactKind::AlphaImage,
                None,
                None,
            ),
            layer_input(
                LayerRole::Foreground,
                LayerCoordinates::SourcePixels,
                LayerArtifactKind::AlphaSequence,
                Some(0),
                Some(1),
            ),
        ];
        for input in &mut inputs {
            input.scene.matte = policy;
        }

        assert_eq!(shared_authored_opaque_source_matte(&inputs), Some(policy));

        inputs[1].scene.matte.solid_threshold = 0.35;
        assert_eq!(shared_authored_opaque_source_matte(&inputs), None);
    }

    #[test]
    fn render_only_foreground_does_not_change_tracking_support() {
        let mut input = layer_input(
            LayerRole::Foreground,
            LayerCoordinates::SourcePixels,
            LayerArtifactKind::AlphaImage,
            None,
            None,
        );
        input.scene.affects_tracking = false;

        assert!(
            !build_tracking_exclusions(
                &[input],
                Path::new("unused-automatic"),
                Path::new("unused-output"),
                4,
                2,
                1,
                None,
            )
            .unwrap()
        );
    }

    #[test]
    fn automatic_tracking_exclusions_require_confident_semantic_support() {
        let root = temporary_directory("automatic-tracking-confidence");
        let automatic_root = root.join("automatic");
        let output_root = root.join("tracking");
        fs::create_dir_all(&automatic_root).unwrap();
        ImageBuffer::<Luma<u8>, _>::from_raw(4, 2, vec![0, 32, 127, 128, 192, 255, 64, 0])
            .unwrap()
            .save(automatic_root.join("000000.png"))
            .unwrap();

        assert!(
            build_tracking_exclusions(&[], &automatic_root, &output_root, 4, 2, 1, None,).unwrap()
        );

        assert_eq!(
            image::open(output_root.join("000000.png"))
                .unwrap()
                .to_luma8()
                .into_raw(),
            vec![0, 0, 0, 255, 255, 255, 0, 0]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn composites_alpha_masks_without_losing_soft_edges() {
        let mut mask = [0, 64, 255];
        alpha_over(&mut mask, &[32, 128, 0]);
        assert_eq!(mask, [32, 160, 255]);
        intersect(&mut mask, &[255, 96, 64]);
        assert_eq!(mask, [32, 96, 64]);
    }

    #[test]
    fn optical_matte_preserves_measured_alpha() {
        let mut mask = [0, 8, 32, 128, 255];
        let original = mask;
        apply_matte_policy(&mut mask, LayerMatte::default());
        assert_eq!(mask, original);
    }

    #[test]
    fn closing_fills_small_sequence_mask_gaps_without_expanding_boundaries() {
        let width = 13;
        let height = 9;
        let mut mask = vec![0_u8; width * height];
        for y in 3..6 {
            for x in 3..10 {
                mask[y * width + x] = 255;
            }
        }
        let hole = 4 * width + 6;
        mask[hole] = 0;

        close_small_gaps(&mut mask, width, height, 2);

        assert_eq!(mask[hole], 255, "a small interior gap must be filled");
        assert_eq!(mask[0], 0, "closing must not reach the outer boundary");
        assert_eq!(mask[width + 1], 0);
        assert_eq!(mask[2 * width + 2], 0, "the boundary stays put");
        assert_eq!(mask[4 * width + 2], 0, "pixels outside stay outside");
        assert_eq!(
            mask[4 * width + 3],
            255,
            "straight boundary segments stay put"
        );
        assert_eq!(mask[4 * width + 9], 255);
        assert_eq!(mask[4 * width + 10], 0, "pixels outside stay outside");
    }

    #[test]
    fn closing_preserves_soft_alpha_ramps() {
        // A plateau-to-ramp profile with no interior local minimum is a fixed
        // point of grayscale closing.
        let mut ramp = vec![0_u8; 25];
        for y in 0..5 {
            for x in 0..5 {
                ramp[y * 5 + x] = [0, 0, 64, 128, 192][x];
            }
        }

        close_small_gaps(&mut ramp, 5, 5, 1);

        assert_eq!(
            ramp,
            [0_u8, 0, 64, 128, 192].repeat(5),
            "a gap-free profile has nothing to close"
        );
    }

    #[test]
    fn opaque_matte_calibrates_semantic_confidence_to_solid_occlusion() {
        let mut mask = [0, 7, 8, 16, 51, 128, 255];
        apply_matte_policy(
            &mut mask,
            LayerMatte {
                mode: LayerMatteMode::Opaque,
                ..LayerMatte::default()
            },
        );
        assert_eq!(mask[0], 0);
        assert_eq!(mask[1], 0);
        assert!(mask[2] < mask[3]);
        assert!(mask[3] < 255);
        assert_eq!(&mask[4..], &[255, 255, 255]);
    }

    #[test]
    fn writing_surface_support_excludes_only_non_surface_pixels() {
        let mut exclusion = [0, 64, 255, 0];
        exclude_outside_surface_support(&mut exclusion, &[255, 192, 128, 0]);
        assert_eq!(exclusion, [0, 64, 255, 255]);
    }

    #[test]
    fn shadow_is_restored_but_does_not_change_layout() {
        let root = temporary_directory("shadow");
        let input_root = root.join("input");
        let pack_root = root.join("pack");
        fs::create_dir_all(&input_root).unwrap();
        fs::create_dir_all(&pack_root).unwrap();
        let source_mask = input_root.join("mask.png");
        let image: GrayImage =
            ImageBuffer::<Luma<u8>, _>::from_raw(4, 2, vec![0, 64, 128, 0, 0, 0, 0, 0]).unwrap();
        image.save(&source_mask).unwrap();
        let input = LayerInput {
            scene: SceneLayer {
                id: "shadow".into(),
                role: LayerRole::Shadow,
                surface: "main".into(),
                in_front_of: Some("main".into()),
                artifact: None,
                active_frames: None,
                affects_layout: true,
                affects_tracking: true,
                matte: LayerMatte::default(),
                subject: crate::scene::LayerSubject::Unspecified,
                prompts: Vec::new(),
            },
            artifact_path: input_root.join("artifact.toml"),
            artifact: LayerArtifact {
                format: LAYER_ARTIFACT_FORMAT.into(),
                kind: LayerArtifactKind::AlphaImage,
                coordinates: LayerCoordinates::PlaqueCanonical,
                path: Some(PathBuf::from("mask.png")),
                pattern: None,
                first_frame: None,
                last_frame: None,
                affects_layout: true,
                generator: None,
            },
        };
        let motion = [MotionSample {
            frame: 0,
            transform: Mat3::IDENTITY,
            measurement_valid: true,
            tracked_points: 20,
            spatial_coverage: 1.0,
            uncertainty_px: 0.25,
            measurement_source: "test".into(),
            inlier_ratio: 1.0,
            reprojection_error: 0.0,
            ecc: Some(1.0),
            plaque_visibility: 1.0,
            occluder_coverage: 0.0,
        }];
        let mut content = vec![255_u8; 8];

        let packed = package(
            &[input],
            &pack_root,
            4,
            2,
            4,
            2,
            RectF {
                x: 0.0,
                y: 0.0,
                width: 4.0,
                height: 2.0,
            },
            &motion,
            &mut content,
        )
        .unwrap();

        assert_eq!(content, vec![255; 8]);
        assert_eq!(packed[0].role, LayerRole::Shadow);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_foreground_is_removed_from_layout() {
        let root = temporary_directory("layout");
        let input_root = root.join("input");
        let pack_root = root.join("pack");
        fs::create_dir_all(&input_root).unwrap();
        fs::create_dir_all(&pack_root).unwrap();
        let source_mask = input_root.join("mask.png");
        let image: GrayImage =
            ImageBuffer::<Luma<u8>, _>::from_raw(4, 2, vec![0, 64, 255, 0, 0, 0, 0, 0]).unwrap();
        image.save(&source_mask).unwrap();
        let input = LayerInput {
            scene: SceneLayer {
                id: "moss".into(),
                role: LayerRole::Foreground,
                surface: "main".into(),
                in_front_of: Some("main".into()),
                artifact: None,
                active_frames: None,
                affects_layout: true,
                affects_tracking: true,
                matte: LayerMatte::default(),
                subject: crate::scene::LayerSubject::Unspecified,
                prompts: Vec::new(),
            },
            artifact_path: input_root.join("artifact.toml"),
            artifact: LayerArtifact {
                format: LAYER_ARTIFACT_FORMAT.into(),
                kind: LayerArtifactKind::AlphaImage,
                coordinates: LayerCoordinates::PlaqueCanonical,
                path: Some(PathBuf::from("mask.png")),
                pattern: None,
                first_frame: None,
                last_frame: None,
                affects_layout: true,
                generator: None,
            },
        };
        let motion = [MotionSample {
            frame: 0,
            transform: Mat3::IDENTITY,
            measurement_valid: true,
            tracked_points: 20,
            spatial_coverage: 1.0,
            uncertainty_px: 0.25,
            measurement_source: "test".into(),
            inlier_ratio: 1.0,
            reprojection_error: 0.0,
            ecc: None,
            plaque_visibility: 1.0,
            occluder_coverage: 0.0,
        }];
        let mut content = vec![255_u8; 8];

        let packed = package(
            &[input],
            &pack_root,
            4,
            2,
            4,
            2,
            RectF {
                x: 0.0,
                y: 0.0,
                width: 4.0,
                height: 2.0,
            },
            &motion,
            &mut content,
        )
        .unwrap();

        assert_eq!(&content[..4], &[255, 191, 0, 255]);
        assert_eq!(packed.len(), 1);
        assert!(pack_root.join("layers/moss/mask.png").is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generated_layer_package_retains_worker_and_selection_provenance() {
        let root = temporary_directory("layer-provenance");
        let input_root = root.join("input");
        let pack_root = root.join("pack");
        fs::create_dir_all(&input_root).unwrap();
        fs::create_dir_all(&pack_root).unwrap();
        ImageBuffer::<Luma<u8>, _>::from_raw(4, 2, vec![0; 8])
            .unwrap()
            .save(input_root.join("mask.png"))
            .unwrap();
        fs::write(input_root.join("result.json"), "{\"backend\":\"cutie\"}\n").unwrap();
        fs::write(
            input_root.join("strategy-selection.json"),
            "{\"selected\":\"canonical\"}\n",
        )
        .unwrap();
        let input = LayerInput {
            scene: SceneLayer {
                id: "spider".into(),
                role: LayerRole::Foreground,
                surface: "main".into(),
                in_front_of: Some("main".into()),
                artifact: None,
                active_frames: None,
                affects_layout: false,
                affects_tracking: false,
                matte: LayerMatte::default(),
                subject: crate::scene::LayerSubject::Unspecified,
                prompts: vec![SegmentationPrompt {
                    frame: 0,
                    coordinates: SpatialCoordinates::SourcePixels,
                    object: Some("spider".into()),
                    concept: None,
                    box_bounds: Some([0.0, 0.0, 2.0, 2.0]),
                    positive_points: Vec::new(),
                    negative_points: Vec::new(),
                    polygon: Vec::new(),
                    quad: None,
                }],
            },
            artifact_path: input_root.join("artifact.toml"),
            artifact: LayerArtifact {
                format: LAYER_ARTIFACT_FORMAT.into(),
                kind: LayerArtifactKind::AlphaImage,
                coordinates: LayerCoordinates::SourcePixels,
                path: Some(PathBuf::from("mask.png")),
                pattern: None,
                first_frame: None,
                last_frame: None,
                affects_layout: false,
                generator: None,
            },
        };
        let motion = [MotionSample {
            frame: 0,
            transform: Mat3::IDENTITY,
            measurement_valid: true,
            tracked_points: 20,
            spatial_coverage: 1.0,
            uncertainty_px: 0.25,
            measurement_source: "test".into(),
            inlier_ratio: 1.0,
            reprojection_error: 0.0,
            ecc: Some(1.0),
            plaque_visibility: 1.0,
            occluder_coverage: 0.0,
        }];
        let mut content = vec![255_u8; 8];

        package(
            &[input],
            &pack_root,
            4,
            2,
            4,
            2,
            RectF {
                x: 0.0,
                y: 0.0,
                width: 4.0,
                height: 2.0,
            },
            &motion,
            &mut content,
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(pack_root.join("layers/spider/result.json")).unwrap(),
            "{\"backend\":\"cutie\"}\n"
        );
        assert_eq!(
            fs::read_to_string(pack_root.join("layers/spider/strategy-selection.json")).unwrap(),
            "{\"selected\":\"canonical\"}\n"
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn layer_input(
        role: LayerRole,
        coordinates: LayerCoordinates,
        kind: LayerArtifactKind,
        first_frame: Option<usize>,
        last_frame: Option<usize>,
    ) -> LayerInput {
        LayerInput {
            scene: SceneLayer {
                id: "test".into(),
                role,
                surface: "main".into(),
                in_front_of: Some("main".into()),
                artifact: None,
                active_frames: None,
                affects_layout: false,
                affects_tracking: true,
                matte: LayerMatte::default(),
                subject: crate::scene::LayerSubject::Unspecified,
                prompts: Vec::new(),
            },
            artifact_path: PathBuf::from("artifact.toml"),
            artifact: LayerArtifact {
                format: LAYER_ARTIFACT_FORMAT.into(),
                kind,
                coordinates,
                path: (kind == LayerArtifactKind::AlphaImage).then(|| "mask.png".into()),
                pattern: (kind == LayerArtifactKind::AlphaSequence)
                    .then(|| "masks/%06d.png".into()),
                first_frame,
                last_frame,
                affects_layout: false,
                generator: None,
            },
        }
    }

    fn temporary_directory(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "plaque-forge-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }
}
