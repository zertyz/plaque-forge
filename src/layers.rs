use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use image::{GrayImage, ImageBuffer, Luma};

use crate::{
    analysis::{Analysis, CONTENT_MASK_FILE, LAYERS_DIR, LayerAsset, sequence_path},
    analyze::extraction::{rectify, transformed_rect},
    color::Rgba,
    model::{MotionSample, RectF},
    refinement::{
        LayerArtifact, LayerArtifactKind, LayerCoordinates, LayerRole, RefinementLayer,
        resolve_relative,
    },
    surface::Surface,
};

pub struct LayerInput {
    pub refinement: RefinementLayer,
    pub artifact_path: std::path::PathBuf,
    pub artifact: LayerArtifact,
}

pub fn has_authored_foreground(inputs: &[LayerInput]) -> bool {
    inputs
        .iter()
        .any(|input| input.refinement.role == LayerRole::Foreground)
}

pub fn build_tracking_exclusions(
    inputs: &[LayerInput],
    automatic_root: &Path,
    output_root: &Path,
    width: u32,
    height: u32,
    frames: usize,
) -> Result<bool> {
    let foregrounds = inputs
        .iter()
        .filter(|input| {
            input.refinement.role == LayerRole::Foreground
                && input.artifact.coordinates == LayerCoordinates::SourcePixels
        })
        .collect::<Vec<_>>();
    if foregrounds.is_empty() {
        return Ok(false);
    }
    for input in &foregrounds {
        validate_frame_range(&input.artifact, frames, &input.refinement.id)?;
    }
    fs::create_dir_all(output_root)?;
    for frame in 0..frames {
        let automatic = automatic_root.join(format!("{frame:06}.png"));
        let mut combined = if automatic.is_file() {
            load_mask(&automatic, width, height)?
        } else {
            vec![0; width as usize * height as usize]
        };
        for input in &foregrounds {
            if let Some(path) = artifact_frame_path(input, frame) {
                alpha_over(&mut combined, &load_mask(&path, width, height)?);
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
        validate_frame_range(artifact, motion.len(), &input.refinement.id)?;
        let directory = Path::new(LAYERS_DIR).join(&input.refinement.id);
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
            let mask = load_mask(&source, expected_width, expected_height)?;
            let destination = match frame {
                Some(frame) => output_root.join(sequence_path(&packed_path, frame)),
                None => output_root.join(&packed_path),
            };
            save_mask(expected_width, expected_height, &mask, &destination)?;
        }

        let layer = LayerAsset {
            id: input.refinement.id.clone(),
            role: input.refinement.role,
            coordinates: artifact.coordinates,
            kind: artifact.kind,
            affects_layout: artifact.affects_layout,
            path: packed_path,
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

pub struct ForegroundReader<'a> {
    pack: &'a Analysis,
    canonical: Vec<(Surface, &'a LayerAsset)>,
    source_static: Vec<(Vec<u8>, &'a LayerAsset)>,
    sequences: Vec<&'a LayerAsset>,
}

impl<'a> ForegroundReader<'a> {
    pub fn open(pack: &'a Analysis) -> Result<Self> {
        let mut canonical = Vec::new();
        let mut source_static = Vec::new();
        let mut sequences = Vec::new();
        for layer in pack
            .manifest
            .layers
            .iter()
            .filter(|layer| matches!(layer.role, LayerRole::Foreground | LayerRole::Shadow))
        {
            match (layer.coordinates, layer.kind) {
                (LayerCoordinates::PlaqueCanonical, LayerArtifactKind::AlphaImage) => {
                    let mask = load_mask(
                        &pack.require_asset_path(&layer.path)?,
                        pack.manifest.canonical_width,
                        pack.manifest.canonical_height,
                    )?;
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
                    source_static.push((
                        load_mask(
                            &pack.require_asset_path(&layer.path)?,
                            pack.manifest.source.width,
                            pack.manifest.source.height,
                        )?,
                        layer,
                    ));
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
                    .require_asset_path(&sequence_path(&layer.path, frame))?;
                alpha_over(
                    &mut combined,
                    &load_mask(
                        &path,
                        self.pack.manifest.source.width,
                        self.pack.manifest.source.height,
                    )?,
                );
            }
        }
        Ok(combined.iter().any(|&alpha| alpha > 0).then_some(combined))
    }
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
            load_mask(&root.join(&layer.path), canonical_width, canonical_height)?
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
            LayerArtifactKind::AlphaImage => root.join(&layer.path),
            LayerArtifactKind::AlphaSequence => root.join(sequence_path(&layer.path, frame)),
        };
        let mask = load_mask(&source, source_width, source_height)?;
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
        Ok(image.to_luma8().into_raw())
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
    use super::{LayerInput, alpha_over, has_authored_foreground, intersect, package};
    use crate::{
        model::{Mat3, MotionSample, RectF},
        refinement::{
            LayerArtifact, LayerArtifactKind, LayerCoordinates, LayerRole, RefinementLayer,
        },
    };
    use image::{GrayImage, ImageBuffer, Luma};
    use std::{fs, path::PathBuf};

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
    fn composites_alpha_masks_without_losing_soft_edges() {
        let mut mask = [0, 64, 255];
        alpha_over(&mut mask, &[32, 128, 0]);
        assert_eq!(mask, [32, 160, 255]);
        intersect(&mut mask, &[255, 96, 64]);
        assert_eq!(mask, [32, 96, 64]);
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
            refinement: RefinementLayer {
                id: "shadow".into(),
                role: LayerRole::Shadow,
                plaque: "main".into(),
                in_front_of: Some("main".into()),
                artifact: None,
                active_frames: None,
                affects_layout: true,
                prompts: Vec::new(),
            },
            artifact_path: input_root.join("artifact.toml"),
            artifact: LayerArtifact {
                schema_version: 1,
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
            refinement: RefinementLayer {
                id: "moss".into(),
                role: LayerRole::Foreground,
                plaque: "main".into(),
                in_front_of: Some("main".into()),
                artifact: None,
                active_frames: None,
                affects_layout: true,
                prompts: Vec::new(),
            },
            artifact_path: input_root.join("artifact.toml"),
            artifact: LayerArtifact {
                schema_version: 1,
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

    fn layer_input(
        role: LayerRole,
        coordinates: LayerCoordinates,
        kind: LayerArtifactKind,
        first_frame: Option<usize>,
        last_frame: Option<usize>,
    ) -> LayerInput {
        LayerInput {
            refinement: RefinementLayer {
                id: "test".into(),
                role,
                plaque: "main".into(),
                in_front_of: Some("main".into()),
                artifact: None,
                active_frames: None,
                affects_layout: false,
                prompts: Vec::new(),
            },
            artifact_path: PathBuf::from("artifact.toml"),
            artifact: LayerArtifact {
                schema_version: 1,
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
