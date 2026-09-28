//! Small numeric helpers for metrics and reports.

/// `total / count` as a float, or zero for no samples.
#[must_use]
pub fn mean(total: u64, count: u64) -> f64 {
    if count == 0 {
        return 0.0;
    }

    to_f64(total) / to_f64(count)
}

/// Lossy widening for statistics.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    reason = "benchmark counts stay far below 2^53"
)]
pub const fn to_f64(value: u64) -> f64 {
    value as f64
}

/// Mean of a sample, or zero when empty.
#[must_use]
pub fn mean_f64(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    values.iter().sum::<f64>() / to_f64(u64::try_from(values.len()).unwrap_or(u64::MAX))
}

/// Smallest and largest value of a sample.
#[must_use]
pub fn min_max(values: &[f64]) -> Option<(f64, f64)> {
    let first = *values.first()?;

    Some(
        values
            .iter()
            .fold((first, first), |(low, high), &v| (low.min(v), high.max(v))),
    )
}
