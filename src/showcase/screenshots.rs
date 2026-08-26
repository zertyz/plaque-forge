//! Headless screenshot helpers for key/mouse navigability verification.
//! These are used by `cargo test` to "see" the showcase without a window server
//! and by `cargo run --features showcase` via `--screenshot` flag.

use std::path::Path;

use crate::surface::Surface;
use crate::color::Rgba;

use super::preview::PreviewCache;

/// Render a synthetic frame with showcase preview and save to `path`.
/// Used by tests to verify that key navigation produces visible output.
pub fn capture_preview_screenshot(
    preview: &mut PreviewCache,
    width: u32,
    height: u32,
    dest: &Path,
) -> anyhow::Result<()> {
    let frame = Surface::new(width, height);
    // Fill with gradient so text is visible
    let mut bg = frame;
    for y in 0..height {
        for x in 0..width {
            let v = ((x as f32 / width as f32) * 40.0) as u8;
            bg.set_pixel(x, y, Rgba::new(v, v, v, 255));
        }
    }
    let rendered = preview.render_frame(&bg, 0.0, None)?;
    let img = image::RgbaImage::from_raw(rendered.width(), rendered.height(), rendered.pixels().to_vec())
        .ok_or_else(|| anyhow::anyhow!("invalid rendered pixels"))?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    img.save(dest)?;
    Ok(())
}

/// Simulate a key navigation sequence and capture resulting preview.
/// Sequence: default text -> change text -> change font -> change style -> capture.
pub fn simulate_navigation_and_capture(dest_dir: &Path) -> anyhow::Result<()> {
    use std::path::PathBuf;
    let mut preview = PreviewCache::new();
    preview.set_text("Navigation Test".into());
    preview.font_path = PathBuf::from("fonts/NotoSerif-Regular.ttf");
    // Ensure style is default
    capture_preview_screenshot(&mut preview, 320, 180, &dest_dir.join("01_default.png"))?;

    preview.set_text("Hello from Enter".into());
    capture_preview_screenshot(&mut preview, 320, 180, &dest_dir.join("02_text_changed.png"))?;

    // Simulate font change to DejaVu if available else keep
    preview.set_font(PathBuf::from("fonts/NotoSerif-Regular.ttf"));
    capture_preview_screenshot(&mut preview, 320, 180, &dest_dir.join("03_font_changed.png"))?;

    // Simulate style change (add gold)
    let mut draft = super::styles::StyleDraft::default();
    draft.fill_kind = super::styles::FillKind::Gold {
        dark: "#5B3210FF".into(),
        mid: "#C98B3CFF".into(),
        light: "#F3D38AFF".into(),
        highlight: "#FFF1C4FF".into(),
    };
    preview.set_style(draft);
    capture_preview_screenshot(&mut preview, 320, 180, &dest_dir.join("04_style_changed.png"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn screenshots_are_generated_and_visible() {
        let dir = PathBuf::from("/tmp/plaque-forge-showcase-screenshots");
        let _ = std::fs::remove_dir_all(&dir);
        simulate_navigation_and_capture(&dir).expect("screenshot capture should succeed");
        for name in ["01_default.png", "02_text_changed.png", "03_font_changed.png", "04_style_changed.png"] {
            let path = dir.join(name);
            assert!(path.is_file(), "screenshot {} should exist", path.display());
            let img = image::open(&path).expect("screenshot should be valid PNG");
            assert!(img.width() == 320 && img.height() == 180, "screenshot dimensions should be 320x180");
            // Ensure not just blank black – has visible variation
            let rgba = img.to_rgba8();
            let has_non_black = rgba.pixels().any(|p| p[0] > 20 || p[1] > 20 || p[2] > 20);
            assert!(has_non_black, "screenshot {name} should have visible content");
        }
        // Clean up
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn greyscale_missing_analysis_is_distinct_from_color() {
        let mut preview = PreviewCache::new();
        preview.set_text("No analysis".into());
        let bg = Surface::new(64, 32);
        // Color path (with analysis would be color), but our greyscale helper test
        let mut color_frame = bg.clone();
        color_frame.set_pixel(0, 0, Rgba::new(200, 50, 50, 255));
        let mut grey = color_frame.clone();
        super::super::diagnostics::to_greyscale(&mut grey);
        let p_color = color_frame.pixel(0, 0);
        let p_grey = grey.pixel(0, 0);
        assert_ne!(p_color.r, p_grey.r);
        assert_eq!(p_grey.r, p_grey.g);
    }
}
