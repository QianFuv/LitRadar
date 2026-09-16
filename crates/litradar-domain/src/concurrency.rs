//! Shared index-concurrency limits and validation.

use std::error::Error;
use std::fmt;

/// Minimum source worker count accepted by index entry points.
pub const INDEX_WORKER_COUNT_MIN: usize = 1;
/// Maximum source worker count accepted by index entry points.
pub const INDEX_WORKER_COUNT_MAX: usize = 32;
/// Minimum journal process count accepted by index entry points.
pub const INDEX_PROCESS_COUNT_MIN: usize = 1;
/// Maximum journal process count accepted by index entry points.
pub const INDEX_PROCESS_COUNT_MAX: usize = 32;
/// Maximum configured process-by-source-worker capacity.
pub const INDEX_AGGREGATE_CONCURRENCY_MAX: usize = 32;
/// Maximum source workers per Scholarly journal process.
pub const SCHOLARLY_WORKER_COUNT_MAX: usize = 32;
/// Maximum Scholarly journal process count.
pub const SCHOLARLY_PROCESS_COUNT_MAX: usize = 3;
/// Maximum Scholarly process-by-worker capacity under independently paced APIs.
pub const SCHOLARLY_AGGREGATE_CONCURRENCY_MAX: usize = 96;
/// Default source workers for generic and Scholarly indexing.
pub const INDEX_WORKER_COUNT_DEFAULT: usize = 6;
/// Default process count for generic and domestic CNKI indexing.
pub const INDEX_PROCESS_COUNT_DEFAULT: usize = 1;
/// Default Scholarly journal process count.
pub const SCHOLARLY_PROCESS_COUNT_DEFAULT: usize = 3;
/// Default domestic CNKI detail workers, verified by bounded live calibration.
pub const DOMESTIC_CNKI_WORKER_COUNT_DEFAULT: usize = 6;
/// Maximum detail workers per domestic CNKI journal process.
pub const DOMESTIC_CNKI_WORKER_COUNT_MAX: usize = 32;

/// Provider-specific defaults and configured capacity limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexConcurrencyProfile {
    /// Existing defaults and capacity for other providers.
    Generic,
    /// Independently paced scholarly APIs.
    Scholarly,
    /// Shared-session domestic CNKI detail requests.
    DomesticCnki,
}

impl IndexConcurrencyProfile {
    /// Return the configured aggregate capacity limit for this provider.
    pub fn aggregate_limit(self) -> usize {
        match self {
            Self::Scholarly => SCHOLARLY_AGGREGATE_CONCURRENCY_MAX,
            Self::Generic | Self::DomesticCnki => INDEX_AGGREGATE_CONCURRENCY_MAX,
        }
    }
}

/// Validate only explicitly supplied individual counts before provider selection.
pub fn validate_index_concurrency_options(
    worker_count: Option<usize>,
    process_count: Option<usize>,
) -> Result<(), IndexConcurrencyError> {
    if worker_count
        .is_some_and(|count| !(INDEX_WORKER_COUNT_MIN..=INDEX_WORKER_COUNT_MAX).contains(&count))
    {
        return Err(concurrency_error(format!(
            "worker_count must be between {INDEX_WORKER_COUNT_MIN} and {INDEX_WORKER_COUNT_MAX}"
        )));
    }
    if process_count
        .is_some_and(|count| !(INDEX_PROCESS_COUNT_MIN..=INDEX_PROCESS_COUNT_MAX).contains(&count))
    {
        return Err(concurrency_error(format!(
            "process_count must be between {INDEX_PROCESS_COUNT_MIN} and {INDEX_PROCESS_COUNT_MAX}"
        )));
    }
    Ok(())
}

/// Resolve each omitted count independently, rejecting explicit invalid capacities.
pub fn resolve_index_concurrency(
    worker_count: Option<usize>,
    process_count: Option<usize>,
    profile: IndexConcurrencyProfile,
) -> Result<IndexConcurrency, IndexConcurrencyError> {
    let default_workers = if profile == IndexConcurrencyProfile::DomesticCnki {
        DOMESTIC_CNKI_WORKER_COUNT_DEFAULT
    } else {
        INDEX_WORKER_COUNT_DEFAULT
    };
    let default_processes = if profile == IndexConcurrencyProfile::Scholarly {
        SCHOLARLY_PROCESS_COUNT_DEFAULT
    } else {
        INDEX_PROCESS_COUNT_DEFAULT
    };
    validate_index_concurrency(
        worker_count.unwrap_or(default_workers),
        process_count.unwrap_or(default_processes),
        profile == IndexConcurrencyProfile::Scholarly,
    )
}

/// Validated configured index-concurrency capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexConcurrency {
    /// Configured source workers per journal process.
    pub worker_count: usize,
    /// Configured journal worker processes.
    pub process_count: usize,
    /// Maximum configured concurrent source work across processes.
    pub aggregate_capacity: usize,
}

/// A stable index-concurrency validation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexConcurrencyError {
    message: String,
}

impl fmt::Display for IndexConcurrencyError {
    /// Format the safe configuration detail.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for IndexConcurrencyError {}

/// Validate generic and optional Scholarly index concurrency.
///
/// # Arguments
///
/// * `worker_count` - Configured source workers per process.
/// * `process_count` - Configured journal worker processes.
/// * `has_scholarly_route` - Whether any selected runtime route uses Scholarly.
///
/// # Returns
///
/// Validated counts and aggregate capacity.
pub fn validate_index_concurrency(
    worker_count: usize,
    process_count: usize,
    has_scholarly_route: bool,
) -> Result<IndexConcurrency, IndexConcurrencyError> {
    validate_index_concurrency_options(Some(worker_count), Some(process_count))?;
    if has_scholarly_route && worker_count > SCHOLARLY_WORKER_COUNT_MAX {
        return Err(concurrency_error(format!(
            "worker_count must be at most {SCHOLARLY_WORKER_COUNT_MAX} for scholarly indexing"
        )));
    }
    if has_scholarly_route && process_count > SCHOLARLY_PROCESS_COUNT_MAX {
        return Err(concurrency_error(format!(
            "process_count must be at most {SCHOLARLY_PROCESS_COUNT_MAX} for scholarly indexing"
        )));
    }
    let aggregate_limit = if has_scholarly_route {
        SCHOLARLY_AGGREGATE_CONCURRENCY_MAX
    } else {
        INDEX_AGGREGATE_CONCURRENCY_MAX
    };
    let aggregate_capacity = worker_count.checked_mul(process_count).ok_or_else(|| {
        concurrency_error(format!(
            "process_count * worker_count must be at most {aggregate_limit}"
        ))
    })?;
    if aggregate_capacity > aggregate_limit {
        return Err(concurrency_error(format!(
            "process_count * worker_count must be at most {aggregate_limit}"
        )));
    }
    Ok(IndexConcurrency {
        worker_count,
        process_count,
        aggregate_capacity,
    })
}

/// Validate a direct domestic CNKI Provider worker count.
///
/// # Arguments
///
/// * `worker_count` - Requested fixed detail-pool size.
///
/// # Returns
///
/// The validated pool size.
pub fn validate_domestic_cnki_worker_count(
    worker_count: usize,
) -> Result<usize, IndexConcurrencyError> {
    if !(INDEX_WORKER_COUNT_MIN..=DOMESTIC_CNKI_WORKER_COUNT_MAX).contains(&worker_count) {
        return Err(concurrency_error(format!(
            "domestic CNKI worker_count must be between {INDEX_WORKER_COUNT_MIN} and {DOMESTIC_CNKI_WORKER_COUNT_MAX}"
        )));
    }
    Ok(worker_count)
}

fn concurrency_error(message: String) -> IndexConcurrencyError {
    IndexConcurrencyError { message }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrency_regression_scholarly_accepts_the_api_budgeted_capacity() {
        let capacity = validate_index_concurrency(32, 3, true)
            .expect("three Scholarly executors may each use 32 DOI workers");
        assert_eq!(capacity.aggregate_capacity, 96);
    }

    #[test]
    fn index_concurrency_enforces_generic_provider_and_aggregate_limits() {
        assert_eq!(
            validate_index_concurrency(6, 3, true)
                .expect("the Scholarly boundary should pass")
                .aggregate_capacity,
            18
        );
        assert_eq!(
            validate_index_concurrency(32, 1, false)
                .expect("the generic worker boundary should pass")
                .aggregate_capacity,
            32
        );
        for (workers, processes, has_scholarly, detail) in [
            (0, 1, false, "worker_count must be between 1 and 32"),
            (33, 1, false, "worker_count must be between 1 and 32"),
            (1, 0, false, "process_count must be between 1 and 32"),
            (1, 33, false, "process_count must be between 1 and 32"),
            (33, 1, true, "worker_count must be between 1 and 32"),
            (
                6,
                4,
                true,
                "process_count must be at most 3 for scholarly indexing",
            ),
            (
                17,
                2,
                false,
                "process_count * worker_count must be at most 32",
            ),
        ] {
            assert_eq!(
                validate_index_concurrency(workers, processes, has_scholarly)
                    .expect_err("out-of-range concurrency should fail")
                    .to_string(),
                detail
            );
        }
    }

    #[test]
    fn omitted_counts_resolve_independently_without_clamping_explicit_values() {
        use IndexConcurrencyProfile::{DomesticCnki, Generic, Scholarly};
        for (profile, workers, processes, expected) in [
            (Scholarly, None, None, (6, 3)),
            (Scholarly, Some(32), None, (32, 3)),
            (Scholarly, None, Some(1), (6, 1)),
            (Generic, None, None, (6, 1)),
            (
                DomesticCnki,
                None,
                None,
                (DOMESTIC_CNKI_WORKER_COUNT_DEFAULT, 1),
            ),
            (DomesticCnki, Some(4), Some(2), (4, 2)),
        ] {
            let capacity = resolve_index_concurrency(workers, processes, profile).unwrap();
            assert_eq!((capacity.worker_count, capacity.process_count), expected);
        }
        assert!(resolve_index_concurrency(None, Some(4), Scholarly).is_err());
        assert!(resolve_index_concurrency(Some(32), Some(3), DomesticCnki).is_err());
        assert!(resolve_index_concurrency(Some(0), None, Generic).is_err());
    }

    #[test]
    fn domestic_cnki_worker_count_rejects_zero_and_one_over_maximum() {
        assert_eq!(
            validate_domestic_cnki_worker_count(DOMESTIC_CNKI_WORKER_COUNT_MAX)
                .expect("the domestic CNKI boundary should pass"),
            DOMESTIC_CNKI_WORKER_COUNT_MAX
        );
        for worker_count in [0, DOMESTIC_CNKI_WORKER_COUNT_MAX + 1] {
            assert_eq!(
                validate_domestic_cnki_worker_count(worker_count)
                    .expect_err("out-of-range domestic workers should fail")
                    .to_string(),
                "domestic CNKI worker_count must be between 1 and 32"
            );
        }
    }
}
