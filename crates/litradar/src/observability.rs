//! Process-level structured logging setup for the LitRadar binary.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use litradar_storage::{
    load_runtime_logging_settings, DEFAULT_RUNTIME_LOG_FILTER, DEFAULT_RUNTIME_LOG_FORMAT,
};
use tracing_appender::non_blocking::{ErrorCounter, NonBlockingBuilder, WorkerGuard};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

const LOG_BUFFERED_LINES_LIMIT: usize = 4_096;

/// Guard that flushes buffered log events and reports overload loss on shutdown.
pub(crate) struct ObservabilityGuard {
    error_counter: ErrorCounter,
    format: LogFormat,
    worker_guard: Option<WorkerGuard>,
}

impl ObservabilityGuard {
    /// Flush buffered events and report any lines dropped by the lossy writer.
    pub(crate) fn shutdown(mut self) {
        self.flush_and_report();
    }

    fn flush_and_report(&mut self) {
        let Some(worker_guard) = self.worker_guard.take() else {
            return;
        };
        drop(worker_guard);
        let dropped_count = self.error_counter.dropped_lines();
        if dropped_count > 0 {
            write_dropped_event(self.format, dropped_count);
        }
    }
}

impl Drop for ObservabilityGuard {
    fn drop(&mut self) {
        self.flush_and_report();
    }
}

/// Fixed public reason for process-level logging initialization failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObservabilityError {
    /// The persisted filter does not use valid tracing-subscriber syntax.
    InvalidFilter,
    /// The persisted format is unsupported.
    InvalidFormat,
    /// Runtime logging settings could not be resolved or read safely.
    RuntimeSettingsUnavailable,
    /// Another global tracing subscriber is already active.
    SubscriberAlreadyInitialized,
}

impl fmt::Display for ObservabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFilter => formatter.write_str("invalid LitRadar log filter"),
            Self::InvalidFormat => formatter.write_str("invalid LitRadar log format"),
            Self::RuntimeSettingsUnavailable => {
                formatter.write_str("LitRadar runtime logging settings are unavailable")
            }
            Self::SubscriberAlreadyInitialized => {
                formatter.write_str("LitRadar logging is already initialized")
            }
        }
    }
}

impl Error for ObservabilityError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogFormat {
    Json,
    Compact,
}

impl LogFormat {
    fn parse(value: Option<&str>) -> Result<Self, ObservabilityError> {
        match value.unwrap_or(DEFAULT_RUNTIME_LOG_FORMAT) {
            "json" => Ok(Self::Json),
            "compact" => Ok(Self::Compact),
            _ => Err(ObservabilityError::InvalidFormat),
        }
    }
}

struct ObservabilityConfig {
    filter: EnvFilter,
    format: LogFormat,
}

impl ObservabilityConfig {
    fn from_args(args: &[String]) -> Result<Self, ObservabilityError> {
        let current_dir =
            std::env::current_dir().map_err(|_| ObservabilityError::RuntimeSettingsUnavailable)?;
        Self::from_args_with_current_dir(args, &current_dir)
    }

    fn from_args_with_current_dir(
        args: &[String],
        current_dir: &Path,
    ) -> Result<Self, ObservabilityError> {
        let auth_db_path = resolve_auth_database_path(args, current_dir)?;
        let settings = load_runtime_logging_settings(auth_db_path)
            .map_err(|_| ObservabilityError::RuntimeSettingsUnavailable)?;
        Self::from_values(Some(&settings.log_filter), Some(&settings.log_format))
    }

    fn from_values(filter: Option<&str>, format: Option<&str>) -> Result<Self, ObservabilityError> {
        let filter = EnvFilter::try_new(filter.unwrap_or(DEFAULT_RUNTIME_LOG_FILTER))
            .map_err(|_| ObservabilityError::InvalidFilter)?;
        let format = LogFormat::parse(format)?;
        Ok(Self { filter, format })
    }
}

/// Initialize the one global tracing subscriber used by the LitRadar process.
///
/// # Returns
///
/// A guard that must remain alive until all application work and terminal events finish.
pub(crate) fn initialize(args: &[String]) -> Result<ObservabilityGuard, ObservabilityError> {
    let ObservabilityConfig { filter, format } = ObservabilityConfig::from_args(args)?;
    let (writer, worker_guard) = NonBlockingBuilder::default()
        .buffered_lines_limit(LOG_BUFFERED_LINES_LIMIT)
        .lossy(true)
        .finish(std::io::stderr());
    let error_counter = writer.error_counter();
    let initialization = match format {
        LogFormat::Json => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(writer)
            .with_ansi(false)
            .with_target(true)
            .json()
            .flatten_event(true)
            .with_current_span(true)
            .with_span_list(true)
            .finish()
            .try_init(),
        LogFormat::Compact => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(writer)
            .with_ansi(false)
            .with_target(true)
            .compact()
            .finish()
            .try_init(),
    };
    initialization.map_err(|_| ObservabilityError::SubscriberAlreadyInitialized)?;
    install_panic_hook();
    Ok(ObservabilityGuard {
        error_counter,
        format,
        worker_guard: Some(worker_guard),
    })
}

fn resolve_auth_database_path(
    args: &[String],
    current_dir: &Path,
) -> Result<PathBuf, ObservabilityError> {
    let project_root = option_value(args, "--project-root")?
        .map(PathBuf::from)
        .unwrap_or_else(|| current_dir.to_path_buf());
    if let Some(auth_db_path) = option_value(args, "--auth-db")? {
        return Ok(PathBuf::from(auth_db_path));
    }
    Ok(project_root.join("data").join("auth.sqlite"))
}

fn option_value<'a>(args: &'a [String], name: &str) -> Result<Option<&'a str>, ObservabilityError> {
    let Some(index) = args.iter().position(|argument| argument == name) else {
        return Ok(None);
    };
    args.get(index + 1)
        .map(String::as_str)
        .map(Some)
        .ok_or(ObservabilityError::RuntimeSettingsUnavailable)
}

fn install_panic_hook() {
    std::panic::set_hook(Box::new(|panic_info| {
        let (file, line, column) = panic_info.location().map_or(("unknown", 0, 0), |location| {
            let file = Path::new(location.file())
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown");
            (file, location.line(), location.column())
        });
        if tracing::enabled!(tracing::Level::ERROR) {
            tracing::error!(
                event = "process.panicked",
                component = "runtime",
                panic_file = file,
                panic_line = line,
                panic_column = column,
                "LitRadar process panicked"
            );
        } else {
            eprintln!("LitRadar process panicked");
        }
    }));
}

fn write_dropped_event(format: LogFormat, dropped_count: usize) {
    match format {
        LogFormat::Json => {
            let event = serde_json::json!({
                "level": "WARN",
                "target": "litradar",
                "event": "logging.events_dropped",
                "component": "logging",
                "dropped_count": dropped_count,
            });
            eprintln!("{event}");
        }
        LogFormat::Compact => eprintln!(
            "WARN litradar logging.events_dropped component=logging dropped_count={dropped_count}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Condvar, Mutex};

    use litradar_storage::{
        migrate_auth_database, open_sqlite_connection, upsert_runtime_settings, SecretCodec,
    };
    use tempfile::tempdir;
    use tracing_appender::non_blocking::NonBlockingBuilder;

    use super::{resolve_auth_database_path, LogFormat, ObservabilityConfig, ObservabilityError};

    #[test]
    fn configuration_defaults_to_json_and_rejects_invalid_values() {
        let default = ObservabilityConfig::from_values(None, None)
            .expect("default logging configuration should be valid");
        assert_eq!(default.format, LogFormat::Json);
        assert_eq!(
            ObservabilityConfig::from_values(Some("["), None)
                .err()
                .expect("invalid filter should fail"),
            ObservabilityError::InvalidFilter
        );
        assert_eq!(
            ObservabilityConfig::from_values(None, Some("pretty"))
                .err()
                .expect("unsupported format should fail"),
            ObservabilityError::InvalidFormat
        );
        assert_eq!(
            ObservabilityConfig::from_values(Some("off"), Some("compact"))
                .expect("explicit compact configuration should be valid")
                .format,
            LogFormat::Compact
        );
    }

    #[test]
    fn configuration_loads_persisted_values_from_the_selected_auth_database() {
        let root = tempdir().expect("temporary project root should be created");
        let default_auth_db_path = root.path().join("data").join("auth.sqlite");
        let default = ObservabilityConfig::from_args_with_current_dir(&[], root.path())
            .expect("missing database should use logging defaults");
        assert_eq!(default.format, LogFormat::Json);
        assert!(!default_auth_db_path.exists());

        migrate_auth_database(&default_auth_db_path).expect("auth database should migrate");
        upsert_runtime_settings(
            &default_auth_db_path,
            &SecretCodec::from_key([41_u8; 32]),
            &HashMap::from([
                ("log_format".to_string(), Some("compact".to_string())),
                ("log_filter".to_string(), Some("off".to_string())),
            ]),
            &HashMap::new(),
        )
        .expect("logging settings should persist");
        let configured = ObservabilityConfig::from_args_with_current_dir(&[], root.path())
            .expect("persisted logging settings should load");
        assert_eq!(configured.format, LogFormat::Compact);
        assert_eq!(configured.filter.to_string(), "off");

        let custom_auth_db_path = root.path().join("custom.sqlite");
        migrate_auth_database(&custom_auth_db_path).expect("custom auth database should migrate");
        upsert_runtime_settings(
            &custom_auth_db_path,
            &SecretCodec::from_key([42_u8; 32]),
            &HashMap::from([("log_filter".to_string(), Some("litradar=debug".to_string()))]),
            &HashMap::new(),
        )
        .expect("custom logging settings should persist");
        let custom_args = vec![
            "--auth-db".to_string(),
            custom_auth_db_path.to_string_lossy().into_owned(),
        ];
        let custom = ObservabilityConfig::from_args_with_current_dir(&custom_args, root.path())
            .expect("explicit auth database should be selected");
        assert_eq!(custom.filter.to_string(), "litradar=debug");

        let connection =
            open_sqlite_connection(&custom_auth_db_path).expect("custom database should open");
        connection
            .execute(
                "UPDATE runtime_settings SET value = ?1 WHERE key = 'log_filter'",
                ["["],
            )
            .expect("invalid direct filter fixture should update");
        assert_eq!(
            ObservabilityConfig::from_args_with_current_dir(&custom_args, root.path())
                .err()
                .expect("invalid persisted filter should fail"),
            ObservabilityError::InvalidFilter
        );
        connection
            .execute(
                "INSERT INTO runtime_settings (key, value, updated_at) VALUES ('log_format', 'pretty', 1.0)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [],
            )
            .expect("invalid direct format fixture should update");
        connection
            .execute(
                "UPDATE runtime_settings SET value = 'off' WHERE key = 'log_filter'",
                [],
            )
            .expect("valid filter fixture should update");
        assert_eq!(
            ObservabilityConfig::from_args_with_current_dir(&custom_args, root.path())
                .err()
                .expect("invalid persisted format should fail"),
            ObservabilityError::InvalidFormat
        );
    }

    #[test]
    fn auth_database_resolution_matches_command_options() {
        let current_dir = Path::new("fixture-root");
        assert_eq!(
            resolve_auth_database_path(&[], current_dir)
                .expect("default auth database should resolve"),
            current_dir.join("data").join("auth.sqlite")
        );
        assert_eq!(
            resolve_auth_database_path(
                &["--project-root".to_string(), "deployment".to_string()],
                current_dir,
            )
            .expect("project auth database should resolve"),
            PathBuf::from("deployment").join("data").join("auth.sqlite")
        );
        assert_eq!(
            resolve_auth_database_path(
                &[
                    "--project-root".to_string(),
                    "deployment".to_string(),
                    "--auth-db".to_string(),
                    "explicit.sqlite".to_string(),
                ],
                current_dir,
            )
            .expect("explicit auth database should win"),
            PathBuf::from("explicit.sqlite")
        );
        assert_eq!(
            resolve_auth_database_path(&["--auth-db".to_string()], current_dir)
                .expect_err("missing option value should fail"),
            ObservabilityError::RuntimeSettingsUnavailable
        );
    }

    #[test]
    fn lossy_writer_reports_drops_without_blocking_producers() {
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let writer = BlockingWriter {
            release: Arc::clone(&release),
        };
        let (non_blocking, worker_guard) = NonBlockingBuilder::default()
            .buffered_lines_limit(1)
            .lossy(true)
            .finish(writer);
        let error_counter = non_blocking.error_counter();

        for line in 0..1_000 {
            let mut producer = non_blocking.clone();
            writeln!(producer, "event-{line}").expect("lossy write should not fail");
        }

        assert!(error_counter.dropped_lines() > 0);
        let (lock, wakeup) = &*release;
        *lock.lock().expect("release lock should not be poisoned") = true;
        wakeup.notify_all();
        drop(worker_guard);
    }

    struct BlockingWriter {
        release: Arc<(Mutex<bool>, Condvar)>,
    }

    impl Write for BlockingWriter {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            let (lock, wakeup) = &*self.release;
            let mut is_released = lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            while !*is_released {
                is_released = wakeup
                    .wait(is_released)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
