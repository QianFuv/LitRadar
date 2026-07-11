//! Observability setup for the API server.

use std::sync::OnceLock;

use tracing_subscriber::EnvFilter;

const DEFAULT_LOG_FILTER: &str = "litradar_api=info,tower_http=info";

static TRACING_INITIALIZED: OnceLock<()> = OnceLock::new();

/// Initialize API tracing with a default request-log filter.
pub fn init_tracing() {
    TRACING_INITIALIZED.get_or_init(|| {
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER));

        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_target(false)
            .try_init();
    });
}
