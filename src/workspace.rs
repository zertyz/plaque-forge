use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub fn scene_path(input: &Path) -> Result<PathBuf> {
    Ok(assets_dir(input)
        .join("scenes")
        .join(stem(input)?)
        .join("scene.toml"))
}

pub fn analysis_path(input: &Path) -> Result<PathBuf> {
    Ok(assets_dir(input).join("analysis").join(stem(input)?))
}

pub fn output_path(input: &Path) -> Result<PathBuf> {
    Ok(project_dir(input)
        .join("output")
        .join(format!("{}.mkv", stem(input)?)))
}

pub fn layer_path(scene: &Path, layer: &str) -> PathBuf {
    let stem = scene.parent().and_then(Path::file_name).unwrap_or_default();
    let assets = scene
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("assets"));
    assets
        .join("analysis")
        .join(stem)
        .join("layers")
        .join(layer)
}

pub fn trajectory_path(input: &Path) -> Result<PathBuf> {
    Ok(scene_path(input)?
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("artifacts")
        .join("trajectory.toml"))
}

fn stem(input: &Path) -> Result<&str> {
    input
        .file_stem()
        .and_then(|value| value.to_str())
        .with_context(|| format!("source has no usable file stem: {}", input.display()))
}

fn assets_dir(input: &Path) -> PathBuf {
    input
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

fn project_dir(input: &Path) -> PathBuf {
    let parent = assets_dir(input);
    if parent.file_name().and_then(|value| value.to_str()) == Some("assets") {
        parent
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    } else {
        parent
    }
}
