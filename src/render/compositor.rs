//! Frame-level composition of a baked title onto decoded source frames.
//!
//! [`FrameCompositor`] owns everything the per-frame path needs — analysis
//! motion, content mask, injected surface, style, typography bake, foreground
//! readers — so both the file-rendering workflow and interactive previews
//! drive one identical compositing implementation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::analysis::{Analysis, OCCLUDER_DIR};
use crate::analyze::extraction::transformed_rect;
use crate::application::{FitMode, TextAlign, VerticalAlign};
use crate::layers::{ForegroundReader, apply_matte_policy, merge_mask};
use crate::model::TypographyMetrics;
use crate::surface::Surface;

use super::{effects, load_full_luma, typography};

/// Everything required to bake one title and composite it onto frames.
pub struct CompositorSetup {
    pub pack: Analysis,
    pub mask: Vec<u8>,
    pub injected_surface: Option<Surface>,
    pub style: effects::Style,
    pub font_path: PathBuf,
    pub text: String,
    pub fit: FitMode,
    pub requested_font_size: Option<f32>,
    pub supersampling: u32,
    pub target_fill: f32,
    pub max_lines: usize,
    pub padding_ratio: f32,
    pub line_height_ratio: f32,
    pub text_align: TextAlign,
    pub vertical_align: VerticalAlign,
}

/// Bakes the title once, then composites it onto any frame index.
pub struct FrameCompositor {
    pack: Analysis,
    mask: Vec<u8>,
    injected_surface: Option<Surface>,
    style: effects::Style,
    font_path: PathBuf,
    text_params: TextParams,
    text_render: typography::TextRender,
    static_presented: Option<Surface>,
    dynamic_target: String,
    dynamic_text_cache: HashMap<String, typography::TextRender>,
    foregrounds: ForegroundReader,
    use_masks: bool,
    masks_dir: PathBuf,
    authored_matte: Option<crate::scene::LayerMatte>,
    needs_original_frame: bool,
    source_width: u32,
    source_height: u32,
    fps: f64,
}

#[derive(Clone, Copy)]
struct TextParams {
    fit: FitMode,
    requested_font_size: Option<f32>,
    supersampling: u32,
    target_fill: f32,
    max_lines: usize,
    padding_ratio: f32,
    line_height_ratio: f32,
    text_align: TextAlign,
    vertical_align: VerticalAlign,
}

impl FrameCompositor {
    /// Shape, fit, and paint the title against the canonical writable mask.
    pub fn open(
        CompositorSetup {
            pack,
            mask,
            injected_surface,
            style,
            font_path,
            text,
            fit,
            requested_font_size,
            supersampling,
            target_fill,
            max_lines,
            padding_ratio,
            line_height_ratio,
            text_align,
            vertical_align,
        }: CompositorSetup,
    ) -> Result<Self> {
        let text_render = typography::render(typography::RenderRequest {
            width: pack.manifest.canonical_width,
            height: pack.manifest.canonical_height,
            mask: &mask,
            text: &text,
            font_path: &font_path,
            fit_mode: fit,
            requested_font_size,
            supersampling,
            target_fill,
            max_lines,
            padding_ratio,
            line_height_ratio,
            text_align,
            vertical_align,
            style: &style,
        })?;
        if text_render.metrics.missing_glyphs > 0 || text_render.metrics.fallback_glyphs > 0 {
            anyhow::bail!(
                "font cannot render the requested title deterministically: {} missing glyphs, {} fallback glyphs",
                text_render.metrics.missing_glyphs,
                text_render.metrics.fallback_glyphs
            );
        }
        let static_presented = (!style.has_frame_variation()).then(|| text_render.layer.clone());
        let use_masks =
            super::should_use_analysis_occluders(&pack) && pack.root.join(OCCLUDER_DIR).is_dir();
        let foregrounds = ForegroundReader::open(&pack, use_masks)?;
        let authored_matte = authored_occluder_matte(&pack);
        Ok(Self {
            needs_original_frame: use_masks || !foregrounds.is_empty(),
            masks_dir: pack.root.join(OCCLUDER_DIR),
            source_width: pack.manifest.source.width,
            source_height: pack.manifest.source.height,
            fps: pack.manifest.source.fps,
            dynamic_target: text_render.metrics.resolved_text.clone(),
            dynamic_text_cache: HashMap::new(),
            text_params: TextParams {
                fit: FitMode::Fixed,
                requested_font_size: Some(text_render.metrics.font_size * 0.97),
                supersampling,
                target_fill,
                max_lines,
                padding_ratio,
                line_height_ratio,
                text_align,
                vertical_align,
            },
            pack,
            mask,
            injected_surface,
            style,
            font_path,
            text_render,
            static_presented,
            foregrounds,
            use_masks,
            authored_matte,
        })
    }

    pub fn metrics(&self) -> &TypographyMetrics {
        &self.text_render.metrics
    }

    pub fn style(&self) -> &effects::Style {
        &self.style
    }

    pub fn write_canonical_mask(&self, path: &Path) -> Result<()> {
        crate::image_io::save_luma_png(
            self.text_render.layer.width(),
            self.text_render.layer.height(),
            &self.text_render.layer.alpha_mask(),
            path,
        )
    }

    pub fn uses_analysis_occluders(&self) -> bool {
        self.use_masks
    }

    /// Whether authored scene foreground layers participate in restoration.
    pub fn restores_foregrounds(&self) -> bool {
        !self.foregrounds.is_empty()
    }

    pub fn scene_foreground_layers(&self) -> usize {
        self.pack
            .manifest
            .layers
            .iter()
            .filter(|layer| layer.role == crate::scene::LayerRole::Foreground)
            .count()
    }

    pub fn into_pack(self) -> Analysis {
        self.pack
    }

    pub fn pack(&self) -> &Analysis {
        &self.pack
    }

    /// Plaque corners for `frame_index`, already transformed into screen space.
    pub fn plaque_quad(&self, frame_index: usize) -> Result<crate::geometry::Quad> {
        let sample = self.sample(frame_index)?;
        Ok(transformed_rect(
            self.pack.manifest.source_plaque_rect,
            sample.transform,
        ))
    }

    pub fn fps(&self) -> f64 {
        self.fps
    }

    fn sample(&self, frame_index: usize) -> Result<&crate::model::MotionSample> {
        self.pack
            .motion
            .get(frame_index)
            .with_context(|| format!("motion sample missing for frame {frame_index}"))
    }

    /// Composite the title (and restore foregrounds) onto one decoded frame.
    ///
    /// Mirrors the historical in-loop body of the file renderer exactly;
    /// behavioral changes here are protected by homologation contracts.
    pub fn composite(&mut self, frame: &mut Surface, frame_index: usize) -> Result<()> {
        let original = self.needs_original_frame.then(|| frame.clone());
        let sample = self.sample(frame_index)?.clone();

        let plaque_quad = transformed_rect(self.pack.manifest.source_plaque_rect, sample.transform);
        if let Some(plaque_layer) = &self.injected_surface {
            frame.warp_blend(
                plaque_layer,
                plaque_quad,
                sample.plaque_visibility.clamp(0.0, 1.0) as f32,
            )?;
        }

        // Static shaping/fitting is reused. Scramble and split-flap intentionally
        // render discrete character states, cached by state string.
        let time_seconds = frame_index as f64 / self.fps.max(f64::EPSILON);
        let dynamic_key = self.style.dynamic_text(&self.dynamic_target, time_seconds);
        if let Some(ref key) = dynamic_key
            && key != &self.dynamic_target
            && !self.dynamic_text_cache.contains_key(key)
        {
            let rendered = typography::render(typography::RenderRequest {
                width: self.pack.manifest.canonical_width,
                height: self.pack.manifest.canonical_height,
                mask: &self.mask,
                text: key,
                font_path: &self.font_path,
                fit_mode: self.text_params.fit,
                requested_font_size: self.text_params.requested_font_size,
                supersampling: self.text_params.supersampling,
                target_fill: self.text_params.target_fill,
                max_lines: self.text_params.max_lines,
                padding_ratio: self.text_params.padding_ratio,
                line_height_ratio: self.text_params.line_height_ratio,
                text_align: self.text_params.text_align,
                vertical_align: self.text_params.vertical_align,
                style: &self.style,
            });
            if let Ok(rendered) = rendered {
                self.dynamic_text_cache.insert(key.clone(), rendered);
            }
        }
        let using_dynamic = dynamic_key.as_ref().is_some_and(|key| {
            key != &self.dynamic_target && self.dynamic_text_cache.contains_key(key)
        });
        let frame_text = dynamic_key
            .as_ref()
            .and_then(|key| self.dynamic_text_cache.get(key))
            .filter(|_| using_dynamic)
            .unwrap_or(&self.text_render);

        let opacity = sample.plaque_visibility.clamp(0.0, 1.0) as f32
            * self.style.frame_opacity(time_seconds);
        let animated_presented = if self.static_presented.is_none() || using_dynamic {
            let mut layer = frame_text.layer.clone();
            if let Some(overlay) = self.style.frame_overlay(
                &frame_text.glyph_mask,
                frame_text.layer.width(),
                frame_text.layer.height(),
                time_seconds,
            )? {
                layer.blend_surface(&overlay, 0, 0, 1.0);
            }
            Some(self.style.frame_transform(&layer, time_seconds)?)
        } else {
            None
        };
        let presented = if using_dynamic {
            animated_presented.as_ref()
        } else {
            self.static_presented
                .as_ref()
                .or(animated_presented.as_ref())
        }
        .context("title presentation was not created")?;

        if self.style.has_surface_effects() {
            let canonical_plaque = Surface::extract_quad(
                frame,
                plaque_quad,
                self.pack.manifest.canonical_width,
                self.pack.manifest.canonical_height,
            )?;
            let transformed_mask = self.style.frame_transform_mask(
                &frame_text.glyph_mask,
                frame_text.layer.width(),
                frame_text.layer.height(),
                time_seconds,
            )?;
            if let Some(surface_layer) = self
                .style
                .surface_overlay(&canonical_plaque, &transformed_mask)?
            {
                frame.warp_blend(&surface_layer, plaque_quad, opacity)?;
            }
        }

        frame.warp_blend(presented, plaque_quad, opacity)?;
        let mut restore = self
            .foregrounds
            .frame_mask(frame_index, sample.transform)?
            .unwrap_or_default();
        if self.use_masks {
            let path = self.masks_dir.join(format!("{frame_index:06}.png"));
            if path.exists() {
                let mut detail = load_full_luma(&path, self.source_width, self.source_height)?;
                if let Some(matte) = self.authored_matte {
                    apply_matte_policy(&mut detail, matte);
                }
                merge_mask(&mut restore, &detail);
            }
        }
        if !restore.is_empty() {
            frame.restore_from_mask(
                original
                    .as_ref()
                    .context("foreground restoration source is unavailable")?,
                &restore,
            )?;
        }
        Ok(())
    }
}

/// Load (and canonical-size-check) the injected plaque declared by the
/// analysis, shared by file rendering and interactive preview.
pub fn load_injected_surface(pack: &Analysis) -> Result<Option<Surface>> {
    let Some(asset) = pack.manifest.injected_surface.as_ref() else {
        return Ok(None);
    };
    let path = pack.require_asset_path(asset.path.as_path())?;
    let image = image::open(&path)
        .with_context(|| format!("failed to load injected plaque {}", path.display()))?
        .to_rgba8();
    anyhow::ensure!(
        image.width() == pack.manifest.canonical_width
            && image.height() == pack.manifest.canonical_height,
        "injected plaque dimensions do not match canonical analysis"
    );
    Ok(Some(Surface::from_rgba(
        image.width(),
        image.height(),
        image.into_raw(),
    )?))
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
