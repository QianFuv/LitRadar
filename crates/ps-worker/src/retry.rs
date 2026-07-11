//! Shared retry-count and backoff safety helpers.

use std::time::Duration;

use ps_domain::DELIVERY_RETRY_ATTEMPTS_MAX;

const MAX_RETRY_BACKOFF_SECONDS: u64 = 8;

/// Cap an unsigned retry count at the shared delivery maximum.
pub(crate) fn bounded_retry_attempts(retry_attempts: usize) -> usize {
    retry_attempts.min(DELIVERY_RETRY_ATTEMPTS_MAX)
}

/// Convert a signed retry count into the shared nonnegative delivery range.
pub(crate) fn bounded_retry_attempts_from_i64(retry_attempts: i64) -> usize {
    let maximum =
        i64::try_from(DELIVERY_RETRY_ATTEMPTS_MAX).expect("delivery retry maximum should fit i64");
    usize::try_from(retry_attempts.clamp(0, maximum))
        .expect("bounded retry attempts should fit usize")
}

/// Return an overflow-safe exponential retry delay capped at eight seconds.
pub(crate) fn retry_backoff_delay(attempt: usize) -> Duration {
    let exponent = u32::try_from(attempt).unwrap_or(u32::MAX);
    let seconds = 1_u64
        .checked_shl(exponent)
        .unwrap_or(MAX_RETRY_BACKOFF_SECONDS)
        .min(MAX_RETRY_BACKOFF_SECONDS);
    Duration::from_secs(seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_backoff_handles_normal_and_extreme_attempts() {
        for (attempt, expected_seconds) in [
            (0_usize, 1_u64),
            (1, 2),
            (2, 4),
            (3, 8),
            (10, 8),
            (usize::MAX, 8),
        ] {
            assert_eq!(
                retry_backoff_delay(attempt),
                Duration::from_secs(expected_seconds)
            );
        }
    }

    #[test]
    fn retry_count_helpers_bound_unsigned_and_signed_inputs() {
        assert_eq!(bounded_retry_attempts(3), 3);
        assert_eq!(bounded_retry_attempts(usize::MAX), 10);
        assert_eq!(bounded_retry_attempts_from_i64(-1), 0);
        assert_eq!(bounded_retry_attempts_from_i64(3), 3);
        assert_eq!(bounded_retry_attempts_from_i64(i64::MAX), 10);
    }
}
