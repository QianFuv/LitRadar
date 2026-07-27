//! Runtime configuration for the Rust API server.

use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use litradar_domain::{RuntimeSettingValue, RuntimeSettingsUpdate};
use litradar_storage::{
    parse_runtime_setting, runtime_setting_default, ParsedRuntimeSettingValue, RuntimeSettingKey,
};

/// Rust API runtime configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiConfig {
    /// Project or deployment root used to resolve data paths.
    pub project_root: PathBuf,
    /// Optional immutable metadata bundle supplied by a packaged runtime.
    pub bundled_meta_dir: Option<PathBuf>,
    /// Hostname or IP address to bind.
    pub host: String,
    /// TCP port to bind.
    pub port: u16,
    /// Raw 32-byte deployment secret key file.
    pub secret_key_file: PathBuf,
    /// Credentialed CORS origins configured through admin runtime settings.
    pub cors_allowed_origins: Vec<String>,
    /// Hosts accepted by the Streamable HTTP MCP endpoint.
    pub mcp_allowed_hosts: Vec<String>,
    /// Browser origins accepted by the Streamable HTTP MCP endpoint.
    pub mcp_allowed_origins: Vec<String>,
    /// Whether browser session cookies include the Secure attribute.
    pub are_session_cookies_secure: bool,
    /// Whether startup must fail unless secure session cookies are enabled.
    pub are_secure_cookies_required: bool,
}

impl ApiConfig {
    /// Build API configuration from explicit launch values.
    ///
    /// # Arguments
    ///
    /// * `project_root` - Project or deployment root used to resolve data paths.
    /// * `host` - Bind host.
    /// * `port` - Bind port.
    /// * `secret_key_file` - Raw 32-byte deployment secret key file.
    ///
    /// # Returns
    ///
    /// Runtime API configuration.
    pub fn new(project_root: PathBuf, host: String, port: u16, secret_key_file: PathBuf) -> Self {
        Self {
            project_root,
            bundled_meta_dir: None,
            host,
            port,
            secret_key_file,
            cors_allowed_origins: default_runtime_string_list(
                RuntimeSettingKey::CorsAllowedOrigins,
            ),
            mcp_allowed_hosts: default_runtime_string_list(RuntimeSettingKey::McpAllowedHosts),
            mcp_allowed_origins: default_runtime_string_list(RuntimeSettingKey::McpAllowedOrigins),
            are_session_cookies_secure: default_runtime_boolean(RuntimeSettingKey::SecureCookies),
            are_secure_cookies_required: false,
        }
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
        settings: &[RuntimeSettingValue],
    ) -> Result<(), ApiConfigError> {
        for setting in settings {
            let Some(key) = RuntimeSettingKey::from_field(&setting.field) else {
                continue;
            };
            let parsed = parse_runtime_setting(key, &setting.value)
                .map_err(|error| ApiConfigError::InvalidRuntimeSetting(error.to_string()))?;
            match (key, parsed) {
                (
                    RuntimeSettingKey::CorsAllowedOrigins,
                    ParsedRuntimeSettingValue::StringList(values),
                ) => self.cors_allowed_origins = values,
                (
                    RuntimeSettingKey::McpAllowedHosts,
                    ParsedRuntimeSettingValue::StringList(values),
                ) => self.mcp_allowed_hosts = values,
                (
                    RuntimeSettingKey::McpAllowedOrigins,
                    ParsedRuntimeSettingValue::StringList(values),
                ) => self.mcp_allowed_origins = values,
                (RuntimeSettingKey::SecureCookies, ParsedRuntimeSettingValue::Boolean(value)) => {
                    self.are_session_cookies_secure = value
                }
                _ => {}
            }
        }
        if self.are_secure_cookies_required && !self.are_session_cookies_secure {
            return Err(ApiConfigError::SecureCookiesRequired);
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
pub enum ApiConfigError {
    /// A managed runtime setting failed its registry parser.
    InvalidRuntimeSetting(String),
    /// Production startup requires secure session cookies.
    SecureCookiesRequired,
}

impl fmt::Debug for ApiConfigError {
    /// Format configuration failures as user-facing non-secret diagnostics.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for ApiConfigError {
    /// Format the configuration error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRuntimeSetting(detail) => formatter.write_str(detail),
            Self::SecureCookiesRequired => formatter.write_str(
                "Secure session cookies are required; set secure_cookies to true before startup",
            ),
        }
    }
}

impl Error for ApiConfigError {}

/// Validate changed runtime settings that share startup parsing rules.
///
/// # Arguments
///
/// * `update` - Runtime settings update submitted by an authenticated administrator.
///
/// # Returns
///
/// Result indicating whether every changed field uses its startup grammar.
pub(crate) fn validate_runtime_settings_update(
    update: &RuntimeSettingsUpdate,
) -> Result<(), ApiConfigError> {
    for (field, value) in &update.values {
        let Some(value) = value else {
            continue;
        };
        let Some(key) = RuntimeSettingKey::from_field(field) else {
            continue;
        };
        if let Err(error) = parse_runtime_setting(key, value) {
            return Err(ApiConfigError::InvalidRuntimeSetting(error.to_string()));
        }
    }
    Ok(())
}

fn default_runtime_string_list(key: RuntimeSettingKey) -> Vec<String> {
    match parse_runtime_setting(key, runtime_setting_default(key))
        .expect("runtime setting defaults must pass their registry parser")
    {
        ParsedRuntimeSettingValue::StringList(values) => values,
        _ => panic!(
            "runtime setting {} must use a string-list parser",
            key.as_str()
        ),
    }
}

fn default_runtime_boolean(key: RuntimeSettingKey) -> bool {
    match parse_runtime_setting(key, runtime_setting_default(key))
        .expect("runtime setting defaults must pass their registry parser")
    {
        ParsedRuntimeSettingValue::Boolean(value) => value,
        _ => panic!("runtime setting {} must use a boolean parser", key.as_str()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use litradar_domain::{RuntimeSettingValue, RuntimeSettingsUpdate};

    use super::{validate_runtime_settings_update, ApiConfig, ApiConfigError};

    #[test]
    fn new_uses_defaults_and_builds_bind_address() {
        let project_root = PathBuf::from("litradar-config-root");

        let config = ApiConfig::new(
            project_root.clone(),
            "127.0.0.1".to_string(),
            8000,
            PathBuf::from("secret.key"),
        );

        assert_eq!(config.project_root, project_root);
        assert_eq!(config.bundled_meta_dir, None);
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 8000);
        assert_eq!(config.bind_address(), "127.0.0.1:8000");
        assert!(config.cors_allowed_origins.is_empty());
        assert_eq!(config.mcp_allowed_hosts, ["localhost", "127.0.0.1", "::1"]);
        assert!(config.mcp_allowed_origins.is_empty());
        assert!(!config.are_session_cookies_secure);
        assert!(!config.are_secure_cookies_required);
    }

    #[test]
    fn runtime_settings_apply_admin_values() {
        let mut config = ApiConfig::new(
            PathBuf::from("litradar-config-root"),
            "127.0.0.1".to_string(),
            8000,
            PathBuf::from("secret.key"),
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
    fn production_flag_requires_secure_cookie_runtime_setting() {
        let mut config = ApiConfig::new(
            PathBuf::from("fixture-root"),
            "127.0.0.1".to_string(),
            8000,
            PathBuf::from("secret.key"),
        );
        config.are_secure_cookies_required = true;

        let error = config
            .apply_runtime_settings(&[runtime_setting("secure_cookies", "false")])
            .expect_err("insecure cookies should fail closed");
        assert!(matches!(error, ApiConfigError::SecureCookiesRequired));

        config
            .apply_runtime_settings(&[runtime_setting("secure_cookies", "true")])
            .expect("secure cookies should satisfy the production gate");
    }

    #[test]
    fn runtime_settings_reject_invalid_cors_origin_header_value() {
        let mut config = ApiConfig::new(
            PathBuf::from("litradar-config-root"),
            "127.0.0.1".to_string(),
            8000,
            PathBuf::from("secret.key"),
        );

        let error = config
            .apply_runtime_settings(&[runtime_setting(
                "cors_allowed_origins",
                "https://ok.example,bad\norigin",
            )])
            .expect_err("invalid CORS origin should fail");

        assert!(matches!(
            &error,
            ApiConfigError::InvalidRuntimeSetting(detail)
                if detail == "Invalid CORS origin: bad\norigin"
        ));
        assert_eq!(error.to_string(), "Invalid CORS origin: bad\norigin");
    }

    #[test]
    fn runtime_settings_reject_invalid_mcp_host_header_value() {
        let mut config = ApiConfig::new(
            PathBuf::from("litradar-config-root"),
            "127.0.0.1".to_string(),
            8000,
            PathBuf::from("secret.key"),
        );

        let error = config
            .apply_runtime_settings(&[runtime_setting("mcp_allowed_hosts", "localhost,bad\nhost")])
            .expect_err("invalid MCP host should fail");

        assert!(matches!(
            &error,
            ApiConfigError::InvalidRuntimeSetting(detail)
                if detail == "Invalid MCP allowed host: bad\nhost"
        ));
        assert_eq!(error.to_string(), "Invalid MCP allowed host: bad\nhost");
    }

    #[test]
    fn runtime_settings_reject_invalid_mcp_origin() {
        let mut config = ApiConfig::new(
            PathBuf::from("litradar-config-root"),
            "127.0.0.1".to_string(),
            8000,
            PathBuf::from("secret.key"),
        );

        let error = config
            .apply_runtime_settings(&[runtime_setting(
                "mcp_allowed_origins",
                "https://paper.example,localhost",
            )])
            .expect_err("invalid MCP origin should fail");

        assert!(matches!(
            &error,
            ApiConfigError::InvalidRuntimeSetting(detail)
                if detail == "Invalid MCP allowed origin: localhost"
        ));
        assert_eq!(error.to_string(), "Invalid MCP allowed origin: localhost");
    }

    #[test]
    fn runtime_logging_updates_use_strict_startup_grammar() {
        let update = |field: &str, value: &str| RuntimeSettingsUpdate {
            values: HashMap::from([(field.to_string(), Some(value.to_string()))]),
            secret_pool_updates: HashMap::new(),
        };

        validate_runtime_settings_update(&update("log_format", "compact"))
            .expect("compact logging should be valid");
        validate_runtime_settings_update(&update(
            "log_filter",
            "warn,litradar=debug,litradar_api::routes=trace",
        ))
        .expect("strict filter directives should be valid");
        assert!(matches!(
            validate_runtime_settings_update(&update("log_format", "pretty")),
            Err(ApiConfigError::InvalidRuntimeSetting(detail))
                if detail == "Invalid LitRadar log format"
        ));
        assert!(matches!(
            validate_runtime_settings_update(&update("log_format", " compact ")),
            Err(ApiConfigError::InvalidRuntimeSetting(detail))
                if detail == "Invalid LitRadar log format"
        ));
        assert!(matches!(
            validate_runtime_settings_update(&update("log_filter", "[")),
            Err(ApiConfigError::InvalidRuntimeSetting(detail))
                if detail == "Invalid LitRadar log filter"
        ));
    }

    fn runtime_setting(field: &str, value: &str) -> RuntimeSettingValue {
        RuntimeSettingValue {
            field: field.to_string(),
            value: value.to_string(),
            source: "database".to_string(),
            updated_at: Some(1.0),
        }
    }
}
