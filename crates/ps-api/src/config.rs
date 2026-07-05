//! Runtime configuration for the Rust API server.

use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use axum::http::{HeaderValue, Uri};
use ps_domain::RuntimeSettingInfo;

const DEFAULT_MCP_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// Rust API runtime configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiConfig {
    /// Project or deployment root used to resolve data paths.
    pub project_root: PathBuf,
    /// Hostname or IP address to bind.
    pub host: String,
    /// TCP port to bind.
    pub port: u16,
    /// Credentialed CORS origins configured through admin runtime settings.
    pub cors_allowed_origins: Vec<String>,
    /// Hosts accepted by the Streamable HTTP MCP endpoint.
    pub mcp_allowed_hosts: Vec<String>,
    /// Browser origins accepted by the Streamable HTTP MCP endpoint.
    pub mcp_allowed_origins: Vec<String>,
    /// Whether browser session cookies include the Secure attribute.
    pub are_session_cookies_secure: bool,
}

impl ApiConfig {
    /// Build API configuration from explicit launch values.
    ///
    /// # Arguments
    ///
    /// * `project_root` - Project or deployment root used to resolve data paths.
    /// * `host` - Bind host.
    /// * `port` - Bind port.
    ///
    /// # Returns
    ///
    /// Runtime API configuration.
    pub fn new(project_root: PathBuf, host: String, port: u16) -> Self {
        Self {
            project_root,
            host,
            port,
            cors_allowed_origins: Vec::new(),
            mcp_allowed_hosts: default_mcp_allowed_hosts(),
            mcp_allowed_origins: Vec::new(),
            are_session_cookies_secure: false,
        }
    }

    /// Build API configuration from explicit CLI arguments.
    ///
    /// # Arguments
    ///
    /// * `args` - Command arguments without the executable name.
    ///
    /// # Returns
    ///
    /// Runtime API configuration.
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Result<Self, ApiConfigError> {
        let mut args = args.into_iter().collect::<Vec<_>>();
        let host =
            extract_string_option(&mut args, "--host")?.unwrap_or_else(|| "127.0.0.1".to_string());
        let port = match extract_string_option(&mut args, "--port")? {
            Some(value) => value
                .parse::<u16>()
                .map_err(|_| ApiConfigError::InvalidPort(value))?,
            None => 8000,
        };
        let project_root = match extract_string_option(&mut args, "--project-root")? {
            Some(value) => PathBuf::from(value),
            None => std::env::current_dir().map_err(ApiConfigError::CurrentDir)?,
        };
        if let Some(argument) = args.first() {
            return Err(ApiConfigError::UnexpectedArgument(argument.clone()));
        }
        Ok(Self::new(project_root, host, port))
    }

    /// Apply database-backed admin runtime settings.
    ///
    /// # Arguments
    ///
    /// * `settings` - Managed runtime settings loaded from the auth database.
    ///
    /// # Returns
    ///
    /// Result indicating whether all configured values were valid.
    pub fn apply_runtime_settings(
        &mut self,
        settings: &[RuntimeSettingInfo],
    ) -> Result<(), ApiConfigError> {
        for setting in settings {
            match setting.field.as_str() {
                "cors_allowed_origins" => {
                    self.cors_allowed_origins = parse_cors_allowed_origins(&setting.value)?;
                }
                "mcp_allowed_hosts" => {
                    self.mcp_allowed_hosts = parse_mcp_allowed_hosts(&setting.value)?;
                }
                "mcp_allowed_origins" => {
                    self.mcp_allowed_origins = parse_mcp_allowed_origins(&setting.value)?;
                }
                "secure_cookies" => {
                    self.are_session_cookies_secure =
                        parse_runtime_bool(&setting.field, &setting.value)?;
                }
                _ => {}
            }
        }
        Ok(())
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
    /// A command argument requires a following value.
    MissingArgumentValue(String),
    /// A command argument is not supported.
    UnexpectedArgument(String),
    /// A configured CORS origin is not a valid HTTP header value.
    InvalidCorsOrigin(String),
    /// A configured MCP host is not a valid HTTP header value.
    InvalidMcpAllowedHost(String),
    /// A configured MCP origin is not a valid Origin value.
    InvalidMcpAllowedOrigin(String),
    /// A configured boolean runtime setting is not valid.
    InvalidRuntimeBoolean { field: String, value: String },
}

impl fmt::Display for ApiConfigError {
    /// Format the configuration error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CurrentDir(error) => write!(formatter, "{error}"),
            Self::InvalidPort(value) => write!(formatter, "Invalid API port: {value}"),
            Self::MissingArgumentValue(name) => write!(formatter, "{name} requires a value"),
            Self::UnexpectedArgument(argument) => {
                write!(formatter, "Unexpected API argument: {argument}")
            }
            Self::InvalidCorsOrigin(value) => {
                write!(formatter, "Invalid CORS origin: {value}")
            }
            Self::InvalidMcpAllowedHost(value) => {
                write!(formatter, "Invalid MCP allowed host: {value}")
            }
            Self::InvalidMcpAllowedOrigin(value) => {
                write!(formatter, "Invalid MCP allowed origin: {value}")
            }
            Self::InvalidRuntimeBoolean { field, value } => {
                write!(
                    formatter,
                    "Invalid boolean runtime setting {field}: {value}"
                )
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

/// Return the API command usage text.
///
/// # Returns
///
/// Usage string for the standalone `api` command.
pub fn api_usage() -> &'static str {
    "api [--host HOST] [--port PORT] [--project-root PATH]"
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

fn parse_mcp_allowed_hosts(value: &str) -> Result<Vec<String>, ApiConfigError> {
    parse_header_value_list(value, ApiConfigError::InvalidMcpAllowedHost)
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

fn default_mcp_allowed_hosts() -> Vec<String> {
    DEFAULT_MCP_HOSTS
        .iter()
        .map(|host| (*host).to_string())
        .collect()
}

fn parse_runtime_bool(field: &str, value: &str) -> Result<bool, ApiConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" | "" => Ok(false),
        _ => Err(ApiConfigError::InvalidRuntimeBoolean {
            field: field.to_string(),
            value: value.to_string(),
        }),
    }
}

fn extract_string_option(
    args: &mut Vec<String>,
    name: &str,
) -> Result<Option<String>, ApiConfigError> {
    if let Some(index) = args.iter().position(|argument| argument == name) {
        if index + 1 >= args.len() {
            return Err(ApiConfigError::MissingArgumentValue(name.to_string()));
        }
        let value = args.remove(index + 1);
        args.remove(index);
        return Ok(Some(value));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ps_domain::RuntimeSettingInfo;

    use super::{parse_cors_allowed_origins, ApiConfig, ApiConfigError};

    #[test]
    fn parses_python_style_cors_origin_list() {
        let origins = parse_cors_allowed_origins(" https://a.example,https://b.example ")
            .expect("origins should parse");

        assert_eq!(origins, ["https://a.example", "https://b.example"]);
    }

    #[test]
    fn new_uses_defaults_and_builds_bind_address() {
        let project_root = PathBuf::from("paper-scanner-config-root");

        let config = ApiConfig::new(project_root.clone(), "127.0.0.1".to_string(), 8000);

        assert_eq!(config.project_root, project_root);
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 8000);
        assert_eq!(config.bind_address(), "127.0.0.1:8000");
        assert!(config.cors_allowed_origins.is_empty());
        assert_eq!(config.mcp_allowed_hosts, ["localhost", "127.0.0.1", "::1"]);
        assert!(config.mcp_allowed_origins.is_empty());
        assert!(!config.are_session_cookies_secure);
    }

    #[test]
    fn from_args_reads_explicit_process_arguments() {
        let project_root = PathBuf::from("paper-scanner-config-root");

        let config = ApiConfig::from_args([
            "--host".to_string(),
            "0.0.0.0".to_string(),
            "--port".to_string(),
            "9001".to_string(),
            "--project-root".to_string(),
            project_root.display().to_string(),
        ])
        .expect("explicit config should load");

        assert_eq!(config.project_root, project_root);
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 9001);
        assert_eq!(config.bind_address(), "0.0.0.0:9001");
    }

    #[test]
    fn runtime_settings_apply_admin_values() {
        let mut config = ApiConfig::new(
            PathBuf::from("paper-scanner-config-root"),
            "127.0.0.1".to_string(),
            8000,
        );

        config
            .apply_runtime_settings(&[
                runtime_setting(
                    "cors_allowed_origins",
                    "https://paper.example, https://admin.example",
                ),
                runtime_setting("mcp_allowed_hosts", "paper.example, paper.example:8443"),
                runtime_setting(
                    "mcp_allowed_origins",
                    "https://paper.example, null, http://localhost:5173",
                ),
                runtime_setting("secure_cookies", "true"),
            ])
            .expect("runtime settings should apply");

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
        assert!(config.are_session_cookies_secure);
    }

    #[test]
    fn from_args_rejects_invalid_port() {
        let error = ApiConfig::from_args(["--port".to_string(), "not-a-port".to_string()])
            .expect_err("invalid port should fail");

        assert!(matches!(&error, ApiConfigError::InvalidPort(value) if value == "not-a-port"));
        assert_eq!(error.to_string(), "Invalid API port: not-a-port");
    }

    #[test]
    fn runtime_settings_reject_invalid_cors_origin_header_value() {
        let mut config = ApiConfig::new(
            PathBuf::from("paper-scanner-config-root"),
            "127.0.0.1".to_string(),
            8000,
        );

        let error = config
            .apply_runtime_settings(&[runtime_setting(
                "cors_allowed_origins",
                "https://ok.example,bad\norigin",
            )])
            .expect_err("invalid CORS origin should fail");

        assert!(
            matches!(&error, ApiConfigError::InvalidCorsOrigin(value) if value == "bad\norigin")
        );
        assert_eq!(error.to_string(), "Invalid CORS origin: bad\norigin");
    }

    #[test]
    fn runtime_settings_reject_invalid_mcp_host_header_value() {
        let mut config = ApiConfig::new(
            PathBuf::from("paper-scanner-config-root"),
            "127.0.0.1".to_string(),
            8000,
        );

        let error = config
            .apply_runtime_settings(&[runtime_setting("mcp_allowed_hosts", "localhost,bad\nhost")])
            .expect_err("invalid MCP host should fail");

        assert!(
            matches!(&error, ApiConfigError::InvalidMcpAllowedHost(value) if value == "bad\nhost")
        );
        assert_eq!(error.to_string(), "Invalid MCP allowed host: bad\nhost");
    }

    #[test]
    fn runtime_settings_reject_invalid_mcp_origin() {
        let mut config = ApiConfig::new(
            PathBuf::from("paper-scanner-config-root"),
            "127.0.0.1".to_string(),
            8000,
        );

        let error = config
            .apply_runtime_settings(&[runtime_setting(
                "mcp_allowed_origins",
                "https://paper.example,localhost",
            )])
            .expect_err("invalid MCP origin should fail");

        assert!(
            matches!(&error, ApiConfigError::InvalidMcpAllowedOrigin(value) if value == "localhost")
        );
        assert_eq!(error.to_string(), "Invalid MCP allowed origin: localhost");
    }

    fn runtime_setting(field: &str, value: &str) -> RuntimeSettingInfo {
        RuntimeSettingInfo {
            field: field.to_string(),
            label: field.to_string(),
            description: String::new(),
            input_type: "text".to_string(),
            is_secret: false,
            value: value.to_string(),
            source: "database".to_string(),
            updated_at: Some(1.0),
        }
    }
}
