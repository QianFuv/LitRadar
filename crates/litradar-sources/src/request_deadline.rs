//! Cooperative monotonic deadlines for blocking request-time source work.

use std::error::Error;
use std::fmt;
use std::thread;
use std::time::{Duration, Instant};

/// Stable failure returned when an article-access deadline has expired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RequestDeadlineError;

impl fmt::Display for RequestDeadlineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("article access deadline expired")
    }
}

impl Error for RequestDeadlineError {}

/// Reject work that would start after an optional monotonic deadline.
///
/// # Arguments
///
/// * `deadline` - Optional shared request deadline.
///
/// # Returns
///
/// Empty result while work may still start, or a stable expiration error.
pub(crate) fn ensure_deadline(deadline: Option<Instant>) -> Result<(), RequestDeadlineError> {
    remaining_duration(deadline).map(|_| ())
}

/// Clamp one configured HTTP timeout to the remaining request budget.
///
/// # Arguments
///
/// * `configured_timeout` - Provider-specific request timeout.
/// * `deadline` - Optional shared request deadline.
///
/// # Returns
///
/// Configured timeout when unbounded, otherwise the smaller positive remaining duration.
pub(crate) fn request_timeout(
    configured_timeout: Duration,
    deadline: Option<Instant>,
) -> Result<Duration, RequestDeadlineError> {
    remaining_duration(deadline).map(|remaining| {
        remaining.map_or(configured_timeout, |value| configured_timeout.min(value))
    })
}

/// Sleep for a retry delay without crossing an optional request deadline.
///
/// # Arguments
///
/// * `delay` - Requested retry or polling delay.
/// * `deadline` - Optional shared request deadline.
///
/// # Returns
///
/// Empty result when the full delay completed inside the budget, otherwise expiration.
pub(crate) fn sleep(
    delay: Duration,
    deadline: Option<Instant>,
) -> Result<(), RequestDeadlineError> {
    let Some(remaining) = remaining_duration(deadline)? else {
        thread::sleep(delay);
        return Ok(());
    };
    if delay >= remaining {
        thread::sleep(remaining);
        return Err(RequestDeadlineError);
    }
    thread::sleep(delay);
    ensure_deadline(deadline)
}

fn remaining_duration(deadline: Option<Instant>) -> Result<Option<Duration>, RequestDeadlineError> {
    let Some(deadline) = deadline else {
        return Ok(None);
    };
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .map(Some)
        .ok_or(RequestDeadlineError)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{ensure_deadline, request_timeout, sleep};

    #[test]
    fn request_timeout_clamps_to_the_remaining_budget() {
        let deadline = Instant::now() + Duration::from_millis(100);
        let timeout = request_timeout(Duration::from_secs(30), Some(deadline))
            .expect("future deadline should provide a timeout");

        assert!(timeout > Duration::ZERO);
        assert!(timeout <= Duration::from_millis(100));
        assert_eq!(
            request_timeout(Duration::from_secs(30), None)
                .expect("missing deadline should retain configured timeout"),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn expired_deadline_rejects_new_work() {
        let deadline = Instant::now()
            .checked_sub(Duration::from_millis(1))
            .expect("test deadline should subtract");

        assert!(ensure_deadline(Some(deadline)).is_err());
        assert!(request_timeout(Duration::from_secs(30), Some(deadline)).is_err());
    }

    #[test]
    fn retry_sleep_stops_at_the_deadline() {
        let started_at = Instant::now();
        let deadline = started_at + Duration::from_millis(25);
        let result = sleep(Duration::from_secs(1), Some(deadline));
        let elapsed = started_at.elapsed();

        assert!(result.is_err());
        assert!(elapsed >= Duration::from_millis(15));
        assert!(elapsed < Duration::from_millis(500));
    }
}
