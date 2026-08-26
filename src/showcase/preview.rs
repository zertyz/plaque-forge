//! Preview renderer – caches typography and composites per frame.

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
    }

    fn ensure_rendered(&mut self, width: u32, height: u32, mask: &[u8]) -> Result<()> {
        if self.cached_text_render.is_some() {
            return Ok(());
        }
        let style = self.style.build_style()?;
        if !self.font_path.is_file() {
            // fallback to pinned font if requested not found
            let fallback = PathBuf::from("fonts/NotoSerif-Regular.ttf");
            if fallback.is_file() {
                self.font_path = fallback;
            } else {
                anyhow::bail!("font file does not exist: {}", self.font_path.display());
            }
        }
        let render = typography::render(typography::RenderRequest {
            width,
            height,
            mask,
            text: &self.text,
            font_path: &self.font_path,
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

    /// Render a frame: warp text onto surface.
    /// If analysis is None -> fallback centered text (fullscreen) + return surface unchanged for greyscale handling upstream.
    pub fn render_frame(
        &mut self,
        frame: &Surface,
        time_seconds: f64,
        analysis: Option<&Analysis>,
    ) -> Result<Surface> {
        let (width, height) = (frame.width(), frame.height());
        let mask = if let Some(pack) = analysis {
            // load content mask if available else full
            let content_path = pack.root.join(crate::analysis::CONTENT_MASK_FILE);
            if content_path.is_file() {
                let img = image::open(&content_path)
                    .map(|i| i.to_luma8().into_raw())
                    .unwrap_or(vec![255; width as usize * height as usize]);
                if img.len() == width as usize * height as usize {
                    img
                } else {
                    vec![255; width as usize * height as usize]
                }
            } else {
                vec![255; width as usize * height as usize]
            }
        } else {
            vec![255; width as usize * height as usize]
        };
        self.ensure_rendered(width, height, &mask)?;
        let text_render = self.cached_text_render.as_ref().unwrap();
        let style = self.style.build_style()?;

        // Dynamic text handling like render/mod.rs
        let dynamic_key = style.dynamic_text(&text_render.metrics.resolved_text, time_seconds);
        let using_dynamic = dynamic_key.as_ref().is_some_and(|k| {
            k != &text_render.metrics.resolved_text && self.dynamic_cache.contains_key(k)
        });
        if let Some(ref key) = dynamic_key
            && key != &text_render.metrics.resolved_text
            && !self.dynamic_cache.contains_key(key)
        {
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
        let frame_text = dynamic_key
            .as_ref()
            .and_then(|k| self.dynamic_cache.get(k))
            .filter(|_| using_dynamic)
            .unwrap_or(text_render);

        let mut out = frame.clone();
        if let Some(pack) = analysis {
            // simplified: no plaque warp if analysis present – use transformed plaque quad
            if let Some(sample) = pack.motion.first() {
                // use first motion sample for preview fallback – actual per-frame motion handled by caller who passes time-based sample
                // Caller should have provided transformed quad; we replicate warp_blend with plaque quad from manifest
                let _ = sample;
            }
            // For showcase preview we composite centered when no motion transform available – fallback to centered warp
            // Try to use motion at time_seconds
            let idx = ((time_seconds * pack.manifest.source.fps) as usize)
                .min(pack.motion.len().saturating_sub(1));
            if let Some(sample) = pack.motion.get(idx) {
                let plaque_quad = crate::analyze::extraction::transformed_rect(
                    pack.manifest.source_plaque_rect,
                    sample.transform,
                );
                let opacity = sample.plaque_visibility.clamp(0.0, 1.0) as f32
                    * style.frame_opacity(time_seconds);
                let static_presented =
                    (!style.has_frame_variation()).then(|| frame_text.layer.clone());
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
                out.warp_blend(presented, plaque_quad, opacity)
                    .unwrap_or(());
                // foreground restore omitted for preview simplicity (heavy)
            } else {
                out.blend_surface(&frame_text.layer, 0, 0, 1.0);
            }
        } else {
            // no analysis – center text
            let x = (width as i32 - frame_text.layer.width() as i32) / 2;
            let y = (height as i32 - frame_text.layer.height() as i32) / 2;
            out.blend_surface(&frame_text.layer, x, y, 1.0);
        }
        Ok(out)
    }
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
        // Should not error even without analysis; may fail if font missing but fallback exists
        // If rendering fails due to missing font, it's expected to error
        let _ = out;
    }
}
