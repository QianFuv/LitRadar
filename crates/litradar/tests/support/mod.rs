//! Shared process helpers for real-binary integration tests.

use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::tempdir;

/// Build a sanitized command for the compiled LitRadar binary.
///
/// # Returns
///
/// Command with ambient legacy logging and packaged-metadata overrides removed.
pub(crate) fn litradar_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_litradar"));
    command
        .env_remove("LITRADAR_BUNDLED_META_DIR")
        .env_remove("RUST_LOG");
    command
}

/// Run LitRadar in the integration test process working directory.
///
/// # Arguments
///
/// * `args` - Command-line arguments after the executable name.
///
/// # Returns
///
/// Captured process output.
pub(crate) fn run_litradar(args: &[&str]) -> Output {
    run_litradar_with_env(args, &[])
}

/// Run LitRadar with explicit environment overrides.
///
/// # Arguments
///
/// * `args` - Command-line arguments after the executable name.
/// * `environment` - Environment values applied after sanitization.
///
/// # Returns
///
/// Captured process output.
pub(crate) fn run_litradar_with_env(args: &[&str], environment: &[(&str, &str)]) -> Output {
    let root = tempdir().expect("temporary project root should be created");
    let mut command = litradar_command();
    command.current_dir(root.path()).args(args);
    for (name, value) in environment {
        command.env(name, value);
    }
    command.output().expect("litradar binary should run")
}

/// Run LitRadar from an isolated project working directory.
///
/// # Arguments
///
/// * `directory` - Child process working directory.
/// * `args` - Command-line arguments after the executable name.
///
/// # Returns
///
/// Captured process output.
pub(crate) fn run_litradar_in(directory: &Path, args: &[&str]) -> Output {
    litradar_command()
        .current_dir(directory)
        .args(args)
        .output()
        .expect("litradar binary should run")
}

/// Parse structured JSON log lines from process stderr.
///
/// # Arguments
///
/// * `output` - Captured LitRadar process output.
///
/// # Returns
///
/// Parsed log events in emission order.
pub(crate) fn log_events(output: &Output) -> Vec<Value> {
    let stderr = String::from_utf8(output.stderr.clone()).expect("logs should be UTF-8");
    stderr
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("each log line should be JSON"))
        .collect()
}
