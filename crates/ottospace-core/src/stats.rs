//! Numbers over a run of samples.

/// The nearest-rank percentile `p` (0 to 100) of `sorted`; none: 0.
pub fn rank(sorted: &[f32], p: usize) -> f32 {
    let n = sorted.len();
    if n == 0 { 0. } else { sorted[(n * p).div_ceil(100).clamp(1, n) - 1] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_are_nearest_rank() {
        let v: Vec<f32> = (1..=100).map(|v| v as f32).collect();
        assert_eq!([0, 50, 95, 99, 100].map(|p| rank(&v, p)), [1., 50., 95., 99., 100.]);
        assert_eq!((rank(&[], 50), rank(&[7.], 99)), (0., 7.));
    }
}
