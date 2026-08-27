#![allow(clippy::collapsible_if)]
//! Preview renderer – exact CLI pipeline for single-frame live preview.
//! Uses the same typography shaping, style composition, plaque warp and
//! foreground restore as `render/mod.rs`, but for one frame at a time and
//! without FFmpeg encode. This keeps the showcase pixel-identical to CLI
//! renders while allowing FPS-aware live preview and future GPU texture
//! caching (pre-rendered `Surface` → `egui::TextureHandle`).

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;

use crate::{
    analysis::Analysis,
    application::{FitMode, TextAlign, VerticalAlign},
    render::typography,
    surface::Surface,
};

use super::styles::StyleDraft;

pub struct PreviewCache {
    pub text: String,
    pub font_path: PathBuf,
    pub style: StyleDraft,
    pub analysis_root: Option<PathBuf>,
    pub cached_text_render: Option<typography::TextRender>,
    pub dynamic_cache: HashMap<String, typography::TextRender>,
    pub last_error: Option<String>,
    // For GPU optimization: pre-rendered canonical layer cached as Surface
    // and its GPU texture handle will be managed by the UI layer (egui).
    // This cache key is (text, font, style hash) – frame-varying overlay
    // (`frame_overlay`/`frame_transform`) is applied per frame on the CPU
    // but could be moved to a shader for zero-copy GPU warp.
}

impl Default for PreviewCache {
    fn default() -> Self {
        Self::new()
    }
}

impl PreviewCache {
    pub fn new() -> Self {
        Self {
            text: "Press ENTER to change this text".into(),
            font_path: PathBuf::from("fonts/NotoSerif-Regular.ttf"),
            style: StyleDraft::default(),
            analysis_root: None,
            cached_text_render: None,
            dynamic_cache: HashMap::new(),
            last_error: None,
        }
    }

    pub fn set_text(&mut self, text: String) {
        if self.text != text {
            self.text = text;
            self.cached_text_render = None;
            self.dynamic_cache.clear();
        }
    }

    pub fn set_font(&mut self, path: PathBuf) {
        if self.font_path != path {
            self.font_path = path;
            self.cached_text_render = None;
            self.dynamic_cache.clear();
        }
    }

    pub fn set_style(&mut self, draft: StyleDraft) {
        self.style = draft;
        self.cached_text_render = None;
        self.dynamic_cache.clear();
    }

    pub fn set_analysis(&mut self, root: Option<PathBuf>) {
        self.analysis_root = root;
        self.cached_text_render = None;
        self.dynamic_cache.clear();
    }

    fn ensure_rendered(&mut self, width: u32, height: u32, mask: &[u8]) -> Result<()> {
        if self.cached_text_render.is_some() {
            return Ok(());
        }
        let style = self.style.build_style()?;
        // Resolve font path: if not found, try workspace-root-relative fallback
        let font_path = if self.font_path.is_file() {
            self.font_path.clone()
        } else {
            // Try workspace root fallback
            let root = find_workspace_root();
            let candidate = root.join(&self.font_path);
            if candidate.is_file() {
                candidate
            } else {
                let fallback = root.join("fonts/NotoSerif-Regular.ttf");
                if fallback.is_file() {
                    fallback
                } else if PathBuf::from("fonts/NotoSerif-Regular.ttf").is_file() {
                    PathBuf::from("fonts/NotoSerif-Regular.ttf")
                } else {
                    anyhow::bail!("font file does not exist: {}", self.font_path.display());
                }
            }
        };
        // Keep resolved path for future renders
        self.font_path = font_path.clone();
        let render = typography::render(typography::RenderRequest {
            width,
            height,
            mask,
            text: &self.text,
            font_path: &font_path,
            fit_mode: FitMode::Artistic,
            requested_font_size: None,
            supersampling: 2,
            target_fill: 0.94,
            max_lines: 5,
            padding_ratio: 0.03,
            line_height_ratio: 1.08,
            text_align: TextAlign::Center,
            vertical_align: VerticalAlign::Center,
            style: &style,
        });
        match render {
            Ok(r) => {
                self.cached_text_render = Some(r);
                self.last_error = None;
            }
            Err(e) => {
                self.last_error = Some(e.to_string());
                return Err(e);
            }
        }
        Ok(())
    }

    /// Render a frame exactly like `render/mod.rs` does for one frame.
    /// If `analysis` is None → fallback centered text (fullscreen greyscale path).
    pub fn render_frame(
        &mut self,
        frame: &Surface,
        time_seconds: f64,
        analysis: Option<&Analysis>,
    ) -> Result<Surface> {
        if let Some(pack) = analysis {
            self.render_frame_with_analysis(frame, time_seconds, pack)
        } else {
            self.render_frame_fallback(frame, time_seconds)
        }
    }

    fn render_frame_fallback(&mut self, frame: &Surface, time_seconds: f64) -> Result<Surface> {
        let (width, height) = (frame.width(), frame.height());
        let mask = vec![255u8; width as usize * height as usize];
        self.ensure_rendered(width, height, &mask)?;
        let text_render = self.cached_text_render.as_ref().unwrap();
        let style = self.style.build_style()?;

        let dynamic_key = style.dynamic_text(&text_render.metrics.resolved_text, time_seconds);
        let using_dynamic = dynamic_key.as_ref().is_some_and(|k| {
            k != &text_render.metrics.resolved_text && self.dynamic_cache.contains_key(k)
        });
        if let Some(ref key) = dynamic_key {
            if key != &text_render.metrics.resolved_text && !self.dynamic_cache.contains_key(key) {
                let r = typography::render(typography::RenderRequest {
                    width,
                    height,
                    mask: &mask,
                    text: key,
                    font_path: &self.font_path,
                    fit_mode: FitMode::Fixed,
                    requested_font_size: Some(text_render.metrics.font_size * 0.97),
                    supersampling: 2,
                    target_fill: 0.94,
                    max_lines: 5,
                    padding_ratio: 0.03,
                    line_height_ratio: 1.08,
                    text_align: TextAlign::Center,
                    vertical_align: VerticalAlign::Center,
                    style: &style,
                });
                if let Ok(r) = r {
                    self.dynamic_cache.insert(key.clone(), r);
                }
            }
        }
        let frame_text = dynamic_key
            .as_ref()
            .and_then(|k| self.dynamic_cache.get(k))
            .filter(|_| using_dynamic)
            .unwrap_or(text_render);

        let mut out = frame.clone();
        // No analysis → center text
        let x = (width as i32 - frame_text.layer.width() as i32) / 2;
        let y = (height as i32 - frame_text.layer.height() as i32) / 2;
        // Handle frame-varying overlay/transform even in fallback (for animations)
        let static_presented = (!style.has_frame_variation()).then(|| frame_text.layer.clone());
        let mut animated: Option<Surface> = None;
        if static_presented.is_none() || using_dynamic {
            let mut layer = frame_text.layer.clone();
            if let Some(overlay) = style
                .frame_overlay(
                    &frame_text.glyph_mask,
                    frame_text.layer.width(),
                    frame_text.layer.height(),
                    time_seconds,
                )
                .unwrap_or(None)
            {
                layer.blend_surface(&overlay, 0, 0, 1.0);
            }
            animated = Some(style.frame_transform(&layer, time_seconds).unwrap_or(layer));
        }
        let presented = if using_dynamic {
            animated.as_ref()
        } else {
            static_presented.as_ref().or(animated.as_ref())
        }
        .unwrap();
        out.blend_surface(presented, x, y, 1.0);
        Ok(out)
    }

    fn render_frame_with_analysis(
        &mut self,
        frame: &Surface,
        time_seconds: f64,
        pack: &Analysis,
    ) -> Result<Surface> {
        let canonical_width = pack.manifest.canonical_width;
        let canonical_height = pack.manifest.canonical_height;
        // Load content mask at canonical size (exact as render/mod.rs)
        let mask = match crate::image_io::load_luma(
            &pack.require_asset(crate::analysis::CONTENT_MASK_FILE)?,
            canonical_width,
            canonical_height,
        ) {
            Ok(m) => m,
            Err(_) => vec![255u8; canonical_width as usize * canonical_height as usize],
        };
        self.ensure_rendered(canonical_width, canonical_height, &mask)?;
        let text_render = self.cached_text_render.as_ref().unwrap();
        let style = self.style.build_style()?;

        // Dynamic text (scramble/split-flap) – same as render/mod.rs
        let dynamic_key = style.dynamic_text(&text_render.metrics.resolved_text, time_seconds);
        let using_dynamic = dynamic_key.as_ref().is_some_and(|k| {
            k != &text_render.metrics.resolved_text && self.dynamic_cache.contains_key(k)
        });
        if let Some(ref key) = dynamic_key {
            if key != &text_render.metrics.resolved_text && !self.dynamic_cache.contains_key(key) {
                let r = typography::render(typography::RenderRequest {
                    width: canonical_width,
                    height: canonical_height,
                    mask: &mask,
                    text: key,
                    font_path: &self.font_path,
                    fit_mode: FitMode::Fixed,
                    requested_font_size: Some(text_render.metrics.font_size * 0.97),
                    supersampling: 2,
                    target_fill: 0.94,
                    max_lines: 5,
                    padding_ratio: 0.03,
                    line_height_ratio: 1.08,
                    text_align: TextAlign::Center,
                    vertical_align: VerticalAlign::Center,
                    style: &style,
                });
                if let Ok(r) = r {
                    self.dynamic_cache.insert(key.clone(), r);
                }
            }
        }
        let frame_text = dynamic_key
            .as_ref()
            .and_then(|k| self.dynamic_cache.get(k))
            .filter(|_| using_dynamic)
            .unwrap_or(text_render);

        // Determine frame index from time (FPS-aware)
        let fps = pack.manifest.source.fps.max(f64::EPSILON);
        let frame_index =
            ((time_seconds * fps).floor() as usize).min(pack.motion.len().saturating_sub(1));
        let sample = pack
            .motion
            .get(frame_index)
            .or_else(|| pack.motion.first())
            .ok_or_else(|| anyhow::anyhow!("motion sample missing for frame {frame_index}"))?;

        let plaque_quad = crate::analyze::extraction::transformed_rect(
            pack.manifest.source_plaque_rect,
            sample.transform,
        );

        // Keep original for foreground restore (exact as render)
        let original = frame.clone();
        let mut out = frame.clone();

        // Handle injected surface if present (as in render/mod.rs)
        if let Some(injected) = &pack.manifest.injected_surface {
            if let Ok(path) = pack.require_asset_path(injected.path.as_path()) {
                if let Ok(img) = image::open(&path).map(|i| i.to_rgba8()) {
                    if img.width() == pack.manifest.canonical_width
                        && img.height() == pack.manifest.canonical_height
                    {
                        if let Ok(surface) =
                            Surface::from_rgba(img.width(), img.height(), img.into_raw())
                        {
                            let _ = out.warp_blend(
                                &surface,
                                plaque_quad,
                                sample.plaque_visibility.clamp(0.0, 1.0) as f32,
                            );
                        }
                    }
                }
            }
        }

        // Prepare text layer with frame-varying overlay/transform
        let opacity =
            sample.plaque_visibility.clamp(0.0, 1.0) as f32 * style.frame_opacity(time_seconds);
        let static_presented = (!style.has_frame_variation()).then(|| frame_text.layer.clone());
        let mut animated: Option<Surface> = None;
        if static_presented.is_none() || using_dynamic {
            let mut layer = frame_text.layer.clone();
            if let Some(overlay) = style
                .frame_overlay(
                    &frame_text.glyph_mask,
                    frame_text.layer.width(),
                    frame_text.layer.height(),
                    time_seconds,
                )
                .unwrap_or(None)
            {
                layer.blend_surface(&overlay, 0, 0, 1.0);
            }
            animated = Some(style.frame_transform(&layer, time_seconds).unwrap_or(layer));
        }
        let presented = if using_dynamic {
            animated.as_ref()
        } else {
            static_presented.as_ref().or(animated.as_ref())
        }
        .unwrap();

        // Plaque-surface effects (laser-burn, emboss) – exact as render
        if style.has_surface_effects() {
            if let Ok(canonical_plaque) =
                Surface::extract_quad(frame, plaque_quad, canonical_width, canonical_height)
            {
                let transformed_mask = style
                    .frame_transform_mask(
                        &frame_text.glyph_mask,
                        frame_text.layer.width(),
                        frame_text.layer.height(),
                        time_seconds,
                    )
                    .unwrap_or_else(|_| frame_text.glyph_mask.clone());
                if let Ok(Some(surface_layer)) =
                    style.surface_overlay(&canonical_plaque, &transformed_mask)
                {
                    let _ = out.warp_blend(&surface_layer, plaque_quad, opacity);
                }
            }
        }

        out.warp_blend(presented, plaque_quad, opacity)
            .unwrap_or(());

        // Foreground restore – exact as render/mod.rs (ForegroundReader + occluder)
        let masks_dir = pack.root.join(crate::analysis::OCCLUDER_DIR);
        let use_masks = crate::render::should_use_analysis_occluders(pack) && masks_dir.is_dir();
        let foregrounds =
            crate::layers::ForegroundReader::open(pack, use_masks).unwrap_or_else(|_| {
                // If no foreground layers, create empty reader
                crate::layers::ForegroundReader::open(pack, false).unwrap()
            });
        // We need to handle the case where open fails due to use_masks true but no layers – fallback to empty
        let mut restore: Vec<u8> = Vec::new();
        let has_foreground = !foregrounds.is_empty();
        if has_foreground {
            if let Ok(Some(mask)) = foregrounds.frame_mask(frame_index, sample.transform) {
                restore = mask;
            }
        }
        if use_masks {
            let path = masks_dir.join(format!("{frame_index:06}.png"));
            if path.is_file() {
                if let Ok(mut detail) = load_full_luma(&path, frame.width(), frame.height()) {
                    // Apply matte policy if needed (DeclaredOnly + opaque)
                    if let Some(matte) = authored_occluder_matte(pack) {
                        crate::layers::apply_matte_policy(&mut detail, matte);
                    }
                    if restore.is_empty() {
                        restore = detail;
                    } else {
                        crate::layers::merge_mask(&mut restore, &detail);
                    }
                }
            }
        }
        if !restore.is_empty() {
            let _ = out.restore_from_mask(&original, &restore);
        }

        Ok(out)
    }
}

fn find_workspace_root() -> PathBuf {
    // Try manifest dir (for showcase crate, it's plaque-forge-showcase)
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidate = manifest_dir.join("..");
    if candidate.join("assets").is_dir() {
        return candidate;
    }
    if manifest_dir.join("assets").is_dir() {
        return manifest_dir;
    }
    // Search upwards from cwd
    if let Ok(mut cur) = std::env::current_dir() {
        for _ in 0..6 {
            if cur.join("assets").is_dir() {
                return cur;
            }
            if let Some(parent) = cur.parent() {
                cur = parent.to_path_buf();
            } else {
                break;
            }
        }
    }
    manifest_dir
}

fn load_full_luma(path: &std::path::Path, width: u32, height: u32) -> Result<Vec<u8>> {
    let image = image::open(path)
        .map_err(|e| anyhow::anyhow!("failed to load occluder mask {}: {e}", path.display()))?
        .to_luma8();
    anyhow::ensure!(
        image.width() == width && image.height() == height,
        "occluder mask dimensions differ from video"
    );
    Ok(image.into_raw())
}

fn authored_occluder_matte(pack: &Analysis) -> Option<crate::scene::LayerMatte> {
    (pack.manifest.occlusion_mode == crate::scene::DepthMode::DeclaredOnly)
        .then(|| {
            pack.manifest.layers.iter().find_map(|layer| {
                (layer.role == crate::scene::LayerRole::Foreground
                    && layer.coordinates == crate::scene::LayerCoordinates::SourcePixels
                    && layer.matte.mode == crate::scene::LayerMatteMode::Opaque)
                    .then_some(layer.matte)
            })
        })
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::Surface;

    #[test]
    fn preview_cache_sets_and_invalidates() {
        let mut c = PreviewCache::new();
        c.set_text("Hello".into());
        assert!(c.cached_text_render.is_none());
        c.set_font(PathBuf::from("fonts/NotoSerif-Regular.ttf"));
        assert!(c.cached_text_render.is_none());
    }

    #[test]
    fn render_frame_without_analysis_centers() {
        let mut c = PreviewCache::new();
        c.set_text("Hi".into());
        c.font_path = PathBuf::from("fonts/NotoSerif-Regular.ttf");
        let frame = Surface::new(128, 72);
        let out = c.render_frame(&frame, 0.0, None);
        let _ = out;
    }
}
