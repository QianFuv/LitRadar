//! Durable, append-only security audit repository.

use std::error::Error;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use super::shared::{now_seconds, open_business_connection};

/// Default number of days retained in the durable security audit table.
pub const DEFAULT_AUDIT_RETENTION_DAYS: u32 = 180;

/// Minimum accepted security audit retention period.
pub const MIN_AUDIT_RETENTION_DAYS: u32 = 1;

/// Maximum accepted security audit retention period.
pub const MAX_AUDIT_RETENTION_DAYS: u32 = 3_650;

const RETENTION_INTERVAL_SECONDS: f64 = 86_400.0;
const RETENTION_DELETE_LIMIT: usize = 10_000;
const MAX_AUDIT_SYMBOL_BYTES: usize = 64;
const MAX_AUDIT_REQUEST_ID_BYTES: usize = 128;

static AUDIT_PERSISTENCE_FAILURE_COUNT: AtomicU64 = AtomicU64::new(0);

/// Fixed-schema security event written independently from ordinary tracing.
#[derive(Debug, Clone, PartialEq)]
pub struct SecurityAuditEvent {
    /// Optional authenticated actor identifier.
    pub actor_id: Option<i64>,
    /// Optional affected record identifier.
    pub target_id: Option<i64>,
    /// Stable snake-case operation name.
    pub action: &'static str,
    /// Stable terminal outcome.
    pub outcome: &'static str,
    /// Stable rejection or failure classification, or an empty string.
    pub reason: &'static str,
    /// Server-generated HTTP request identifier, or an empty string for local work.
    pub request_id: String,
    /// Stable client-source classification, or an empty string.
    pub source_class: &'static str,
    /// Stable limiter bucket classification, or an empty string.
    pub bucket: &'static str,
    /// Process-local rejection count when rate limited.
    pub rejected_count: u64,
    /// Retry delay returned to the client when rate limited.
    pub retry_after_seconds: u64,
    /// Unix timestamp recorded for the terminal event.
    pub occurred_at: f64,
}

impl SecurityAuditEvent {
    /// Create a terminal event with empty optional classifications.
    ///
    /// # Arguments
    ///
    /// * `action` - Stable snake-case operation name.
    /// * `outcome` - Stable terminal outcome.
    ///
    /// # Returns
    ///
    /// Event initialized with the current Unix timestamp.
    pub fn new(action: &'static str, outcome: &'static str) -> Self {
        Self {
            actor_id: None,
            target_id: None,
            action,
            outcome,
            reason: "",
            request_id: String::new(),
            source_class: "",
            bucket: "",
            rejected_count: 0,
            retry_after_seconds: 0,
            occurred_at: now_seconds(),
        }
    }

    /// Set the authenticated actor identifier.
    pub fn with_actor_id(mut self, actor_id: i64) -> Self {
        self.actor_id = Some(actor_id);
        self
    }

    /// Set the affected record identifier.
    pub fn with_target_id(mut self, target_id: i64) -> Self {
        self.target_id = Some(target_id);
        self
    }

    /// Set the stable terminal reason.
    pub fn with_reason(mut self, reason: &'static str) -> Self {
        self.reason = reason;
        self
    }

    /// Set the server-generated request identifier.
    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = request_id.into();
        self
    }

    /// Set authentication limiter classifications and counters.
    pub fn with_rate_limit(
        mut self,
        reason: &'static str,
        bucket: &'static str,
        source_class: &'static str,
        rejected_count: u64,
        retry_after_seconds: u64,
    ) -> Self {
        self.reason = reason;
        self.bucket = bucket;
        self.source_class = source_class;
        self.rejected_count = rejected_count;
        self.retry_after_seconds = retry_after_seconds;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_occurred_at(mut self, occurred_at: f64) -> Self {
        self.occurred_at = occurred_at;
        self
    }
}

/// Persisted security audit row returned to trusted repository callers.
#[derive(Debug, Clone, PartialEq)]
pub struct SecurityAuditRecord {
    /// Monotonic audit row identifier.
    pub id: i64,
    /// Optional authenticated actor identifier.
    pub actor_id: Option<i64>,
    /// Optional affected record identifier.
    pub target_id: Option<i64>,
    /// Stable operation name.
    pub action: String,
    /// Stable terminal outcome.
    pub outcome: String,
    /// Stable rejection or failure classification.
    pub reason: String,
    /// Server-generated request identifier.
    pub request_id: String,
    /// Stable client-source classification.
    pub source_class: String,
    /// Stable limiter bucket classification.
    pub bucket: String,
    /// Process-local rejection count when rate limited.
    pub rejected_count: u64,
    /// Retry delay returned to the client when rate limited.
    pub retry_after_seconds: u64,
    /// Unix timestamp recorded for the terminal event.
    pub occurred_at: f64,
}

/// Result of one persistent retention check.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SecurityAuditRetentionResult {
    /// Whether this call acquired the daily cleanup window.
    pub did_run: bool,
    /// Number of rows deleted in the bounded batch.
    pub deleted_count: usize,
    /// Whether additional expired rows remain for a later daily batch.
    pub has_more_expired: bool,
    /// Timestamp threshold used by the cleanup.
    pub cutoff: f64,
}

/// Durable audit repository error with no request or credential content.
#[derive(Debug)]
pub enum SecurityAuditError {
    /// SQLite rejected an audit operation.
    Sqlite(rusqlite::Error),
    /// Filesystem setup for the audit database failed.
    Io(std::io::Error),
    /// A fixed-schema event contained an invalid classification.
    InvalidEvent,
    /// Retention days fell outside the managed setting range.
    InvalidRetentionDays,
}

impl fmt::Display for SecurityAuditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(_) => formatter.write_str("Security audit persistence failed"),
            Self::Io(_) => formatter.write_str("Security audit persistence failed"),
            Self::InvalidEvent => formatter.write_str("Security audit event is invalid"),
            Self::InvalidRetentionDays => {
                formatter.write_str("Security audit retention days are invalid")
            }
        }
    }
}

impl Error for SecurityAuditError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::InvalidEvent | Self::InvalidRetentionDays => None,
        }
    }
}

impl From<rusqlite::Error> for SecurityAuditError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

/// Append one event in its own immediate transaction.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the migrated authentication database.
/// * `event` - Fixed-schema terminal event.
///
/// # Returns
///
/// Inserted audit row identifier.
pub fn append_security_audit_event(
    auth_db_path: impl AsRef<std::path::Path>,
    event: &SecurityAuditEvent,
) -> Result<i64, SecurityAuditError> {
    let result = (|| {
        let mut connection =
            open_business_connection(auth_db_path).map_err(business_error_to_audit_error)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(SecurityAuditError::from)?;
        let id = insert_security_audit_event(&transaction, event)?;
        transaction.commit().map_err(SecurityAuditError::from)?;
        Ok(id)
    })();
    result.map_err(record_security_audit_failure)
}

/// List persisted events in insertion order for trusted offline consumers and tests.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the migrated authentication database.
///
/// # Returns
///
/// All durable audit rows ordered by identifier.
pub fn list_security_audit_events(
    auth_db_path: impl AsRef<std::path::Path>,
) -> Result<Vec<SecurityAuditRecord>, SecurityAuditError> {
    let connection =
        open_business_connection(auth_db_path).map_err(business_error_to_audit_error)?;
    let mut statement = connection
        .prepare(
            "SELECT id, actor_id, target_id, action, outcome, reason, request_id, source_class, \
                    bucket, rejected_count, retry_after_seconds, occurred_at \
             FROM security_audit_events ORDER BY id",
        )
        .map_err(SecurityAuditError::from)?;
    let rows = statement
        .query_map([], |row| {
            Ok(SecurityAuditRecord {
                id: row.get(0)?,
                actor_id: row.get(1)?,
                target_id: row.get(2)?,
                action: row.get(3)?,
                outcome: row.get(4)?,
                reason: row.get(5)?,
                request_id: row.get(6)?,
                source_class: row.get(7)?,
                bucket: row.get(8)?,
                rejected_count: row.get(9)?,
                retry_after_seconds: row.get(10)?,
                occurred_at: row.get(11)?,
            })
        })
        .map_err(SecurityAuditError::from)?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(SecurityAuditError::from)
}

/// Delete one bounded batch after atomically claiming the persistent daily window.
///
/// # Arguments
///
/// * `auth_db_path` - Path to the migrated authentication database.
/// * `retention_days` - Managed number of days to retain.
/// * `current_time` - Current Unix timestamp.
///
/// # Returns
///
/// Transactional cleanup outcome.
pub fn cleanup_security_audit_events(
    auth_db_path: impl AsRef<std::path::Path>,
    retention_days: u32,
    current_time: f64,
) -> Result<SecurityAuditRetentionResult, SecurityAuditError> {
    let result = (|| {
        if !(MIN_AUDIT_RETENTION_DAYS..=MAX_AUDIT_RETENTION_DAYS).contains(&retention_days)
            || !current_time.is_finite()
        {
            return Err(SecurityAuditError::InvalidRetentionDays);
        }
        let cutoff = current_time - f64::from(retention_days) * RETENTION_INTERVAL_SECONDS;
        let mut connection =
            open_business_connection(auth_db_path).map_err(business_error_to_audit_error)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(SecurityAuditError::from)?;
        let last_cleanup = transaction
            .query_row(
                "SELECT last_retention_at FROM security_audit_maintenance WHERE id = 1",
                [],
                |row| row.get::<_, Option<f64>>(0),
            )
            .optional()
            .map_err(SecurityAuditError::from)?
            .flatten();
        if last_cleanup.is_some_and(|value| value >= current_time - RETENTION_INTERVAL_SECONDS) {
            transaction.commit().map_err(SecurityAuditError::from)?;
            return Ok(SecurityAuditRetentionResult {
                did_run: false,
                deleted_count: 0,
                has_more_expired: false,
                cutoff,
            });
        }
        let deleted_count = transaction
            .execute(
                "DELETE FROM security_audit_events \
                 WHERE id IN ( \
                     SELECT id FROM security_audit_events \
                     WHERE occurred_at < ?1 ORDER BY occurred_at, id LIMIT ?2 \
                 )",
                params![cutoff, RETENTION_DELETE_LIMIT],
            )
            .map_err(SecurityAuditError::from)?;
        let has_more_expired = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM security_audit_events WHERE occurred_at < ?1)",
                [cutoff],
                |row| row.get::<_, bool>(0),
            )
            .map_err(SecurityAuditError::from)?;
        let maintenance_rows = transaction
            .execute(
                "UPDATE security_audit_maintenance SET last_retention_at = ?1 WHERE id = 1",
                [current_time],
            )
            .map_err(SecurityAuditError::from)?;
        if maintenance_rows != 1 {
            return Err(SecurityAuditError::Sqlite(
                rusqlite::Error::QueryReturnedNoRows,
            ));
        }
        transaction.commit().map_err(SecurityAuditError::from)?;
        Ok(SecurityAuditRetentionResult {
            did_run: true,
            deleted_count,
            has_more_expired,
            cutoff,
        })
    })();
    result.map_err(record_security_audit_failure)
}

/// Return the process-local count of durable audit persistence failures.
pub fn security_audit_persistence_failure_count() -> u64 {
    AUDIT_PERSISTENCE_FAILURE_COUNT.load(Ordering::Relaxed)
}

/// Record a fixed audit persistence failure when execution never reached SQLite.
///
/// # Arguments
///
/// * `error_kind` - Static safe execution failure classification.
///
/// # Returns
///
/// Updated process-local failure count.
pub fn report_security_audit_persistence_failure(error_kind: &'static str) -> u64 {
    let failure_count = AUDIT_PERSISTENCE_FAILURE_COUNT
        .fetch_add(1, Ordering::Relaxed)
        .saturating_add(1);
    tracing::error!(
        event = "audit.persistence_failed",
        component = "security",
        outcome = "failure",
        error_kind,
        failure_count,
    );
    failure_count
}

pub(crate) fn insert_required_security_audit_event(
    connection: &Connection,
    event: &SecurityAuditEvent,
) -> Result<i64, SecurityAuditError> {
    insert_security_audit_event(connection, event).map_err(record_security_audit_failure)
}

fn insert_security_audit_event(
    connection: &Connection,
    event: &SecurityAuditEvent,
) -> Result<i64, SecurityAuditError> {
    validate_event(event)?;
    connection.execute(
        "INSERT INTO security_audit_events \
         (actor_id, target_id, action, outcome, reason, request_id, source_class, bucket, \
          rejected_count, retry_after_seconds, occurred_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            event.actor_id,
            event.target_id,
            event.action,
            event.outcome,
            event.reason,
            event.request_id.as_str(),
            event.source_class,
            event.bucket,
            event.rejected_count,
            event.retry_after_seconds,
            event.occurred_at,
        ],
    )?;
    Ok(connection.last_insert_rowid())
}

fn validate_event(event: &SecurityAuditEvent) -> Result<(), SecurityAuditError> {
    if !is_valid_symbol(event.action, false)
        || !is_valid_symbol(event.outcome, false)
        || !is_valid_symbol(event.reason, true)
        || !is_valid_symbol(event.source_class, true)
        || !is_valid_symbol(event.bucket, true)
        || event.actor_id.is_some_and(|value| value <= 0)
        || event.target_id.is_some_and(|value| value <= 0)
        || !event.occurred_at.is_finite()
        || event.request_id.len() > MAX_AUDIT_REQUEST_ID_BYTES
        || !event
            .request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(SecurityAuditError::InvalidEvent);
    }
    Ok(())
}

fn is_valid_symbol(value: &str, is_empty_allowed: bool) -> bool {
    (is_empty_allowed || !value.is_empty())
        && value.len() <= MAX_AUDIT_SYMBOL_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn business_error_to_audit_error(error: super::BusinessRepositoryError) -> SecurityAuditError {
    match error {
        super::BusinessRepositoryError::Sqlite(error) => SecurityAuditError::Sqlite(error),
        super::BusinessRepositoryError::Io(error) => SecurityAuditError::Io(error),
        _ => SecurityAuditError::InvalidEvent,
    }
}

fn record_security_audit_failure(error: SecurityAuditError) -> SecurityAuditError {
    report_security_audit_persistence_failure(match &error {
        SecurityAuditError::Sqlite(_) => "sqlite",
        SecurityAuditError::Io(_) => "io",
        SecurityAuditError::InvalidEvent => "invalid_event",
        SecurityAuditError::InvalidRetentionDays => "invalid_retention_days",
    });
    error
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::tempdir;

    use super::*;
    use crate::migrate_auth_database;

    #[test]
    fn security_audit_events_persist_under_concurrent_log_pressure_without_content() {
        let temp_dir = tempdir().expect("temporary database should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("audit schema should migrate");
        let auth_db_path = Arc::new(auth_db_path);
        let mut writers = Vec::new();
        for worker in 0..4_i64 {
            let auth_db_path = Arc::clone(&auth_db_path);
            writers.push(std::thread::spawn(move || {
                for sequence in 0..50_i64 {
                    tracing::info!(event = "ordinary.pressure", worker, sequence);
                    append_security_audit_event(
                        auth_db_path.as_path(),
                        &SecurityAuditEvent::new("login", "rejected")
                            .with_reason("authentication_failed")
                            .with_request_id(format!("request-{worker}-{sequence}")),
                    )
                    .expect("audit event should persist");
                }
            }));
        }
        for writer in writers {
            writer.join().expect("audit writer should finish");
        }

        let records =
            list_security_audit_events(auth_db_path.as_path()).expect("audit records should load");
        assert_eq!(records.len(), 200);
        let encoded = format!("{records:?}");
        assert!(!encoded.contains("password_sentinel"));
        assert!(!encoded.contains("token_sentinel"));
    }

    #[test]
    fn security_audit_retention_is_daily_bounded_and_transactional() {
        let temp_dir = tempdir().expect("temporary database should be created");
        let auth_db_path = temp_dir.path().join("auth.sqlite");
        migrate_auth_database(&auth_db_path).expect("audit schema should migrate");
        append_security_audit_event(
            &auth_db_path,
            &SecurityAuditEvent::new("login", "completed").with_occurred_at(1.0),
        )
        .expect("expired event should persist");
        append_security_audit_event(
            &auth_db_path,
            &SecurityAuditEvent::new("login", "completed").with_occurred_at(20_000_000.0),
        )
        .expect("current event should persist");

        let first = cleanup_security_audit_events(&auth_db_path, 180, 20_000_000.0)
            .expect("first retention should run");
        let second = cleanup_security_audit_events(&auth_db_path, 180, 20_000_001.0)
            .expect("same-day retention should skip");

        assert!(first.did_run);
        assert_eq!(first.deleted_count, 1);
        assert!(!first.has_more_expired);
        assert!(!second.did_run);
        assert_eq!(list_security_audit_events(&auth_db_path).unwrap().len(), 1);

        let connection = open_business_connection(&auth_db_path).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_audit_retention BEFORE DELETE ON security_audit_events \
                 BEGIN SELECT RAISE(FAIL, 'forced audit retention failure'); END;",
            )
            .unwrap();
        append_security_audit_event(
            &auth_db_path,
            &SecurityAuditEvent::new("login", "completed").with_occurred_at(2.0),
        )
        .unwrap();
        connection
            .execute(
                "UPDATE security_audit_maintenance SET last_retention_at = NULL WHERE id = 1",
                [],
            )
            .unwrap();
        let failures_before = security_audit_persistence_failure_count();
        let failure = cleanup_security_audit_events(&auth_db_path, 180, 20_100_000.0)
            .expect_err("forced delete failure should roll back");
        assert!(matches!(failure, SecurityAuditError::Sqlite(_)));
        assert_eq!(
            security_audit_persistence_failure_count(),
            failures_before + 1
        );
        let last_retention_at: Option<f64> = connection
            .query_row(
                "SELECT last_retention_at FROM security_audit_maintenance WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(last_retention_at.is_none());

        connection
            .execute_batch(
                "DROP TRIGGER fail_audit_retention;
                 DELETE FROM security_audit_maintenance WHERE id = 1;",
            )
            .unwrap();
        let missing_marker = cleanup_security_audit_events(&auth_db_path, 180, 20_200_000.0)
            .expect_err("missing maintenance marker should roll back retention");
        assert!(matches!(missing_marker, SecurityAuditError::Sqlite(_)));
        assert_eq!(list_security_audit_events(&auth_db_path).unwrap().len(), 2);
    }
}
