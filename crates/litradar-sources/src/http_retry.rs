//! Shared server-delay parsing and bounded waits for live source requests.

use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Datelike, NaiveDateTime, Utc};
use reqwest::header::HeaderMap;

const LOGICAL_REQUEST_BUDGET: Duration = Duration::from_secs(180);

/// Bound one logical request while retaining an earlier caller deadline.
pub(crate) fn deadline(caller_deadline: Option<Instant>) -> Instant {
    let limit = Instant::now() + LOGICAL_REQUEST_BUDGET;
    caller_deadline.map_or(limit, |caller| caller.min(limit))
}

/// Return the positive time left before a logical request expires.
pub(crate) fn remaining(deadline: Instant) -> Option<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
}

/// Check that a complete server delay fits without retrying early.
pub(crate) fn can_wait(delay: Duration, deadline: Instant) -> bool {
    remaining(deadline).is_some_and(|remaining| delay < remaining)
}

/// Wait the full delay, or reject it immediately when the deadline cannot fit it.
pub(crate) fn sleep(delay: Duration, deadline: Instant) -> bool {
    if !can_wait(delay, deadline) {
        return false;
    }
    thread::sleep(delay);
    remaining(deadline).is_some()
}

/// Read a Retry-After delay without trusting malformed header values.
pub(crate) fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    parse_retry_after(
        headers.get("retry-after")?.to_str().ok()?,
        SystemTime::now(),
    )
}

fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let value = value.trim();
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        return value.parse::<u64>().ok().map(Duration::from_secs);
    }
    let now = DateTime::<Utc>::from(now);
    let date = DateTime::parse_from_rfc2822(value)
        .ok()
        .map(|date| date.with_timezone(&Utc))
        .or_else(|| parse_rfc850_date(value, now))
        .or_else(|| {
            NaiveDateTime::parse_from_str(value, "%a %b %e %H:%M:%S %Y")
                .ok()
                .map(|date| date.and_utc())
        })?;
    Some(date.signed_duration_since(now).to_std().unwrap_or_default())
}

fn parse_rfc850_date(value: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let (weekday, remainder) = value.split_once(", ")?;
    let (date, time) = remainder.split_once(' ')?;
    let (day_month, short_year) = date.rsplit_once('-')?;
    if short_year.len() != 2 || !short_year.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let year = now.year().div_euclid(100) * 100 + short_year.parse::<i32>().ok()?;
    let expanded = format!("{day_month}-{year:04} {time}");
    let mut date = NaiveDateTime::parse_from_str(&expanded, "%d-%b-%Y %H:%M:%S GMT").ok()?;
    if (date.year(), date.month(), date.day(), date.time())
        > (now.year() + 50, now.month(), now.day(), now.time())
    {
        date = date.with_year(year - 100)?;
    }
    (date.format("%A").to_string() == weekday).then(|| date.and_utc())
}

/// Advance an initial UTC epoch using elapsed monotonic time for cohort phases and cooldowns.
pub(crate) fn monotonic_time() -> Duration {
    static CLOCK: OnceLock<(Instant, Duration)> = OnceLock::new();
    let (started_at, epoch) = CLOCK.get_or_init(|| {
        (
            Instant::now(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default(),
        )
    });
    epoch.saturating_add(started_at.elapsed())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_accepts_seconds_and_http_dates() {
        let now = UNIX_EPOCH + Duration::from_secs(1_445_412_000);
        assert_eq!(
            parse_retry_after(" 12 ", now),
            Some(Duration::from_secs(12))
        );
        assert_eq!(parse_retry_after("0", now), Some(Duration::ZERO));
        assert_eq!(
            parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT", now),
            Some(Duration::from_secs(480)),
        );
        assert_eq!(
            parse_retry_after("Wed, 21 Oct 2015 07:00:00 GMT", now),
            Some(Duration::ZERO),
        );
        for value in ["", "-1", "+1", "0.5", "later", "18446744073709551616"] {
            assert_eq!(parse_retry_after(value, now), None, "{value}");
        }
    }

    #[test]
    fn logical_deadline_keeps_earlier_callers_and_caps_later_ones() {
        let early = Instant::now() + Duration::from_secs(1);
        assert_eq!(deadline(Some(early)), early);
        let started_at = Instant::now();
        let capped = deadline(Some(started_at + Duration::from_secs(600)));
        assert!(capped >= started_at + Duration::from_secs(180));
        assert!(capped < started_at + Duration::from_secs(181));
    }

    #[test]
    fn retry_after_accepts_obsolete_http_dates_and_the_fifty_year_rule() {
        let now = DateTime::parse_from_rfc3339("1994-11-06T08:00:00Z").unwrap();
        for value in ["Sunday, 06-Nov-94 08:49:37 GMT", "Sun Nov  6 08:49:37 1994"] {
            assert_eq!(
                parse_retry_after(value, SystemTime::from(now)),
                Some(Duration::from_secs(2_977))
            );
        }
        let now = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap();
        let future = DateTime::parse_from_rfc3339("2075-11-06T08:49:37Z").unwrap();
        let future_text = future.format("%A, %d-%b-%y %H:%M:%S GMT").to_string();
        assert_eq!(
            parse_retry_after(&future_text, SystemTime::from(now)),
            Some(future.signed_duration_since(now).to_std().unwrap())
        );
        let past = DateTime::parse_from_rfc3339("1977-11-06T08:49:37Z").unwrap();
        let past_text = past.format("%A, %d-%b-%y %H:%M:%S GMT").to_string();
        assert_eq!(
            parse_retry_after(&past_text, SystemTime::from(now)),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn oversized_wait_returns_without_sleeping_to_the_deadline() {
        let started_at = Instant::now();
        assert!(!sleep(
            Duration::from_secs(300),
            started_at + Duration::from_secs(30)
        ));
        assert!(started_at.elapsed() < Duration::from_secs(1));
        assert!(!can_wait(
            Duration::MAX,
            started_at + Duration::from_secs(30)
        ));
    }
}
