//! Index run statistics and secret-safe API attempt aggregation.

use std::collections::BTreeMap;

use ps_sources::SourceAttempt;
use serde::Serialize;

/// API statistics aggregation key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct ApiStatsKey {
    /// Index source identifier.
    pub source: String,
    /// Upstream service identifier.
    pub service: String,
    /// Logical endpoint identifier.
    pub endpoint: String,
    /// HTTP method.
    pub method: String,
    /// Query-free URL path.
    pub url_path: String,
    /// Current journal id.
    pub journal_id: Option<i64>,
    /// Current journal title.
    pub journal_title: String,
}

/// Aggregated source API attempt statistics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApiCallStats {
    /// Aggregation key.
    pub key: ApiStatsKey,
    /// Logical API calls.
    pub logical_calls: i64,
    /// HTTP attempts.
    pub attempts: i64,
    /// Successful attempts.
    pub successes: i64,
    /// Failed attempts.
    pub failures: i64,
    /// Retry attempt count.
    pub retry_count: i64,
    /// Attempt counts by status code.
    pub status_codes: BTreeMap<u16, i64>,
    /// Transport error count.
    pub transport_errors: i64,
    /// Rate-limit failure count.
    pub rate_limit_failures: i64,
    /// Total latency in milliseconds.
    pub total_latency_ms: i64,
    /// Secret-free error samples.
    pub error_samples: Vec<String>,
}

impl ApiCallStats {
    /// Build an empty API bucket.
    ///
    /// # Arguments
    ///
    /// * `key` - Aggregation key.
    ///
    /// # Returns
    ///
    /// API statistics bucket.
    pub fn new(key: ApiStatsKey) -> Self {
        Self {
            key,
            logical_calls: 0,
            attempts: 0,
            successes: 0,
            failures: 0,
            retry_count: 0,
            status_codes: BTreeMap::new(),
            transport_errors: 0,
            rate_limit_failures: 0,
            total_latency_ms: 0,
            error_samples: Vec::new(),
        }
    }

    /// Record one source attempt.
    ///
    /// # Arguments
    ///
    /// * `attempt` - Source attempt.
    pub fn record_attempt(&mut self, attempt: &SourceAttempt) {
        self.logical_calls += 1;
        self.attempts += 1;
        if attempt.did_retry {
            self.retry_count += 1;
        }
        if let Some(status_code) = attempt.status_code {
            *self.status_codes.entry(status_code).or_insert(0) += 1;
            if status_code == 429 && !attempt.did_succeed {
                self.rate_limit_failures += 1;
            }
        } else {
            self.transport_errors += 1;
        }
        if attempt.did_succeed {
            self.successes += 1;
        } else {
            self.failures += 1;
            if let Some(sample) = sanitize_error_sample(attempt.error.as_deref()) {
                if !self.error_samples.contains(&sample) {
                    self.error_samples.push(sample);
                    self.error_samples.truncate(5);
                }
            }
        }
    }
}

/// Path statistics aggregation key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct PathStatsKey {
    /// Index source identifier.
    pub source: String,
    /// Path kind.
    pub path: String,
    /// Current journal id.
    pub journal_id: Option<i64>,
    /// Current journal title.
    pub journal_title: String,
}

/// Aggregated path statistics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PathStats {
    /// Aggregation key.
    pub key: PathStatsKey,
    /// Path status.
    pub status: String,
    /// Started timestamp.
    pub started_at: String,
    /// Finished timestamp.
    pub finished_at: Option<String>,
    /// Work count.
    pub works_count: i64,
    /// Issue count.
    pub issues_count: i64,
    /// Article summaries count.
    pub article_summaries_count: i64,
    /// Article details count.
    pub article_details_count: i64,
    /// Written articles count.
    pub articles_written_count: i64,
    /// Deleted no-author articles count.
    pub articles_deleted_no_authors_count: i64,
    /// Error type.
    pub error_type: Option<String>,
    /// Error message.
    pub error_message: Option<String>,
}

/// Complete index run statistics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexRunStats {
    /// Run identifier.
    pub run_id: String,
    /// Source CSV filename.
    pub csv_file: String,
    /// Started timestamp.
    pub started_at: String,
    /// Finished timestamp.
    pub finished_at: Option<String>,
    /// Run status.
    pub status: String,
    /// Total journal count.
    pub total_journals: i64,
    /// Succeeded journal count.
    pub succeeded_journals: i64,
    /// Failed journal count.
    pub failed_journals: i64,
    /// Resumed journal count.
    pub resumed_journals: i64,
    /// Error summary.
    pub error_summary: Option<String>,
    /// Path statistics buckets.
    pub path_stats: BTreeMap<PathStatsKey, PathStats>,
    /// API statistics buckets.
    pub api_stats: BTreeMap<ApiStatsKey, ApiCallStats>,
}

impl IndexRunStats {
    /// Build an index run statistics object.
    ///
    /// # Arguments
    ///
    /// * `run_id` - Run identifier.
    /// * `csv_file` - Source CSV filename.
    /// * `started_at` - Started timestamp.
    ///
    /// # Returns
    ///
    /// Index run statistics.
    pub fn new(run_id: String, csv_file: String, started_at: String) -> Self {
        Self {
            run_id,
            csv_file,
            started_at,
            finished_at: None,
            status: "running".to_string(),
            total_journals: 0,
            succeeded_journals: 0,
            failed_journals: 0,
            resumed_journals: 0,
            error_summary: None,
            path_stats: BTreeMap::new(),
            api_stats: BTreeMap::new(),
        }
    }

    /// Start tracking one journal path.
    ///
    /// # Arguments
    ///
    /// * `source` - Source identifier.
    /// * `path` - Path kind.
    /// * `journal_id` - Current journal id.
    /// * `journal_title` - Current journal title.
    /// * `started_at` - Started timestamp.
    ///
    /// # Returns
    ///
    /// Path key.
    pub fn start_path(
        &mut self,
        source: &str,
        path: &str,
        journal_id: Option<i64>,
        journal_title: String,
        started_at: String,
    ) -> PathStatsKey {
        let key = PathStatsKey {
            source: source.to_string(),
            path: path.to_string(),
            journal_id,
            journal_title,
        };
        self.total_journals += 1;
        self.path_stats.insert(
            key.clone(),
            PathStats {
                key: key.clone(),
                status: "running".to_string(),
                started_at,
                finished_at: None,
                works_count: 0,
                issues_count: 0,
                article_summaries_count: 0,
                article_details_count: 0,
                articles_written_count: 0,
                articles_deleted_no_authors_count: 0,
                error_type: None,
                error_message: None,
            },
        );
        key
    }

    /// Increment path counters.
    ///
    /// # Arguments
    ///
    /// * `key` - Path key.
    /// * `counts` - Path counter increments.
    pub fn record_path_counts(&mut self, key: &PathStatsKey, counts: PathCountIncrements) {
        if let Some(stats) = self.path_stats.get_mut(key) {
            stats.works_count += counts.works_count;
            stats.issues_count += counts.issues_count;
            stats.article_summaries_count += counts.article_summaries_count;
            stats.article_details_count += counts.article_details_count;
            stats.articles_written_count += counts.articles_written_count;
            stats.articles_deleted_no_authors_count += counts.articles_deleted_no_authors_count;
        }
    }

    /// Finish a path bucket.
    ///
    /// # Arguments
    ///
    /// * `key` - Path key.
    /// * `status` - Final path status.
    /// * `finished_at` - Finished timestamp.
    /// * `error` - Optional error message.
    pub fn finish_path(
        &mut self,
        key: &PathStatsKey,
        status: &str,
        finished_at: String,
        error: Option<&str>,
    ) {
        if let Some(stats) = self.path_stats.get_mut(key) {
            stats.status = status.to_string();
            stats.finished_at = Some(finished_at);
            if let Some(error) = error {
                stats.error_type = error.split(':').next().map(str::to_string);
                stats.error_message = sanitize_error_sample(Some(error));
            }
        }
        match status {
            "succeeded" => self.succeeded_journals += 1,
            "failed" => self.failed_journals += 1,
            "resumed" => self.resumed_journals += 1,
            _ => {}
        }
    }

    /// Record captured source attempts.
    ///
    /// # Arguments
    ///
    /// * `attempts` - Source attempts.
    /// * `journal_id` - Current journal id.
    /// * `journal_title` - Current journal title.
    pub fn record_source_attempts(
        &mut self,
        attempts: &[SourceAttempt],
        journal_id: Option<i64>,
        journal_title: &str,
    ) {
        self.record_source_attempts_for_source("scholarly", attempts, journal_id, journal_title);
    }

    /// Record captured source attempts for an explicit index source.
    ///
    /// # Arguments
    ///
    /// * `source` - Index source identifier.
    /// * `attempts` - Source attempts.
    /// * `journal_id` - Current journal id.
    /// * `journal_title` - Current journal title.
    pub fn record_source_attempts_for_source(
        &mut self,
        source: &str,
        attempts: &[SourceAttempt],
        journal_id: Option<i64>,
        journal_title: &str,
    ) {
        for attempt in attempts {
            let key = ApiStatsKey {
                source: source.to_string(),
                service: attempt.service.clone(),
                endpoint: attempt.endpoint.clone(),
                method: attempt.method.clone(),
                url_path: sanitize_url_path(&attempt.url),
                journal_id,
                journal_title: journal_title.to_string(),
            };
            self.api_stats
                .entry(key.clone())
                .or_insert_with(|| ApiCallStats::new(key))
                .record_attempt(attempt);
        }
    }

    /// Finish the run.
    ///
    /// # Arguments
    ///
    /// * `status` - Final run status.
    /// * `finished_at` - Finished timestamp.
    /// * `error_summary` - Optional error summary.
    pub fn finish(&mut self, status: &str, finished_at: String, error_summary: Option<String>) {
        self.status = status.to_string();
        self.finished_at = Some(finished_at);
        self.error_summary = error_summary.and_then(|value| sanitize_error_sample(Some(&value)));
    }
}

/// Path counter increments.
#[derive(Debug, Clone, Copy, Default)]
pub struct PathCountIncrements {
    /// Work count increment.
    pub works_count: i64,
    /// Issue count increment.
    pub issues_count: i64,
    /// Article summaries count increment.
    pub article_summaries_count: i64,
    /// Article details count increment.
    pub article_details_count: i64,
    /// Written articles count increment.
    pub articles_written_count: i64,
    /// Deleted no-author articles count increment.
    pub articles_deleted_no_authors_count: i64,
}

/// Convert a URL or path into a query-free path value.
///
/// # Arguments
///
/// * `url` - Raw URL or path.
///
/// # Returns
///
/// Sanitized URL path.
pub fn sanitize_url_path(url: &str) -> String {
    let text = url.trim();
    if text.is_empty() {
        return String::new();
    }
    if let Some(after_scheme) = text.split_once("://").map(|(_, right)| right) {
        let path_start = after_scheme.find('/').unwrap_or(after_scheme.len());
        let path = &after_scheme[path_start..];
        return path
            .split(['?', '#'])
            .next()
            .filter(|value| !value.is_empty())
            .unwrap_or("/")
            .to_string();
    }
    text.split(['?', '#']).next().unwrap_or("").to_string()
}

/// Build a compact secret-free error sample.
///
/// # Arguments
///
/// * `error` - Error text.
///
/// # Returns
///
/// Sanitized error sample.
pub fn sanitize_error_sample(error: Option<&str>) -> Option<String> {
    let mut text = error?.trim().to_string();
    if text.is_empty() {
        return None;
    }
    text = sanitize_urls_in_text(&text);
    for key in [
        "api_key",
        "api-key",
        "x-api-key",
        "token",
        "secret",
        "password",
        "proxy",
    ] {
        text = redact_key_values(&text, key);
    }
    text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.len() > 500 {
        text.truncate(500);
    }
    Some(text)
}

fn sanitize_urls_in_text(text: &str) -> String {
    text.split_whitespace()
        .map(|part| {
            let trimmed = part.trim_matches(['\'', '"', '<', '>', ',']);
            if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                part.replace(trimmed, &sanitize_full_url(trimmed))
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn sanitize_full_url(url: &str) -> String {
    let Some((scheme, after_scheme)) = url.split_once("://") else {
        return sanitize_url_path(url);
    };
    let path_start = after_scheme.find('/').unwrap_or(after_scheme.len());
    let host = &after_scheme[..path_start];
    let path = after_scheme
        .get(path_start..)
        .unwrap_or("/")
        .split(['?', '#'])
        .next()
        .unwrap_or("/");
    format!("{scheme}://{host}{path}")
}

fn redact_key_values(text: &str, key: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    let needle_equals = format!("{key}=");
    let needle_colon = format!("{key}:");
    loop {
        let lower = rest.to_lowercase();
        let equals_index = lower.find(&needle_equals);
        let colon_index = lower.find(&needle_colon);
        let next = match (equals_index, colon_index) {
            (Some(left), Some(right)) => {
                Some((left.min(right), if left <= right { '=' } else { ':' }))
            }
            (Some(index), None) => Some((index, '=')),
            (None, Some(index)) => Some((index, ':')),
            (None, None) => None,
        };
        let Some((index, separator)) = next else {
            output.push_str(rest);
            break;
        };
        output.push_str(&rest[..index]);
        let value_start = index + key.len() + 1;
        output.push_str(&rest[index..value_start]);
        output.push_str("<redacted>");
        let value_end = rest[value_start..]
            .find(['&', ' ', ',', '\'', '"'])
            .map(|offset| value_start + offset)
            .unwrap_or(rest.len());
        if separator == ':' && value_end == value_start {
            output.push_str("<redacted>");
        }
        rest = &rest[value_end..];
    }
    output
}

#[cfg(test)]
mod tests {
    use ps_sources::SourceAttempt;

    use super::{sanitize_error_sample, sanitize_url_path, ApiCallStats, ApiStatsKey};

    #[test]
    fn stats_redact_query_secrets_and_aggregate_counts() {
        let key = ApiStatsKey {
            source: "scholarly".into(),
            service: "openalex".into(),
            endpoint: "works".into(),
            method: "GET".into(),
            url_path: sanitize_url_path("https://api.openalex.org/works?api_key=SECRET"),
            journal_id: Some(1),
            journal_title: "Test Journal".into(),
        };
        let mut stats = ApiCallStats::new(key);

        stats.record_attempt(&SourceAttempt {
            service: "openalex".into(),
            endpoint: "works".into(),
            method: "GET".into(),
            url: "https://api.openalex.org/works?api_key=SECRET".into(),
            status_code: Some(429),
            did_succeed: false,
            did_retry: true,
            error: Some("https://api.openalex.org/works?api_key=SECRET&token=TOKEN".into()),
        });
        stats.record_attempt(&SourceAttempt {
            service: "openalex".into(),
            endpoint: "works".into(),
            method: "GET".into(),
            url: "https://api.openalex.org/works".into(),
            status_code: Some(200),
            did_succeed: true,
            did_retry: false,
            error: None,
        });

        assert_eq!(stats.logical_calls, 2);
        assert_eq!(stats.attempts, 2);
        assert_eq!(stats.successes, 1);
        assert_eq!(stats.failures, 1);
        assert_eq!(stats.retry_count, 1);
        assert_eq!(stats.rate_limit_failures, 1);
        assert_eq!(stats.status_codes.get(&429), Some(&1));
        assert_eq!(stats.key.url_path, "/works");
        assert!(!format!("{:?}", stats).contains("SECRET"));
    }

    #[test]
    fn error_samples_drop_url_queries() {
        let sample = sanitize_error_sample(Some(
            "Client error https://api.openalex.org/works?api_key=SECRET&token=TOKEN",
        ))
        .expect("sample should sanitize");

        assert_eq!(sample, "Client error https://api.openalex.org/works");
    }
}
