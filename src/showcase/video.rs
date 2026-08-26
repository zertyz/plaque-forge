//! Video player – opencv VideoCapture wrapper, FPS-aware loop.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use opencv::prelude::*;
use opencv::videoio::{VideoCapture, CAP_ANY, CAP_PROP_FPS, CAP_PROP_FRAME_COUNT, CAP_PROP_FRAME_WIDTH, CAP_PROP_FRAME_HEIGHT, CAP_PROP_POS_FRAMES};

use crate::surface::Surface;
use crate::color::Rgba;

/// Simple synchronous player; UI thread drives `next_frame` at FPS cadence.
pub struct VideoPlayer {
    capture: Option<VideoCapture>,
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub frames: usize,
    pub current_frame: usize,
    pub has_analysis: bool,
}

impl VideoPlayer {
    pub fn open(path: &Path) -> Result<Self> {
        let mut cap = VideoCapture::from_file(&*path.to_string_lossy(), CAP_ANY)
            .map_err(|e| anyhow::anyhow!("failed to open video {}: {e}", path.display()))?;
        if !cap.is_opened()? {
            anyhow::bail!("video could not be opened: {}", path.display());
        }
        cap.set(opencv::videoio::CAP_PROP_ORIENTATION_AUTO, 0.0)?;
        let width = cap.get(CAP_PROP_FRAME_WIDTH)? as u32;
        let height = cap.get(CAP_PROP_FRAME_HEIGHT)? as u32;
        let fps = cap.get(CAP_PROP_FPS)?;
        let fps = if fps.is_finite() && fps > 0.0 { fps } else { 24.0 };
        let frames = cap.get(CAP_PROP_FRAME_COUNT)? as usize;
        let analysis_path = crate::workspace::analysis_path(path).unwrap_or(PathBuf::from("assets/analysis/unknown"));
        let has_analysis = analysis_path.is_dir();
        Ok(Self {
            capture: Some(cap),
            path: path.to_path_buf(),
            width,
            height,
            fps: fps as f64,
            frames: if frames == 0 { 1 } else { frames },
            current_frame: 0,
            has_analysis,
        })
    }

    pub fn next_surface(&mut self) -> Result<Option<Surface>> {
        let cap = match &mut self.capture {
            Some(c) => c,
            None => return Ok(None),
        };
        let mut mat = opencv::core::Mat::default();
        let ok = cap.read(&mut mat)?;
        if !ok || mat.empty() {
            // loop
            cap.set(CAP_PROP_POS_FRAMES, 0.0)?;
            self.current_frame = 0;
            let ok2 = cap.read(&mut mat)?;
            if !ok2 || mat.empty() {
                return Ok(None);
            }
        }
        self.current_frame = cap.get(CAP_PROP_POS_FRAMES)? as usize;
        // Convert BGR Mat to RGBA Surface
        let mut rgba = opencv::core::Mat::default();
        opencv::imgproc::cvt_color(&mat, &mut rgba, opencv::imgproc::COLOR_BGR2RGBA, 0, opencv::core::AlgorithmHint::ALGO_HINT_DEFAULT)?;
        let w = rgba.cols() as u32;
        let h = rgba.rows() as u32;
        let bytes = rgba.data_bytes()?.to_vec();
        let surface = Surface::from_rgba(w, h, bytes)?;
        Ok(Some(surface))
    }

    pub fn seek(&mut self, frame: usize) -> Result<()> {
        if let Some(cap) = &mut self.capture {
            cap.set(CAP_PROP_POS_FRAMES, frame as f64)?;
            self.current_frame = frame;
        }
        Ok(())
    }

    pub fn frame_duration(&self) -> Duration {
        Duration::from_secs_f64(1.0 / self.fps.max(1.0))
    }

    pub fn time_seconds(&self) -> f64 {
        self.current_frame as f64 / self.fps.max(1.0)
    }
}

// Mock player for tests – no opencv.
pub struct MockPlayer {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub frames: usize,
}

impl MockPlayer {
    pub fn new(path: &str, fps: f64) -> Self {
        Self { path: PathBuf::from(path), width: 1280, height: 720, fps, frames: 100 }
    }
    pub fn frame_duration(&self) -> Duration {
        Duration::from_secs_f64(1.0 / self.fps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_frame_duration() {
        let p = MockPlayer::new("a.mp4", 24.0);
        let d = p.frame_duration();
        assert!((d.as_secs_f64() - 1.0/24.0).abs() < 1e-6);
    }
}
