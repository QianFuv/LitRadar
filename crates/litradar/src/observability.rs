//! Process-level structured logging setup for the LitRadar binary.

use std::error::Error;
use std::fmt;
use std::path::Path;

use tracing_appender::non_blocking::{ErrorCounter, NonBlockingBuilder, WorkerGuard};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

const DEFAULT_LOG_FILTER: &str = concat!(
    "warn,",
    "litradar=info,",
    "litradar_api=info,",
    "litradar_cli=info,",
    "litradar_index=info,",
    "litradar_sources=info,",
    "litradar_storage=info,",
    "litradar_worker=info"
);
const LOG_BUFFERED_LINES_LIMIT: usize = 4_096;
const LOG_FILTER_ENV: &str = "LITRADAR_LOG_FILTER";
const LOG_FORMAT_ENV: &str = "LITRADAR_LOG_FORMAT";

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
    /// The filter variable is not valid Unicode or filter syntax.
    InvalidFilter,
    /// The format variable is not valid Unicode or a supported format.
    InvalidFormat,
    /// Another global tracing subscriber is already active.
    SubscriberAlreadyInitialized,
}

impl fmt::Display for ObservabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFilter => formatter.write_str("invalid LitRadar log filter"),
            Self::InvalidFormat => formatter.write_str("invalid LitRadar log format"),
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
        match value.unwrap_or("json") {
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
    fn from_env() -> Result<Self, ObservabilityError> {
        let filter = unicode_env(LOG_FILTER_ENV, ObservabilityError::InvalidFilter)?;
        let format = unicode_env(LOG_FORMAT_ENV, ObservabilityError::InvalidFormat)?;
        Self::from_values(filter.as_deref(), format.as_deref())
    }

    fn from_values(filter: Option<&str>, format: Option<&str>) -> Result<Self, ObservabilityError> {
        let filter = EnvFilter::try_new(filter.unwrap_or(DEFAULT_LOG_FILTER))
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
pub(crate) fn initialize() -> Result<ObservabilityGuard, ObservabilityError> {
    let ObservabilityConfig { filter, format } = ObservabilityConfig::from_env()?;
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

fn unicode_env(
    name: &str,
    error: ObservabilityError,
) -> Result<Option<String>, ObservabilityError> {
    std::env::var_os(name)
        .map(|value| value.into_string().map_err(|_| error))
        .transpose()
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
    use std::io::Write;
    use std::sync::{Arc, Condvar, Mutex};

    use tracing_appender::non_blocking::NonBlockingBuilder;

    use super::{LogFormat, ObservabilityConfig, ObservabilityError};

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
