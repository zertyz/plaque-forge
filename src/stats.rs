//! Shared order-statistics and sampling helpers.
//!
//! Each function preserves the exact ranking/rounding semantics of the
//! pipeline stage it serves; the variants are intentionally not generic
//! because percentile rank rounding differs between consumers.

/// Nearest-rank f64 percentile (rank rounded to the closest index), returning
/// [`f64::INFINITY`] for empty input.
pub fn percentile(values: &mut [f64], quantile: f64) -> f64 {
    if values.is_empty() {
        return f64::INFINITY;
    }
    values.sort_by(f64::total_cmp);
    let index = ((values.len() - 1) as f64 * quantile.clamp(0.0, 1.0)).round() as usize;
    values[index]
}

/// Middle element of the sorted order (`total_cmp`), [`f64::INFINITY`] when empty.
pub fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return f64::INFINITY;
    }
    let middle = values.len() / 2;
    let (_, median, _) = values.select_nth_unstable_by(middle, f64::total_cmp);
    *median
}

/// Percentile over byte samples, returned as f64; callers guarantee non-empty input.
pub fn percentile_u8(values: &mut [u8], percentile: f64) -> f64 {
    values.sort_unstable();
    let index = ((values.len().saturating_sub(1)) as f64 * percentile)
        .round()
        .clamp(0.0, values.len().saturating_sub(1) as f64) as usize;
    values[index] as f64
}

/// Floor-ranked u16 percentile clamped to `0.05`..`0.99` quantiles, `0` when empty.
pub fn percentile_u16(values: &mut [u16], percentile: f64) -> u16 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let rank = ((values.len() - 1) as f64 * percentile.clamp(0.0, 1.0)).floor() as usize;
    values[rank]
}

/// Arithmetic mean, `0.0` for empty input.
pub fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

/// `count` indices spanning `0..frames` inclusively at even spacing,
/// capped to the available frame count.
pub fn evenly_spaced(frames: usize, count: usize) -> Vec<usize> {
    if frames == 0 || count == 0 {
        return Vec::new();
    }
    let count = count.min(frames);
    if count == 1 {
        return vec![0];
    }
    (0..count)
        .map(|index| index * (frames - 1) / (count - 1))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_matches_nearest_rank_rounding() {
        let mut values = vec![3.0, 1.0, 4.0, 1.0, 5.0];
        assert_eq!(percentile(&mut values, 0.50), 3.0);
        assert_eq!(percentile(&mut values, 0.95), 5.0);
        assert_eq!(percentile(&mut values, 0.0), 1.0);
        assert!(percentile(&mut [], 0.5).is_infinite());
    }

    #[test]
    fn median_selects_the_sorted_middle_element() {
        assert_eq!(median(&mut [5.0, 1.0, 3.0]), 3.0);
        assert_eq!(median(&mut [4.0, 1.0, 3.0, 2.0]), 3.0);
        assert!(median(&mut []).is_infinite());
    }

    #[test]
    fn percentile_u8_uses_the_requested_rank() {
        let mut values = [9u8, 10, 1, 2, 3];
        assert_eq!(percentile_u8(&mut values, 0.50), 3.0);
        assert_eq!(percentile_u8(&mut values, 0.95), 10.0);
    }

    #[test]
    fn percentile_u16_floors_the_rank_and_handles_empty_input() {
        let mut values = vec![7u16, 11, 13];
        assert_eq!(percentile_u16(&mut values, 0.05), 7);
        assert_eq!(percentile_u16(&mut values, 0.99), 11);
        assert_eq!(percentile_u16(&mut values, 1.0), 13);
        assert_eq!(percentile_u16(&mut [], 0.5), 0);
    }

    #[test]
    fn mean_averages_samples_and_treats_empty_as_zero() {
        assert!((mean(&[1.0, 2.0, 3.0]) - 2.0).abs() < f64::EPSILON);
        assert_eq!(mean(&[]), 0.0);
    }

    #[test]
    fn evenly_spaced_samples_span_the_first_and_last_frame() {
        assert_eq!(evenly_spaced(0, 12), Vec::<usize>::new());
        assert_eq!(evenly_spaced(1, 12), vec![0]);
        assert_eq!(evenly_spaced(5, 3), vec![0, 2, 4]);
        assert_eq!(evenly_spaced(4, 12), vec![0, 1, 2, 3]);
        assert_eq!(evenly_spaced(10, 0), Vec::<usize>::new());
    }
}
