//! Unified service runtime configuration and argument parsing.

use std::error::Error;
use std::path::PathBuf;
use std::time::Duration;

use litradar_api::config::ApiConfig;
use litradar_storage::StorageConfig;

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 8000;
const DEFAULT_SCHEDULER_INTERVAL_SECONDS: u64 = 30;

/// Parsed configuration for the unified long-running service.
#[derive(Debug, Clone)]
pub(crate) struct ServeConfig {
    /// Prepared HTTP adapter configuration.
    pub(crate) api_config: ApiConfig,
    /// Canonical executable used for same-application subprocesses.
    pub(crate) application_executable: PathBuf,
    /// Auth database shared by HTTP and scheduling.
    pub(crate) auth_db_path: PathBuf,
    /// Delay between immediate-first scheduler ticks.
    pub(crate) scheduler_interval: Duration,
}

impl ServeConfig {
    /// Parse explicit `litradar serve` arguments.
    ///
    /// # Arguments
    ///
    /// * `args` - Serve arguments without the application or subcommand names.
    /// * `application_executable` - Canonical executable used for child processes.
    ///
    /// # Returns
    ///
    /// Validated service configuration.
    pub(crate) fn from_args(
        mut args: Vec<String>,
        application_executable: PathBuf,
    ) -> Result<Self, Box<dyn Error>> {
        let host =
            extract_string_option(&mut args, "--host")?.unwrap_or_else(|| DEFAULT_HOST.to_string());
        let port = extract_string_option(&mut args, "--port")?
            .map(|value| {
                value
                    .parse::<u16>()
                    .map_err(|_| format!("invalid serve port: {value}"))
            })
            .transpose()?
            .unwrap_or(DEFAULT_PORT);
        let project_root = extract_string_option(&mut args, "--project-root")?
            .map(PathBuf::from)
            .map_or_else(std::env::current_dir, Ok)?;
        let secret_key_file = extract_string_option(&mut args, "--secret-key-file")?
            .map(PathBuf::from)
            .ok_or("--secret-key-file is required")?;
        let interval_seconds = extract_string_option(&mut args, "--scheduler-interval-seconds")?
            .map(|value| {
                value
                    .parse::<u64>()
                    .map_err(|_| format!("invalid scheduler interval: {value}"))
            })
            .transpose()?
            .unwrap_or(DEFAULT_SCHEDULER_INTERVAL_SECONDS);
        if interval_seconds == 0 {
            return Err("--scheduler-interval-seconds must be greater than zero".into());
        }
        let are_secure_cookies_required = remove_flag(&mut args, "--require-secure-cookies");
        if !args.is_empty() {
            return Err(format!("unexpected serve arguments: {}", args.join(" ")).into());
        }

        let storage_config = StorageConfig::from_project_root(&project_root);
        let mut api_config = ApiConfig::new(project_root, host, port, secret_key_file);
        api_config.are_secure_cookies_required = are_secure_cookies_required;
        Ok(Self {
            api_config,
            application_executable,
            auth_db_path: storage_config.auth_db_path().to_path_buf(),
            scheduler_interval: Duration::from_secs(interval_seconds),
        })
    }
}

/// Return the canonical `serve` command usage.
///
/// # Returns
///
/// Usage text for service runtime options.
pub(crate) fn serve_usage() -> &'static str {
    "Usage: litradar serve --secret-key-file PATH [--host HOST] [--port PORT] [--project-root PATH] [--scheduler-interval-seconds N] [--require-secure-cookies]"
}

fn extract_string_option(
    args: &mut Vec<String>,
    name: &str,
) -> Result<Option<String>, Box<dyn Error>> {
    let Some(index) = args.iter().position(|argument| argument == name) else {
        return Ok(None);
    };
    if index + 1 >= args.len() {
        return Err(format!("{name} requires a value").into());
    }
    let value = args.remove(index + 1);
    args.remove(index);
    Ok(Some(value))
}

fn remove_flag(args: &mut Vec<String>, name: &str) -> bool {
    args.iter()
        .position(|argument| argument == name)
        .map(|index| args.remove(index))
        .is_some()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::ServeConfig;

    #[test]
    fn parses_explicit_unified_service_configuration() {
        let config = ServeConfig::from_args(
            vec![
                "--host".to_string(),
                "0.0.0.0".to_string(),
                "--port".to_string(),
                "9001".to_string(),
                "--project-root".to_string(),
                "fixture-root".to_string(),
                "--secret-key-file".to_string(),
                "secret.key".to_string(),
                "--scheduler-interval-seconds".to_string(),
                "5".to_string(),
                "--require-secure-cookies".to_string(),
            ],
            PathBuf::from("litradar"),
        )
        .expect("serve configuration should parse");

        assert_eq!(config.api_config.bind_address(), "0.0.0.0:9001");
        assert!(config.api_config.are_secure_cookies_required);
        assert_eq!(config.application_executable, PathBuf::from("litradar"));
        assert_eq!(config.scheduler_interval, Duration::from_secs(5));
    }

    #[test]
    fn rejects_zero_scheduler_interval() {
        let error = ServeConfig::from_args(
            vec![
                "--secret-key-file".to_string(),
                "secret.key".to_string(),
                "--scheduler-interval-seconds".to_string(),
                "0".to_string(),
            ],
            PathBuf::from("litradar"),
        )
        .expect_err("zero interval should fail");

        assert!(error.to_string().contains("must be greater than zero"));
    }
}
