//! Runtime configuration for the Rust API server.

use std::env;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use axum::http::HeaderValue;

/// Environment variable used by the Python API for the bind host.
pub const API_HOST_ENV: &str = "API_HOST";

/// Environment variable used by the Python API for credentialed CORS origins.
pub const API_CORS_ALLOWED_ORIGINS_ENV: &str = "API_CORS_ALLOWED_ORIGINS";

/// Environment variable used by the Rust API to locate existing data files.
pub const PROJECT_ROOT_ENV: &str = "PAPER_SCANNER_PROJECT_ROOT";

const API_PORT_ENV: &str = "API_PORT";

/// Rust API runtime configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiConfig {
    /// Project or deployment root used to resolve data paths.
    pub project_root: PathBuf,
    /// Hostname or IP address to bind.
    pub host: String,
    /// TCP port to bind.
    pub port: u16,
    /// Credentialed CORS origins configured through the Python-compatible env var.
    pub cors_allowed_origins: Vec<String>,
}

impl ApiConfig {
    /// Load API configuration from process environment variables.
    ///
    /// # Returns
    ///
    /// Runtime API configuration.
    pub fn from_env() -> Result<Self, ApiConfigError> {
        let project_root = match env::var(PROJECT_ROOT_ENV) {
            Ok(value) if !value.trim().is_empty() => PathBuf::from(value),
            _ => env::current_dir().map_err(ApiConfigError::CurrentDir)?,
        };
        let host = env::var(API_HOST_ENV).unwrap_or_else(|_| "127.0.0.1".to_string());
        let port = match env::var(API_PORT_ENV) {
            Ok(value) if !value.trim().is_empty() => value
                .parse::<u16>()
                .map_err(|_| ApiConfigError::InvalidPort(value))?,
            _ => 8000,
        };
        let cors_allowed_origins = parse_cors_allowed_origins(
            &env::var(API_CORS_ALLOWED_ORIGINS_ENV).unwrap_or_default(),
        )?;

        Ok(Self {
            project_root,
            host,
            port,
            cors_allowed_origins,
        })
    }

    /// Return a host:port bind address.
    ///
    /// # Returns
    ///
    /// Bind address string suitable for Tokio TCP binding.
    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Configuration loading error.
#[derive(Debug)]
pub enum ApiConfigError {
    /// Current directory resolution failed.
    CurrentDir(std::io::Error),
    /// The configured port is not a valid unsigned 16-bit integer.
    InvalidPort(String),
    /// A configured CORS origin is not a valid HTTP header value.
    InvalidCorsOrigin(String),
}

impl fmt::Display for ApiConfigError {
    /// Format the configuration error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CurrentDir(error) => write!(formatter, "{error}"),
            Self::InvalidPort(value) => write!(formatter, "Invalid API port: {value}"),
            Self::InvalidCorsOrigin(value) => {
                write!(formatter, "Invalid CORS origin: {value}")
            }
        }
    }
}

impl Error for ApiConfigError {
    /// Return the underlying source error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CurrentDir(error) => Some(error),
            _ => None,
        }
    }
}

fn parse_cors_allowed_origins(value: &str) -> Result<Vec<String>, ApiConfigError> {
    let mut origins = Vec::new();
    for origin in value
        .split(',')
        .map(str::trim)
        .filter(|origin| !origin.is_empty())
    {
        HeaderValue::from_str(origin)
            .map_err(|_| ApiConfigError::InvalidCorsOrigin(origin.to_string()))?;
        origins.push(origin.to_string());
    }
    Ok(origins)
}

#[cfg(test)]
mod tests {
    use super::parse_cors_allowed_origins;

    #[test]
    fn parses_python_style_cors_origin_list() {
        let origins = parse_cors_allowed_origins(" https://a.example,https://b.example ")
            .expect("origins should parse");

        assert_eq!(origins, ["https://a.example", "https://b.example"]);
    }
}
