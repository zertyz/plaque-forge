use crate::{
    color::Rgba,
    geometry::{homography, Quad},
};
use anyhow::{bail, Result};
use std::sync::LazyLock;

static SRGB_TO_LINEAR_TABLE: LazyLock<[f32; 256]> = LazyLock::new(|| {
    std::array::from_fn(|index| {
        let encoded = index as f32 / 255.0;
        if encoded <= 0.04045 {
            encoded / 12.92
        } else {
            ((encoded + 0.055) / 1.055).powf(2.4)
        }
    })
});

static LINEAR_TO_SRGB_TABLE: LazyLock<Box<[u8; 65_536]>> = LazyLock::new(|| {
    Box::new(std::array::from_fn(|index| {
        let linear = index as f32 / 65_535.0;
        let encoded = if linear <= 0.003_130_8 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        };
        (encoded * 255.0).round().clamp(0.0, 255.0) as u8
    }))
});

static MIXTURE_BOUNDS_TABLE: LazyLock<Box<[(u8, u8); 65_536]>> = LazyLock::new(|| {
    let srgb_to_lin = *SRGB_TO_LINEAR_TABLE;
    let lin_to_srgb = &**LINEAR_TO_SRGB_TABLE;
    let mut bounds = Box::new([(0u8, 0u8); 65_536]);
    for index in 0..=u16::MAX as usize {
        let known = (index >> 8) as u8;
        let known_weight = (index & 0xff) as f32 / 255.0;
        let known_contribution = srgb_to_lin[known as usize] * known_weight;
        let idx_min = (known_contribution.clamp(0.0, 1.0) * 65_535.0).round() as usize;
        let idx_max =
            ((known_contribution + 1.0 - known_weight).clamp(0.0, 1.0) * 65_535.0).round() as usize;
        bounds[index] = (
            lin_to_srgb[idx_min].saturating_sub(1),
            lin_to_srgb[idx_max].saturating_add(1),
        );
    }
    bounds
});

#[derive(Clone, Debug)]
pub struct Surface {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Surface {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; width as usize * height as usize * 4],
        }
    }

    pub fn from_rgba(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self> {
        let expected = width as usize * height as usize * 4;
        if pixels.len() != expected {
            bail!(
                "RGBA buffer has {} bytes, expected {expected} for {width}x{height}",
                pixels.len()
            );
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Restores pixels from `original` according to a luma mask. 255 means exact
    /// original, 0 means keep the composed pixel, intermediate values feather.
    pub fn restore_from_mask(&mut self, original: &Surface, mask: &[u8]) -> Result<()> {
        if self.width != original.width || self.height != original.height {
            bail!("restore source dimensions do not match destination");
        }
        if mask.len() != self.width as usize * self.height as usize {
            bail!("restore mask dimensions do not match frame");
        }
        let width = self.width as usize;
        let height = self.height as usize;
        let workers = parallel_workers(width * height);
        let orig_pixels = &original.pixels;
        // Row-sharded: each row is independent, preserves bitwise identity
        // because `blend_over` is per-pixel and order-independent.
        run_rows(self, 0, height as u32, workers, |band, band_top| {
            let band_rows = band.len() / (width * 4);
            let start = band_top as usize * width;
            let mask_slice = &mask[start..start + band_rows * width];
            let orig_start = start * 4;
            let orig_slice = &orig_pixels[orig_start..orig_start + band_rows * width * 4];
            for ((dst, src), &alpha) in band
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(orig_slice.as_chunks::<4>().0.iter())
                .zip(mask_slice)
            {
                if alpha == 255 && src[3] == 255 {
                    dst.copy_from_slice(src);
                    continue;
                }
                blend_over(
                    dst,
                    Rgba::new(src[0], src[1], src[2], src[3]),
                    alpha as f32 / 255.0,
                );
            }
        });
        Ok(())
    }

    pub fn apply_alpha_mask(&mut self, mask: &[u8]) -> Result<()> {
        let expected = self.width as usize * self.height as usize;
        if mask.len() != expected {
            bail!(
                "alpha mask has {} bytes, expected {expected} for {}x{}",
                mask.len(),
                self.width,
                self.height
            );
        }
        let width = self.width as usize;
        let height = self.height as usize;
        let workers = parallel_workers(width * height);
        run_rows(self, 0, height as u32, workers, |band, band_top| {
            let band_rows = band.len() / (width * 4);
            let start = band_top as usize * width;
            let mask_slice = &mask[start..start + band_rows * width];
            for (pixel, &mask_alpha) in band.as_chunks_mut::<4>().0.iter_mut().zip(mask_slice) {
                pixel[3] = ((pixel[3] as u16 * mask_alpha as u16 + 127) / 255) as u8;
            }
        });
        Ok(())
    }

    pub fn pixel(&self, x: u32, y: u32) -> Rgba {
        let i = self.index(x, y);
        Rgba::new(
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        )
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, color: Rgba) {
        if x >= self.width || y >= self.height {
            return;
        }
        let i = self.index(x, y);
        self.pixels[i..i + 4].copy_from_slice(&color.as_array());
    }

    pub fn blend_pixel(&mut self, x: i32, y: i32, color: Rgba, opacity: f32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let i = self.index(x as u32, y as u32);
        blend_over(&mut self.pixels[i..i + 4], color, opacity);
    }

    pub fn blend_surface(&mut self, source: &Surface, dx: i32, dy: i32, opacity: f32) {
        if opacity <= 0.0 {
            return;
        }
        let left = dx.max(0);
        let top = dy.max(0);
        let right = (dx + source.width as i32).min(self.width as i32);
        let bottom = (dy + source.height as i32).min(self.height as i32);
        if right <= left || bottom <= top {
            return;
        }

        // Hoist clamped opacity once per call (was recomputed per pixel via
        // `opacity.clamp` inside `blend_over`). Bitwise identical because
        // `blend_over` clamps identically.
        let opacity = opacity.clamp(0.0, 1.0);
        if opacity <= 0.0 {
            return;
        }
        let area = (right - left) as usize * (bottom - top) as usize;
        let workers = parallel_workers(area);
        let src_pixels = &source.pixels;
        let src_width = source.width as usize;
        let dst_width = self.width as usize;
        let left_usize = left as usize;
        let right_usize = right as usize;

        run_rows(
            self,
            top as u32,
            bottom as u32,
            workers,
            |band, band_top| {
                let band_rows = band.len() / (dst_width * 4);
                for row_offset in 0..band_rows {
                    let y = band_top as i32 + row_offset as i32;
                    let dst_row_start = row_offset * dst_width * 4;
                    let dst_row = &mut band[dst_row_start..dst_row_start + dst_width * 4];
                    let src_y = (y - dy) as usize;
                    // `src_y` is in-bounds because `top..bottom` was clipped
                    // against `src` height via `bottom`.
                    let src_row_start = src_y * src_width * 4;
                    let src_row = &src_pixels[src_row_start..src_row_start + src_width * 4];
                    for x in left_usize..right_usize {
                        let src_x = x as i32 - dx;
                        let sx_byte = (src_x as usize) * 4;
                        // Bounds already validated, but keep graceful fallback
                        let Some(pixel) = src_row
                            .get(sx_byte..sx_byte + 4)
                            .and_then(|s| s.try_into().ok())
                        else {
                            continue;
                        };
                        let pixel: [u8; 4] = pixel;
                        if pixel[3] > 0 {
                            let dx4 = x * 4;
                            blend_over(
                                &mut dst_row[dx4..dx4 + 4],
                                Rgba::new(pixel[0], pixel[1], pixel[2], pixel[3]),
                                opacity,
                            );
                        }
                    }
                }
            },
        );
    }

    /// Warps the whole source canvas to the destination quadrilateral and alpha-composites it.
    pub fn warp_blend(&mut self, source: &Surface, destination: Quad, opacity: f32) -> Result<()> {
        if opacity <= 0.0 {
            return Ok(());
        }
        let workers = parallel_workers(destination_area(destination));
        self.warp_blend_with_workers(source, destination, opacity, workers)
    }

    /// [`Surface::warp_blend`] with an explicit worker budget. Any worker count
    /// must produce bitwise-identical output because every destination pixel is
    /// computed independently by the same expression sequence.
    pub(crate) fn warp_blend_with_workers(
        &mut self,
        source: &Surface,
        destination: Quad,
        opacity: f32,
        workers: usize,
    ) -> Result<()> {
        let source_quad = Quad::from_rect(
            0.0,
            0.0,
            (source.width.saturating_sub(1)) as f64,
            (source.height.saturating_sub(1)) as f64,
        );
        let inverse = homography(source_quad, destination)?.inverse()?;
        let (min_x, min_y, max_x, max_y) = destination.bounds();
        let left = min_x.floor().max(0.0) as i32;
        let top = min_y.floor().max(0.0) as i32;
        let right = max_x.ceil().min(self.width as f64 - 1.0) as i32;
        let bottom = max_y.ceil().min(self.height as f64 - 1.0) as i32;
        if left > right || top > bottom {
            return Ok(());
        }
        let self_width_bytes = self.width as usize * 4;
        let job = ProjectiveResample {
            source,
            matrix: inverse.m,
            transform_shift: 0.5,
            sample_shift: -0.5,
            left,
            right,
            mode: Resampling::Blend(opacity),
        };
        run_rows(
            self,
            top as u32,
            bottom as u32 + 1,
            workers,
            |band, band_top| {
                job.run(band, band_top, self_width_bytes);
            },
        );
        Ok(())
    }

    /// Pulls a planar region from a video frame into a canonical rectangular canvas.
    pub fn extract_quad(
        frame: &Surface,
        source_quad: Quad,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let workers = parallel_workers(width as usize * height as usize);
        Self::extract_quad_with_workers(frame, source_quad, width, height, workers)
    }

    /// [`Surface::extract_quad`] with an explicit worker budget. Any worker count
    /// must produce bitwise-identical output because every canonical pixel is
    /// computed independently by the same expression sequence.
    pub(crate) fn extract_quad_with_workers(
        frame: &Surface,
        source_quad: Quad,
        width: u32,
        height: u32,
        workers: usize,
    ) -> Result<Self> {
        let canonical = Quad::from_rect(
            0.0,
            0.0,
            (width.saturating_sub(1)) as f64,
            (height.saturating_sub(1)) as f64,
        );
        let to_frame = homography(canonical, source_quad)?;
        let mut result = Self::new(width, height);
        if height > 0 && width > 0 {
            let row_bytes = width as usize * 4;
            let job = ProjectiveResample {
                source: frame,
                matrix: to_frame.m,
                transform_shift: 0.0,
                sample_shift: 0.0,
                left: 0,
                right: width as i32 - 1,
                mode: Resampling::Overwrite,
            };
            run_rows(&mut result, 0, height, workers, |band, band_top| {
                job.run(band, band_top, row_bytes);
            });
        }
        Ok(result)
    }

    pub fn alpha_bounds(&self) -> Option<(u32, u32, u32, u32)> {
        let mut min_x = self.width;
        let mut min_y = self.height;
        let mut max_x = 0;
        let mut max_y = 0;
        let mut any = false;
        for y in 0..self.height {
            for x in 0..self.width {
                if self.pixels[self.index(x, y) + 3] != 0 {
                    any = true;
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
            }
        }
        any.then_some((min_x, min_y, max_x, max_y))
    }

    pub fn alpha_mask(&self) -> Vec<u8> {
        self.pixels
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| pixel[3])
            .collect()
    }

    pub fn recolor(&mut self, color: Rgba) {
        for pixel in self.pixels.as_chunks_mut::<4>().0.iter_mut() {
            pixel[0] = color.r;
            pixel[1] = color.g;
            pixel[2] = color.b;
            pixel[3] = ((pixel[3] as u16 * color.a as u16 + 127) / 255) as u8;
        }
    }

    pub fn from_alpha_mask(width: u32, height: u32, mask: &[u8], color: Rgba) -> Result<Self> {
        if mask.len() != width as usize * height as usize {
            bail!("alpha mask dimensions do not match its length");
        }
        let mut surface = Self::new(width, height);
        for (pixel, &alpha) in surface.pixels.as_chunks_mut::<4>().0.iter_mut().zip(mask) {
            pixel[0] = color.r;
            pixel[1] = color.g;
            pixel[2] = color.b;
            pixel[3] = ((alpha as u16 * color.a as u16 + 127) / 255) as u8;
        }
        Ok(surface)
    }

    pub fn box_blur(&self, radius: u32, passes: u32) -> Self {
        if radius == 0 || passes == 0 {
            return self.clone();
        }
        let mut data = self.pixels.clone();
        let mut scratch = vec![0_u8; data.len()];
        for _ in 0..passes {
            blur_rgba_horizontal(
                &data,
                &mut scratch,
                self.width as usize,
                self.height as usize,
                radius as usize,
            );
            blur_rgba_vertical(
                &scratch,
                &mut data,
                self.width as usize,
                self.height as usize,
                radius as usize,
            );
        }
        Self {
            width: self.width,
            height: self.height,
            pixels: data,
        }
    }

    fn index(&self, x: u32, y: u32) -> usize {
        (y as usize * self.width as usize + x as usize) * 4
    }
}

#[inline(always)]
fn blend_over(destination: &mut [u8], source: Rgba, opacity: f32) {
    let sa = source.a as f32 / 255.0 * opacity.clamp(0.0, 1.0);
    if sa <= 0.0 {
        return;
    }
    let da = destination[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= f32::EPSILON {
        destination.copy_from_slice(&[0, 0, 0, 0]);
        return;
    }
    let source_channels = [source.r, source.g, source.b];
    for channel in 0..3 {
        let source_linear = srgb_to_linear(source_channels[channel]);
        let destination_linear = srgb_to_linear(destination[channel]);
        destination[channel] =
            linear_to_srgb((source_linear * sa + destination_linear * da * (1.0 - sa)) / out_a);
    }
    destination[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

#[inline(always)]
fn srgb_to_linear(value: u8) -> f32 {
    SRGB_TO_LINEAR_TABLE[value as usize]
}

#[inline(always)]
fn linear_to_srgb(value: f32) -> u8 {
    let index = (value.clamp(0.0, 1.0) * 65_535.0).round() as usize;
    LINEAR_TO_SRGB_TABLE[index]
}

/// Distance outside all possible linear-light mixtures containing `known` at
/// exactly `known_weight`. A one-level encoded allowance covers LUT/FFmpeg rounding.
#[inline(always)]
pub(crate) fn constrained_linear_mixture_error(known: u8, observed: u8, known_weight: u8) -> u64 {
    if known_weight == 255 {
        return known.abs_diff(observed).saturating_sub(1) as u64;
    }
    let (minimum, maximum) =
        MIXTURE_BOUNDS_TABLE[usize::from(known) * 256 + usize::from(known_weight)];
    if observed < minimum {
        u64::from(minimum - observed)
    } else if observed > maximum {
        u64::from(observed - maximum)
    } else {
        0
    }
}

/// Whether a resampled pixel overwrites the destination or is alpha-composited.
#[derive(Clone, Copy)]
enum Resampling {
    Overwrite,
    Blend(f32),
}

/// Areas below this many destination pixels stay on the calling thread; thread
/// coordination would cost more than the parallelism saves.
const PARALLEL_AREA_THRESHOLD: usize = 96 * 1024;

fn parallel_workers(area: usize) -> usize {
    if area < PARALLEL_AREA_THRESHOLD {
        return 1;
    }
    match std::thread::available_parallelism() {
        Ok(count) => count.get(),
        // `available_parallelism` can fail in containers without cgroup
        // information. It is a deployment observation, not a program error:
        // fall back to single-threaded execution and avoid a hard failure
        // when the crate is used as a library.
        Err(error) => {
            // Best-effort diagnostic; verification callers already have a
            // `ProgressReporter`, but `surface` is a low-level primitive that
            // must not panic when used as a library. `eprintln` is
            // `BrokenPipe`-tolerant by convention and never aborts.
            eprintln!("warning: available_parallelism failed ({error}), using 1 thread");
            1
        }
    }
}

fn destination_area(quad: Quad) -> usize {
    let (min_x, min_y, max_x, max_y) = quad.bounds();
    let width = (max_x - min_x).max(0.0).ceil() as usize;
    let height = (max_y - min_y).max(0.0).ceil() as usize;
    width.saturating_mul(height)
}

/// Applies `process` to contiguous row bands of `surface` between `top` and
/// `bottom` (exclusive), optionally on scoped worker threads. Bands never overlap,
/// and each pixel's value depends only on its own inputs, so any worker count
/// yields identical bytes.
fn run_rows(
    surface: &mut Surface,
    top: u32,
    bottom: u32,
    workers: usize,
    process: impl Fn(&mut [u8], u32) + Send + Sync,
) {
    if bottom <= top {
        return;
    }
    let row_bytes = surface.width as usize * 4;
    let first_byte = top as usize * row_bytes;
    let end_byte = bottom as usize * row_bytes;
    let total_rows = (bottom - top) as usize;
    let workers = workers.clamp(1, total_rows);
    if workers == 1 {
        process(&mut surface.pixels[first_byte..end_byte], top);
        return;
    }
    let rows_per_band = total_rows.div_ceil(workers);
    let mut rest = &mut surface.pixels[first_byte..end_byte];
    let process = &process;
    std::thread::scope(|scope| {
        let mut band_top = top;
        while band_top < bottom {
            let band_rows = rows_per_band.min((bottom - band_top) as usize);
            let (band, tail) = rest.split_at_mut(band_rows * row_bytes);
            rest = tail;
            scope.spawn(move || process(band, band_top));
            band_top += band_rows as u32;
        }
    });
}

/// A projective resampling job over a destination rectangle: the authoritative
/// implementation behind both [`Surface::warp_blend`] and [`Surface::extract_quad`].
/// It preserves their historical per-pixel expression sequence exactly, including
/// operation order, so results do not depend on how rows are distributed.
struct ProjectiveResample<'a> {
    source: &'a Surface,
    matrix: [[f64; 3]; 3],
    transform_shift: f64,
    sample_shift: f64,
    left: i32,
    right: i32,
    mode: Resampling,
}

impl ProjectiveResample<'_> {
    fn run(&self, band: &mut [u8], band_top: u32, row_bytes: usize) {
        let m = &self.matrix;
        let first_column = self.left as usize * 4;
        let span_end = (self.right as usize + 1) * 4;
        for (row_offset, row) in band.chunks_mut(row_bytes).enumerate() {
            let y = band_top as f64 + row_offset as f64 + self.transform_shift;
            let mut x = self.left as f64 + self.transform_shift;
            for destination_pixel in row[first_column..span_end].chunks_mut(4) {
                let z = m[2][0] * x + m[2][1] * y + m[2][2];
                if z.abs() >= 1e-12 {
                    let sample_x = (m[0][0] * x + m[0][1] * y + m[0][2]) / z + self.sample_shift;
                    let sample_y = (m[1][0] * x + m[1][1] * y + m[1][2]) / z + self.sample_shift;
                    if let Some(sampled) = sample_bilinear_pixels(self.source, sample_x, sample_y) {
                        match self.mode {
                            Resampling::Overwrite => {
                                destination_pixel.copy_from_slice(&sampled);
                            }
                            Resampling::Blend(opacity) => blend_over(
                                destination_pixel,
                                Rgba::new(sampled[0], sampled[1], sampled[2], sampled[3]),
                                opacity,
                            ),
                        }
                    }
                }
                x += 1.0;
            }
        }
    }
}

#[inline]
fn pixel_at(surface: &Surface, x: u32, y: u32) -> [u8; 4] {
    let index = (y as usize * surface.width as usize + x as usize) * 4;
    // Bounds are validated by callers (`sample_bilinear_pixels` clamps to
    // `width-1`/`height-1` and `ProjectiveResample` limits `left`/`right`).
    // The `expect` documents that invariant; for library use we degrade to a
    // transparent-black fallback rather than aborting the caller if a future
    // caller violates it. The fallback is visually equivalent to an out-of-
    // bounds sample and never silently succeeds on a partial slice.
    surface
        .pixels
        .get(index..index + 4)
        .and_then(|slice| slice.try_into().ok())
        .unwrap_or([0, 0, 0, 0])
}

/// The authoritative bilinear sampler: linear-light, premultiplied-alpha
/// interpolation. `None` means the coordinate falls outside the half-pixel
/// boundary; a fully transparent mixture is reported as transparent black.
#[inline]
fn sample_bilinear_pixels(surface: &Surface, x: f64, y: f64) -> Option<[u8; 4]> {
    if x < -0.5 || y < -0.5 || x > surface.width as f64 - 0.5 || y > surface.height as f64 - 0.5 {
        return None;
    }
    let clamped_x = x.clamp(0.0, surface.width.saturating_sub(1) as f64);
    let clamped_y = y.clamp(0.0, surface.height.saturating_sub(1) as f64);
    let x0 = clamped_x.floor() as u32;
    let y0 = clamped_y.floor() as u32;
    let x1 = (x0 + 1).min(surface.width - 1);
    let y1 = (y0 + 1).min(surface.height - 1);
    let fx = (clamped_x - x0 as f64) as f32;
    let fy = (clamped_y - y0 as f64) as f32;
    let p00 = pixel_at(surface, x0, y0);
    let p10 = pixel_at(surface, x1, y0);
    let p01 = pixel_at(surface, x0, y1);
    let p11 = pixel_at(surface, x1, y1);
    // Every fully transparent neighborhood mixes to transparent black; skipping
    // the weight math yields exactly the same bytes as the general path below.
    if (p00[3] | p10[3] | p01[3] | p11[3]) == 0 {
        return Some([0, 0, 0, 0]);
    }
    let pixels = [p00, p10, p01, p11];
    let weights = [
        (1.0 - fx) * (1.0 - fy),
        fx * (1.0 - fy),
        (1.0 - fx) * fy,
        fx * fy,
    ];
    let alpha = pixels
        .iter()
        .zip(weights)
        .map(|(pixel, weight)| pixel[3] as f32 / 255.0 * weight)
        .sum::<f32>();
    let mut sampled = [0_u8; 4];
    if alpha > f32::EPSILON {
        for channel in 0..3 {
            let premultiplied = pixels
                .iter()
                .zip(weights)
                .map(|(pixel, weight)| {
                    srgb_to_linear(pixel[channel]) * (pixel[3] as f32 / 255.0) * weight
                })
                .sum::<f32>();
            sampled[channel] = linear_to_srgb(premultiplied / alpha);
        }
        sampled[3] = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    Some(sampled)
}

fn blur_rgba_horizontal(
    source: &[u8],
    destination: &mut [u8],
    width: usize,
    height: usize,
    radius: usize,
) {
    let workers = parallel_workers(width * height);
    if workers <= 1 {
        for y in 0..height {
            for channel in 0..4 {
                let mut sum = 0_u32;
                let mut count = 0_u32;
                for x in 0..=radius.min(width.saturating_sub(1)) {
                    sum += source[(y * width + x) * 4 + channel] as u32;
                    count += 1;
                }
                for x in 0..width {
                    destination[(y * width + x) * 4 + channel] = (sum / count) as u8;
                    let leaving = x.saturating_sub(radius);
                    let entering = x + radius + 1;
                    if x >= radius {
                        sum -= source[(y * width + leaving) * 4 + channel] as u32;
                        count -= 1;
                    }
                    if entering < width {
                        sum += source[(y * width + entering) * 4 + channel] as u32;
                        count += 1;
                    }
                }
            }
        }
        return;
    }
    // Row-sharded: each row is independent, preserves exact sums.
    let rows_per_band = height.div_ceil(workers);
    let band_bytes = rows_per_band * width * 4;
    std::thread::scope(|scope| {
        for (src_band, dst_band) in source
            .chunks(band_bytes)
            .zip(destination.chunks_mut(band_bytes))
        {
            scope.spawn(move || {
                let band_height = src_band.len() / (width * 4);
                for y in 0..band_height {
                    for channel in 0..4 {
                        let mut sum = 0_u32;
                        let mut count = 0_u32;
                        for x in 0..=radius.min(width.saturating_sub(1)) {
                            sum += src_band[(y * width + x) * 4 + channel] as u32;
                            count += 1;
                        }
                        for x in 0..width {
                            dst_band[(y * width + x) * 4 + channel] = (sum / count) as u8;
                            let leaving = x.saturating_sub(radius);
                            let entering = x + radius + 1;
                            if x >= radius {
                                sum -= src_band[(y * width + leaving) * 4 + channel] as u32;
                                count -= 1;
                            }
                            if entering < width {
                                sum += src_band[(y * width + entering) * 4 + channel] as u32;
                                count += 1;
                            }
                        }
                    }
                }
            });
        }
    });
}

fn blur_rgba_vertical(
    source: &[u8],
    destination: &mut [u8],
    width: usize,
    height: usize,
    radius: usize,
) {
    // Vertical blur remains sequential in this phase: each column's
    // `y` loop is independent but writes to strided `y*width+x` offsets
    // that are not contiguous per column. A fully parallel version needs
    // raw-pointer sharding or a transpose, which is deferred to keep this
    // phase bitwise-identical with minimal unsafe. Horizontal blur (row-
    // sharded) already covers the dominant cost for typical `radius<=16`.
    for x in 0..width {
        for channel in 0..4 {
            let mut sum = 0_u32;
            let mut count = 0_u32;
            for y in 0..=radius.min(height.saturating_sub(1)) {
                sum += source[(y * width + x) * 4 + channel] as u32;
                count += 1;
            }
            for y in 0..height {
                destination[(y * width + x) * 4 + channel] = (sum / count) as u8;
                let leaving = y.saturating_sub(radius);
                let entering = y + radius + 1;
                if y >= radius {
                    sum -= source[(leaving * width + x) * 4 + channel] as u32;
                    count -= 1;
                }
                if entering < height {
                    sum += source[(entering * width + x) * 4 + channel] as u32;
                    count += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Point;

    #[test]
    fn extracting_a_quad_preserves_sampled_alpha() {
        let mut source = Surface::new(3, 3);
        for y in 0..3 {
            for x in 0..3 {
                source.set_pixel(x, y, Rgba::new(255, 255, 255, (x + y * 3) as u8 * 20));
            }
        }

        let extracted =
            Surface::extract_quad(&source, Quad::from_rect(0.0, 0.0, 2.0, 2.0), 3, 3).unwrap();

        assert_eq!(extracted.alpha_mask(), source.alpha_mask());
    }

    #[test]
    fn projective_warp_preserves_an_opaque_solid_center() {
        let mut source = Surface::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                source.set_pixel(x, y, Rgba::new(20, 120, 220, 255));
            }
        }
        let mut destination = Surface::new(32, 32);
        destination
            .warp_blend(
                &source,
                Quad::new(
                    Point::new(4.0, 5.0),
                    Point::new(27.0, 3.0),
                    Point::new(25.0, 28.0),
                    Point::new(6.0, 26.0),
                ),
                1.0,
            )
            .unwrap();
        let center = destination.pixel(16, 16);
        assert_eq!(center, Rgba::new(20, 120, 220, 255));
    }

    #[test]
    fn source_over_compositing_uses_linear_light() {
        let mut destination = [0, 0, 0, 255];
        blend_over(&mut destination, Rgba::new(255, 255, 255, 128), 1.0);
        assert!((187..=189).contains(&destination[0]));
        assert_eq!(destination[0], destination[1]);
        assert_eq!(destination[1], destination[2]);
        assert_eq!(destination[3], 255);
    }

    #[test]
    fn bilinear_sampling_interpolates_premultiplied_color() {
        let mut source = Surface::new(2, 1);
        source.set_pixel(0, 0, Rgba::new(255, 0, 0, 255));
        source.set_pixel(1, 0, Rgba::new(0, 0, 255, 0));

        let middle = sample_bilinear_pixels(&source, 0.5, 0.0).unwrap();
        assert_eq!(middle[0], 255);
        assert_eq!(middle[1], 0);
        assert_eq!(middle[2], 0);
        assert!((127..=128).contains(&middle[3]));
    }

    #[test]
    fn mixture_bound_uses_linear_light_not_encoded_alpha_distance() {
        assert_eq!(constrained_linear_mixture_error(0, 188, 127), 0);
        assert_eq!(constrained_linear_mixture_error(0, 190, 127), 1);
        assert_eq!(constrained_linear_mixture_error(32, 33, 255), 0);
        assert_eq!(constrained_linear_mixture_error(32, 35, 255), 2);
        assert_eq!(constrained_linear_mixture_error(128, 0, 0), 0);
        assert_eq!(constrained_linear_mixture_error(128, 255, 0), 0);
    }

    /// Pseudo-random but fully deterministic RGBA content exercising many alpha,
    /// chroma, and gradient combinations in warp/compositing equivalence tests.
    fn patterned_surface(width: u32, height: u32, seed: u64) -> Surface {
        let mut state = seed | 1;
        let mut next = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as u8
        };
        let pixels = (0..width as usize * height as usize * 4)
            .map(|_| next())
            .collect();
        Surface::from_rgba(width, height, pixels).unwrap()
    }

    fn assert_warp_equivalent_across_worker_counts(
        source: &Surface,
        destination: Quad,
        opacity: f32,
        canvas: (u32, u32),
    ) {
        let mut serial = Surface::new(canvas.0, canvas.1);
        serial
            .warp_blend_with_workers(source, destination, opacity, 1)
            .expect("serial warp should succeed");
        let mut parallel = Surface::new(canvas.0, canvas.1);
        parallel
            .warp_blend_with_workers(source, destination, opacity, 8)
            .expect("parallel warp should succeed");
        assert_eq!(
            parallel.pixels(),
            serial.pixels(),
            "parallel warp diverged bitwise from serial warp"
        );
    }

    #[test]
    fn threaded_warp_is_bitwise_identical_to_serial_warp() {
        let source = patterned_surface(97, 61, 0xDEADBEEF);
        // Perspective-heavy quad crossing canvas edges exercises clipping,
        // boundary sampling, and the transparent-sampling skip path together.
        let quads = [
            Quad::new(
                Point::new(-30.0, 7.5),
                Point::new(180.0, -12.0),
                Point::new(150.0, 90.0),
                Point::new(-18.0, 70.0),
            ),
            Quad::new(
                Point::new(3.0, 2.0),
                Point::new(50.25, 4.75),
                Point::new(48.0, 44.5),
                Point::new(1.5, 39.0),
            ),
        ];
        for quad in quads {
            for &opacity in &[0.0_f32, 0.37, 1.0] {
                assert_warp_equivalent_across_worker_counts(&source, quad, opacity, (128, 80));
            }
        }
    }

    #[test]
    fn threaded_extract_is_bitwise_identical_to_serial_extract() {
        let frame = patterned_surface(211, 127, 0x5EED_1234);
        let quads = [
            Quad::new(
                Point::new(9.5, 4.25),
                Point::new(190.0, 1.0),
                Point::new(185.0, 120.0),
                Point::new(2.0, 118.0),
            ),
            Quad::from_rect(0.0, 0.0, 210.0, 126.0),
        ];
        for quad in quads {
            let mut serial = Surface::extract_quad_with_workers(&frame, quad, 160, 100, 1).unwrap();
            let parallel = Surface::extract_quad_with_workers(&frame, quad, 160, 100, 8).unwrap();
            assert_eq!(
                parallel.pixels(),
                serial.pixels(),
                "parallel extraction diverged bitwise from serial extraction"
            );
            serial.apply_alpha_mask(&vec![255_u8; 160 * 100]).unwrap();
        }
    }

    #[test]
    fn restore_fast_path_matches_general_source_over_blending_exactly() {
        let mut original = Surface::new(256, 4);
        let mut restored = Surface::new(256, 4);
        let mut mask = Vec::with_capacity(256 * 4);
        for alpha in 0..=255_u8 {
            for row in 0..4_u32 {
                let index = (row * 256 + alpha as u32) as usize;
                original.set_pixel(
                    alpha as u32,
                    row,
                    Rgba::new(alpha, 255 - alpha, alpha / 2, 255),
                );
                restored.set_pixel(
                    alpha as u32,
                    row,
                    Rgba::new(
                        255 - alpha,
                        alpha,
                        200,
                        if alpha % 3 == 0 { 0 } else { 255 },
                    ),
                );
                mask.push(if row == 3 {
                    alpha.wrapping_add(17)
                } else {
                    alpha
                });
                let _ = index;
            }
        }
        let reference = restored.clone();
        let mut fast = restored.clone();
        fast.restore_from_mask(&original, &mask).unwrap();

        let mut expected = reference;
        for ((dst, src), &alpha) in expected
            .pixels
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(original.pixels.as_chunks::<4>().0.iter())
            .zip(&mask)
        {
            blend_over(
                dst,
                Rgba::new(src[0], src[1], src[2], src[3]),
                alpha as f32 / 255.0,
            );
        }
        assert_eq!(
            fast.pixels(),
            expected.pixels(),
            "restore_from_mask diverged from direct source-over blending"
        );

        // The opaque fast path is exact only because every encoded level survives
        // the linear-light round trip; pin that property exhaustively.
        for level in 0..=255_u8 {
            assert_eq!(
                linear_to_srgb(srgb_to_linear(level)),
                level,
                "linear round trip is not identity for level {level}"
            );
        }
    }
}
