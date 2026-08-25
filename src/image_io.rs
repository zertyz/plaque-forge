use crate::surface::Surface;
use anyhow::{Context, Result};
use std::path::Path;

pub fn load_rgba(path: &Path) -> Result<Surface> {
    let image = image::open(path)
        .with_context(|| format!("failed to load image {}", path.display()))?
        .to_rgba8();
    let (width, height) = image.dimensions();
    Surface::from_rgba(width, height, image.into_raw())
}

pub fn load_luma(path: &Path, expected_width: u32, expected_height: u32) -> Result<Vec<u8>> {
    let image = image::open(path)
        .with_context(|| format!("failed to load mask {}", path.display()))?
        .to_luma8();
    let (width, height) = image.dimensions();
    anyhow::ensure!(
        width == expected_width && height == expected_height,
        "mask {} is {}x{}, expected {}x{}",
        path.display(),
        width,
        height,
        expected_width,
        expected_height
    );
    Ok(image.into_raw())
}

/// Persist a single-channel u8 buffer as a grayscale PNG diagnostic artifact.
pub fn save_luma_png(width: u32, height: u32, data: &[u8], path: &Path) -> Result<()> {
    let image = image::ImageBuffer::<image::Luma<u8>, _>::from_raw(width, height, data.to_vec())
        .context("invalid luma buffer dimensions")?;
    image
        .save(path)
        .with_context(|| format!("failed to save grayscale image {}", path.display()))?;
    Ok(())
}
