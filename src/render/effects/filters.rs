//! Image filters, alpha mask dilation, morphology, and geometric surface distortion.

use crate::surface::Surface;

pub fn directional_bevel_masks(
    source: &[u8],
    width: usize,
    height: usize,
    radius: i32,
) -> (Vec<u8>, Vec<u8>) {
    let mut highlight = vec![0; source.len()];
    let mut shadow = vec![0; source.len()];
    for y in 0..height {
        for x in 0..width {
            let current = source[y * width + x];
            if current == 0 {
                continue;
            }
            let top_left =
                sample_alpha(source, width, height, x as i32 - radius, y as i32 - radius);
            let bottom_right =
                sample_alpha(source, width, height, x as i32 + radius, y as i32 + radius);
            highlight[y * width + x] = current.saturating_sub(top_left);
            shadow[y * width + x] = current.saturating_sub(bottom_right);
        }
    }
    (highlight, shadow)
}

pub fn sample_alpha(source: &[u8], width: usize, height: usize, x: i32, y: i32) -> u8 {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        0
    } else {
        source[y as usize * width + x as usize]
    }
}

pub fn alpha_bounds(source: &[u8], width: usize) -> Option<(u32, u32, u32, u32)> {
    if width == 0 || source.is_empty() || !source.len().is_multiple_of(width) {
        return None;
    }
    let height = source.len() / width;
    let (mut x0, mut y0, mut x1, mut y1) = (width, height, 0_usize, 0_usize);
    let mut any = false;
    for (index, &value) in source.iter().enumerate() {
        if value == 0 {
            continue;
        }
        let x = index % width;
        let y = index / width;
        any = true;
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    any.then_some((x0 as u32, y0 as u32, x1 as u32, y1 as u32))
}

pub fn alpha_over_shifted(
    output: &mut [u8],
    input: &[u8],
    width: usize,
    height: usize,
    dx: i32,
    dy: i32,
) {
    debug_assert_eq!(output.len(), width * height);
    debug_assert_eq!(input.len(), width * height);
    let left = dx.max(0) as usize;
    let top = dy.max(0) as usize;
    let right = (width as i32 + dx).min(width as i32).max(0) as usize;
    let bottom = (height as i32 + dy).min(height as i32).max(0) as usize;
    for y in top..bottom {
        let source_y = (y as i32 - dy) as usize;
        for x in left..right {
            let source_x = (x as i32 - dx) as usize;
            let source_alpha = input[source_y * width + source_x] as u16;
            let destination = &mut output[y * width + x];
            let remaining = (255 - *destination as u16) * (255 - source_alpha);
            *destination = (255 - (remaining + 127) / 255) as u8;
        }
    }
}

pub fn dilate_alpha_circular(source: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    if radius == 0 {
        return source.to_vec();
    }
    let Some((x0, y0, x1, y1)) = alpha_bounds(source, width) else {
        return vec![0; source.len()];
    };
    let (x0, y0, x1, y1) = (x0 as usize, y0 as usize, x1 as usize, y1 as usize);

    // Exact grayscale disk dilation. For each vertical disk offset, compute the
    // corresponding horizontal max-filter with a monotonic deque. This preserves
    // every output byte while reducing O(width*height*radius²) to
    // O(ink-bounds*radius).
    let radius_squared = radius * radius;
    let row_spans = (0..=radius)
        .map(|dy| {
            let dx = ((radius_squared - dy * dy) as f64).sqrt().floor() as usize;
            (dy, dx)
        })
        .collect::<Vec<_>>();
    let mut output = vec![0u8; source.len()];
    for (dy, dx) in row_spans {
        let left = x0.saturating_sub(dx);
        let right = x1.saturating_add(dx).min(width - 1);
        let mut filtered = vec![0_u8; right - left + 1];
        for source_y in y0..=y1 {
            horizontal_max_filter(
                &source[source_y * width..(source_y + 1) * width],
                x0,
                x1,
                dx,
                left,
                &mut filtered,
            );
            if let Some(target_y) = source_y.checked_sub(dy) {
                merge_max(
                    &mut output[target_y * width + left..=target_y * width + right],
                    &filtered,
                );
            }
            if dy > 0 && source_y + dy < height {
                let target_y = source_y + dy;
                merge_max(
                    &mut output[target_y * width + left..=target_y * width + right],
                    &filtered,
                );
            }
        }
    }
    output
}

pub fn horizontal_max_filter(
    source: &[u8],
    source_left: usize,
    source_right: usize,
    radius: usize,
    output_left: usize,
    output: &mut [u8],
) {
    let mut deque = std::collections::VecDeque::<usize>::new();
    let mut next = source_left;
    for (offset, destination) in output.iter_mut().enumerate() {
        let x = output_left + offset;
        let window_right = x.saturating_add(radius).min(source_right);
        while next <= window_right {
            while deque
                .back()
                .is_some_and(|&index| source[index] <= source[next])
            {
                deque.pop_back();
            }
            deque.push_back(next);
            next += 1;
        }
        let window_left = x.saturating_sub(radius).max(source_left);
        while deque.front().is_some_and(|&index| index < window_left) {
            deque.pop_front();
        }
        *destination = deque.front().map_or(0, |&index| source[index]);
    }
}

pub fn merge_max(output: &mut [u8], input: &[u8]) {
    for (output, &input) in output.iter_mut().zip(input) {
        *output = (*output).max(input);
    }
}

pub fn reveal_surface(source: &Surface, progress: f32) -> Surface {
    let Some(bounds) = source.alpha_bounds() else {
        return source.clone();
    };
    let right = bounds.0 as f32 + (bounds.2 - bounds.0 + 1) as f32 * progress.clamp(0.0, 1.0);
    let mut output = Surface::new(source.width(), source.height());
    for y in 0..source.height() {
        for x in 0..source.width() {
            if x as f32 <= right {
                output.set_pixel(x, y, source.pixel(x, y));
            }
        }
    }
    output
}

pub fn dissolve_surface(source: &Surface, progress: f32, seed: u32) -> Surface {
    let progress = progress.clamp(0.0, 1.0);
    let mut output = Surface::new(source.width(), source.height());
    for y in 0..source.height() {
        for x in 0..source.width() {
            let pixel = source.pixel(x, y);
            if pixel.a == 0 {
                continue;
            }
            let mut hash = seed ^ x.wrapping_mul(0x9E37_79B9) ^ y.wrapping_mul(0x85EB_CA6B);
            hash ^= hash >> 16;
            hash = hash.wrapping_mul(0x7FEB_352D);
            hash ^= hash >> 15;
            let threshold = (hash & 0xFFFF) as f32 / 65535.0;
            if threshold <= progress {
                output.set_pixel(x, y, pixel);
            }
        }
    }
    output
}

pub fn wave_surface(
    source: &Surface,
    time_seconds: f64,
    period_seconds: f32,
    amplitude_ratio: f32,
    wavelength_ratio: f32,
    phase: f32,
) -> Surface {
    let Some(bounds) = source.alpha_bounds() else {
        return source.clone();
    };
    let glyph_height = (bounds.3 - bounds.1 + 1).max(1) as f32;
    let glyph_width = (bounds.2 - bounds.0 + 1).max(1) as f32;
    let amplitude = glyph_height * amplitude_ratio;
    let wavelength = (glyph_width * wavelength_ratio).max(1.0);
    let temporal = std::f32::consts::TAU * (time_seconds as f32 / period_seconds + phase);
    let mut output = Surface::new(source.width(), source.height());
    for y in 0..source.height() {
        for x in 0..source.width() {
            let shift = amplitude
                * (std::f32::consts::TAU * (x as f32 - bounds.0 as f32) / wavelength + temporal)
                    .sin();
            let sy = (y as f32 - shift).round() as i32;
            if sy >= 0 && sy < source.height() as i32 {
                output.set_pixel(x, y, source.pixel(x, sy as u32));
            }
        }
    }
    output
}
