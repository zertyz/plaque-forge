use crate::{
    color::Rgba,
    geometry::{Point, Quad, homography},
};
use anyhow::{Result, bail};
use std::sync::OnceLock;

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
        for ((dst, src), &alpha) in self
            .pixels
            .chunks_exact_mut(4)
            .zip(original.pixels.chunks_exact(4))
            .zip(mask)
        {
            blend_over(
                dst,
                Rgba::new(src[0], src[1], src[2], src[3]),
                alpha as f32 / 255.0,
            );
        }
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
        for (pixel, &mask_alpha) in self.pixels.chunks_exact_mut(4).zip(mask) {
            pixel[3] = ((pixel[3] as u16 * mask_alpha as u16 + 127) / 255) as u8;
        }
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

        for y in top..bottom {
            for x in left..right {
                let sx = (x - dx) as u32;
                let sy = (y - dy) as u32;
                self.blend_pixel(x, y, source.pixel(sx, sy), opacity);
            }
        }
    }

    /// Warps the whole source canvas to the destination quadrilateral and alpha-composites it.
    pub fn warp_blend(&mut self, source: &Surface, destination: Quad, opacity: f32) -> Result<()> {
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

        for y in top..=bottom {
            for x in left..=right {
                let Some(mapped) = inverse.transform(Point::new(x as f64 + 0.5, y as f64 + 0.5))
                else {
                    continue;
                };
                if let Some(pixel) = source.sample_bilinear(mapped.x - 0.5, mapped.y - 0.5) {
                    self.blend_pixel(x, y, pixel, opacity);
                }
            }
        }
        Ok(())
    }

    /// Pulls a planar region from a video frame into a canonical rectangular canvas.
    pub fn extract_quad(
        frame: &Surface,
        source_quad: Quad,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let canonical = Quad::from_rect(
            0.0,
            0.0,
            (width.saturating_sub(1)) as f64,
            (height.saturating_sub(1)) as f64,
        );
        let to_frame = homography(canonical, source_quad)?;
        let mut result = Self::new(width, height);

        for y in 0..height {
            for x in 0..width {
                let Some(mapped) = to_frame.transform(Point::new(x as f64, y as f64)) else {
                    continue;
                };
                if let Some(pixel) = frame.sample_bilinear(mapped.x, mapped.y) {
                    result.set_pixel(x, y, pixel);
                }
            }
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
        self.pixels.chunks_exact(4).map(|pixel| pixel[3]).collect()
    }

    pub fn recolor(&mut self, color: Rgba) {
        for pixel in self.pixels.chunks_exact_mut(4) {
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
        for (pixel, &alpha) in surface.pixels.chunks_exact_mut(4).zip(mask) {
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

    fn sample_bilinear(&self, x: f64, y: f64) -> Option<Rgba> {
        if x < -0.5 || y < -0.5 || x > self.width as f64 - 0.5 || y > self.height as f64 - 0.5 {
            return None;
        }
        let x = x.clamp(0.0, self.width.saturating_sub(1) as f64);
        let y = y.clamp(0.0, self.height.saturating_sub(1) as f64);
        let x0 = x.floor() as u32;
        let y0 = y.floor() as u32;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let fx = (x - x0 as f64) as f32;
        let fy = (y - y0 as f64) as f32;

        let p00 = self.pixel(x0, y0);
        let p10 = self.pixel(x1, y0);
        let p01 = self.pixel(x0, y1);
        let p11 = self.pixel(x1, y1);
        let weights = [
            (1.0 - fx) * (1.0 - fy),
            fx * (1.0 - fy),
            (1.0 - fx) * fy,
            fx * fy,
        ];
        let pixels = [p00, p10, p01, p11];
        let alpha = pixels
            .iter()
            .zip(weights)
            .map(|(pixel, weight)| pixel.a as f32 / 255.0 * weight)
            .sum::<f32>();
        if alpha <= f32::EPSILON {
            return Some(Rgba::new(0, 0, 0, 0));
        }
        let channel = |select: fn(&Rgba) -> u8| -> u8 {
            let premultiplied = pixels
                .iter()
                .zip(weights)
                .map(|(pixel, weight)| {
                    srgb_to_linear(select(pixel)) * (pixel.a as f32 / 255.0) * weight
                })
                .sum::<f32>();
            linear_to_srgb(premultiplied / alpha)
        };
        Some(Rgba::new(
            channel(|pixel| pixel.r),
            channel(|pixel| pixel.g),
            channel(|pixel| pixel.b),
            (alpha * 255.0).round().clamp(0.0, 255.0) as u8,
        ))
    }

    fn index(&self, x: u32, y: u32) -> usize {
        (y as usize * self.width as usize + x as usize) * 4
    }
}

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

fn srgb_to_linear(value: u8) -> f32 {
    static TABLE: OnceLock<[f32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|index| {
            let encoded = index as f32 / 255.0;
            if encoded <= 0.04045 {
                encoded / 12.92
            } else {
                ((encoded + 0.055) / 1.055).powf(2.4)
            }
        })
    })[value as usize]
}

fn linear_to_srgb(value: f32) -> u8 {
    static TABLE: OnceLock<Box<[u8; 65_536]>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
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
    let index = (value.clamp(0.0, 1.0) * 65_535.0).round() as usize;
    table[index]
}

/// Distance outside all possible linear-light mixtures containing `known` at
/// exactly `known_weight`. A one-level encoded allowance covers LUT/FFmpeg rounding.
pub(crate) fn constrained_linear_mixture_error(known: u8, observed: u8, known_weight: u8) -> u64 {
    static BOUNDS: OnceLock<Box<[(u8, u8)]>> = OnceLock::new();
    let bounds = BOUNDS.get_or_init(|| {
        (0..=u16::MAX)
            .map(|index| {
                let known = (index >> 8) as u8;
                let known_weight = (index & 0xff) as f32 / 255.0;
                let known_contribution = srgb_to_linear(known) * known_weight;
                (
                    linear_to_srgb(known_contribution).saturating_sub(1),
                    linear_to_srgb(known_contribution + 1.0 - known_weight).saturating_add(1),
                )
            })
            .collect()
    });
    let (minimum, maximum) = bounds[usize::from(known) * 256 + usize::from(known_weight)];
    if observed < minimum {
        u64::from(minimum - observed)
    } else if observed > maximum {
        u64::from(observed - maximum)
    } else {
        0
    }
}

fn blur_rgba_horizontal(
    source: &[u8],
    destination: &mut [u8],
    width: usize,
    height: usize,
    radius: usize,
) {
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
}

fn blur_rgba_vertical(
    source: &[u8],
    destination: &mut [u8],
    width: usize,
    height: usize,
    radius: usize,
) {
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

        let middle = source.sample_bilinear(0.5, 0.0).unwrap();
        assert_eq!(middle.r, 255);
        assert_eq!(middle.g, 0);
        assert_eq!(middle.b, 0);
        assert!((127..=128).contains(&middle.a));
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
}
