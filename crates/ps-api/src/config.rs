//! Runtime configuration for the Rust API server.

use std::env;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use axum::http::{HeaderValue, Uri};

/// Environment variable used by the Python API for the bind host.
pub const API_HOST_ENV: &str = "API_HOST";

/// Environment variable used by the Python API for credentialed CORS origins.
pub const API_CORS_ALLOWED_ORIGINS_ENV: &str = "API_CORS_ALLOWED_ORIGINS";

/// Environment variable used by the Rust API to locate existing data files.
pub const PROJECT_ROOT_ENV: &str = "PAPER_SCANNER_PROJECT_ROOT";

/// Environment variable used by the Rust API for MCP Host validation.
pub const MCP_ALLOWED_HOSTS_ENV: &str = "MCP_ALLOWED_HOSTS";

/// Environment variable used by the Rust API for MCP Origin validation.
pub const MCP_ALLOWED_ORIGINS_ENV: &str = "MCP_ALLOWED_ORIGINS";

const API_PORT_ENV: &str = "API_PORT";
const DEFAULT_MCP_ALLOWED_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

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
    /// Hosts accepted by the Streamable HTTP MCP endpoint.
    pub mcp_allowed_hosts: Vec<String>,
    /// Browser origins accepted by the Streamable HTTP MCP endpoint.
    pub mcp_allowed_origins: Vec<String>,
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
        let mcp_allowed_hosts = parse_mcp_allowed_hosts(&env::var(MCP_ALLOWED_HOSTS_ENV).ok())?;
        let mcp_allowed_origins =
            parse_mcp_allowed_origins(&env::var(MCP_ALLOWED_ORIGINS_ENV).unwrap_or_default())?;

        Ok(Self {
            project_root,
            host,
            port,
            cors_allowed_origins,
            mcp_allowed_hosts,
            mcp_allowed_origins,
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
    /// A configured MCP host is not a valid HTTP header value.
    InvalidMcpAllowedHost(String),
    /// A configured MCP origin is not a valid Origin value.
    InvalidMcpAllowedOrigin(String),
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
            Self::InvalidMcpAllowedHost(value) => {
                write!(formatter, "Invalid MCP allowed host: {value}")
            }
            Self::InvalidMcpAllowedOrigin(value) => {
                write!(formatter, "Invalid MCP allowed origin: {value}")
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

fn parse_mcp_allowed_hosts(value: &Option<String>) -> Result<Vec<String>, ApiConfigError> {
    match value {
        Some(value) => parse_header_value_list(value, ApiConfigError::InvalidMcpAllowedHost),
        None => Ok(DEFAULT_MCP_ALLOWED_HOSTS
            .iter()
            .map(|host| (*host).to_string())
            .collect()),
    }
}

fn parse_mcp_allowed_origins(value: &str) -> Result<Vec<String>, ApiConfigError> {
    let origins = parse_header_value_list(value, ApiConfigError::InvalidMcpAllowedOrigin)?;
    for origin in &origins {
        validate_mcp_origin(origin)?;
    }
    Ok(origins)
}

fn parse_header_value_list(
    value: &str,
    error: impl Fn(String) -> ApiConfigError,
) -> Result<Vec<String>, ApiConfigError> {
    let mut values = Vec::new();
    for entry in value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        HeaderValue::from_str(entry).map_err(|_| error(entry.to_string()))?;
        values.push(entry.to_string());
    }
    Ok(values)
}

fn validate_mcp_origin(origin: &str) -> Result<(), ApiConfigError> {
    if origin == "null" {
        return Ok(());
    }
    let uri = origin
        .parse::<Uri>()
        .map_err(|_| ApiConfigError::InvalidMcpAllowedOrigin(origin.to_string()))?;
    if uri.scheme().is_some() && uri.authority().is_some() {
        Ok(())
    } else {
        Err(ApiConfigError::InvalidMcpAllowedOrigin(origin.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use super::{
        parse_cors_allowed_origins, ApiConfig, ApiConfigError, API_CORS_ALLOWED_ORIGINS_ENV,
        API_HOST_ENV, API_PORT_ENV, MCP_ALLOWED_HOSTS_ENV, MCP_ALLOWED_ORIGINS_ENV,
        PROJECT_ROOT_ENV,
    };

    #[test]
    fn parses_python_style_cors_origin_list() {
        let origins = parse_cors_allowed_origins(" https://a.example,https://b.example ")
            .expect("origins should parse");

        assert_eq!(origins, ["https://a.example", "https://b.example"]);
    }

    #[test]
    fn from_env_uses_defaults_and_builds_bind_address() {
        let _guard = env_guard();
        clear_config_env();
        let project_root = set_project_root_env();

        let config = ApiConfig::from_env().expect("default config should load");

        assert_eq!(config.project_root, project_root);
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 8000);
        assert_eq!(config.bind_address(), "127.0.0.1:8000");
        assert!(config.cors_allowed_origins.is_empty());
        assert_eq!(config.mcp_allowed_hosts, ["localhost", "127.0.0.1", "::1"]);
        assert!(config.mcp_allowed_origins.is_empty());
    }

    #[test]
    fn from_env_reads_python_compatible_overrides() {
        let _guard = env_guard();
        clear_config_env();
        let project_root = set_project_root_env();
        env::set_var(API_HOST_ENV, "0.0.0.0");
        env::set_var(API_PORT_ENV, "9001");
        env::set_var(
            API_CORS_ALLOWED_ORIGINS_ENV,
            "https://paper.example, https://admin.example",
        );
        env::set_var(MCP_ALLOWED_HOSTS_ENV, "paper.example, paper.example:8443");
        env::set_var(
            MCP_ALLOWED_ORIGINS_ENV,
            "https://paper.example, null, http://localhost:5173",
        );

        let config = ApiConfig::from_env().expect("overridden config should load");

        assert_eq!(config.project_root, project_root);
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 9001);
        assert_eq!(config.bind_address(), "0.0.0.0:9001");
        assert_eq!(
            config.cors_allowed_origins,
            ["https://paper.example", "https://admin.example"]
        );
        assert_eq!(
            config.mcp_allowed_hosts,
            ["paper.example", "paper.example:8443"]
        );
        assert_eq!(
            config.mcp_allowed_origins,
            ["https://paper.example", "null", "http://localhost:5173"]
        );
        clear_config_env();
    }

    #[test]
    fn from_env_rejects_invalid_port() {
        let _guard = env_guard();
        clear_config_env();
        set_project_root_env();
        env::set_var(API_PORT_ENV, "not-a-port");

        let error = ApiConfig::from_env().expect_err("invalid port should fail");

        assert!(matches!(&error, ApiConfigError::InvalidPort(value) if value == "not-a-port"));
        assert_eq!(error.to_string(), "Invalid API port: not-a-port");
        clear_config_env();
    }

    #[test]
    fn from_env_rejects_invalid_cors_origin_header_value() {
        let _guard = env_guard();
        clear_config_env();
        set_project_root_env();
        env::set_var(
            API_CORS_ALLOWED_ORIGINS_ENV,
            "https://ok.example,bad\norigin",
        );

        let error = ApiConfig::from_env().expect_err("invalid CORS origin should fail");

        assert!(
            matches!(&error, ApiConfigError::InvalidCorsOrigin(value) if value == "bad\norigin")
        );
        assert_eq!(error.to_string(), "Invalid CORS origin: bad\norigin");
        clear_config_env();
    }

    #[test]
    fn from_env_rejects_invalid_mcp_host_header_value() {
        let _guard = env_guard();
        clear_config_env();
        set_project_root_env();
        env::set_var(MCP_ALLOWED_HOSTS_ENV, "localhost,bad\nhost");

        let error = ApiConfig::from_env().expect_err("invalid MCP host should fail");

        assert!(
            matches!(&error, ApiConfigError::InvalidMcpAllowedHost(value) if value == "bad\nhost")
        );
        assert_eq!(error.to_string(), "Invalid MCP allowed host: bad\nhost");
        clear_config_env();
    }

    #[test]
    fn from_env_rejects_invalid_mcp_origin() {
        let _guard = env_guard();
        clear_config_env();
        set_project_root_env();
        env::set_var(MCP_ALLOWED_ORIGINS_ENV, "https://paper.example,localhost");

        let error = ApiConfig::from_env().expect_err("invalid MCP origin should fail");

        assert!(
            matches!(&error, ApiConfigError::InvalidMcpAllowedOrigin(value) if value == "localhost")
        );
        assert_eq!(error.to_string(), "Invalid MCP allowed origin: localhost");
        clear_config_env();
    }

    fn env_guard() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env lock should be acquired")
    }

    fn set_project_root_env() -> PathBuf {
        let project_root = PathBuf::from("paper-scanner-config-root");
        env::set_var(PROJECT_ROOT_ENV, &project_root);
        project_root
    }

    fn clear_config_env() {
        for name in [
            PROJECT_ROOT_ENV,
            API_HOST_ENV,
            API_PORT_ENV,
            API_CORS_ALLOWED_ORIGINS_ENV,
            MCP_ALLOWED_HOSTS_ENV,
            MCP_ALLOWED_ORIGINS_ENV,
        ] {
            env::remove_var(name);
        }
    }
}
