//! Unified LitRadar application composition root.

mod config;
mod openapi;
mod runtime;

use std::error::Error;
use std::path::Path;
use std::time::Instant;

const SERVICE_RUNTIME_WORKER_THREADS: usize = 2;
const PARENT_RUN_ID_ENV: &str = "LITRADAR_PARENT_RUN_ID";

/// Run the application command selected by process arguments.
///
/// # Arguments
///
/// * `args` - Command arguments without the executable name.
///
/// # Returns
///
/// Result indicating whether the selected command completed successfully.
pub fn run(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let application_executable = std::env::current_exe()?;
    let command = command_name(&args);
    let parent_run_id = parent_run_id();
    let process_span = tracing::info_span!(
        "process",
        component = "runtime",
        command,
        version = env!("CARGO_PKG_VERSION"),
        process_id = std::process::id(),
        parent_run_id = tracing::field::Empty,
    );
    if let Some(parent_run_id) = parent_run_id.as_deref() {
        process_span.record("parent_run_id", parent_run_id);
    }
    process_span.in_scope(|| {
        let started_at = Instant::now();
        tracing::info!(event = "process.started", component = "runtime");
        let result = run_with_executable(args, &application_executable);
        let duration_ms = started_at.elapsed().as_millis();
        match &result {
            Ok(()) => tracing::info!(
                event = "process.completed",
                component = "runtime",
                outcome = "success",
                duration_ms,
            ),
            Err(_) => tracing::error!(
                event = "process.failed",
                component = "runtime",
                outcome = "failure",
                error_kind = "command_failed",
                duration_ms,
            ),
        }
        result
    })
}

fn command_name(args: &[String]) -> &'static str {
    match args.first().map(String::as_str) {
        None | Some("--help" | "-h") => "help",
        Some("serve") => "serve",
        Some("admin") => "admin",
        Some("index") => "index",
        Some("notify") => "notify",
        Some("push") => "push",
        Some("scheduler") => "scheduler",
        Some("openapi") => "openapi",
        Some(_) => "unknown",
    }
}

fn parent_run_id() -> Option<String> {
    std::env::var_os(PARENT_RUN_ID_ENV)
        .and_then(|value| value.into_string().ok())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
        })
}

fn run_with_executable(
    args: Vec<String>,
    application_executable: &Path,
) -> Result<(), Box<dyn Error>> {
    let Some((subcommand, subcommand_args)) = args.split_first() else {
        println!("{}", application_usage());
        return Ok(());
    };
    if subcommand == "--help" || subcommand == "-h" {
        println!("{}", application_usage());
        return Ok(());
    }

    match subcommand.as_str() {
        "serve" if has_help(subcommand_args) => {
            println!("{}", config::serve_usage());
            Ok(())
        }
        "serve" => {
            let config = config::ServeConfig::from_args(
                subcommand_args.to_vec(),
                application_executable.to_path_buf(),
            )?;
            run_service(config)
        }
        "admin" => litradar_cli::run_admin_command(subcommand_args.to_vec()),
        "index" => {
            litradar_cli::run_index_command(subcommand_args.to_vec(), application_executable)
        }
        "notify" => litradar_cli::run_notify_command(subcommand_args.to_vec()),
        "push" => litradar_cli::run_push_command(subcommand_args.to_vec()),
        "scheduler" => {
            litradar_cli::run_scheduler_command(subcommand_args.to_vec(), application_executable)
        }
        "openapi" => openapi::run(subcommand_args.to_vec()),
        _ => Err(format!(
            "unknown LitRadar subcommand: {subcommand}\n{}",
            application_usage()
        )
        .into()),
    }
}

fn run_service(config: config::ServeConfig) -> Result<(), Box<dyn Error>> {
    let service_runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(SERVICE_RUNTIME_WORKER_THREADS)
        .thread_name("litradar-service")
        .enable_all()
        .build()?;
    service_runtime.block_on(runtime::run_service(config))
}

/// Return the canonical top-level application usage.
///
/// # Returns
///
/// Help text containing every supported top-level subcommand.
pub fn application_usage() -> &'static str {
    "Usage: litradar <COMMAND> [OPTIONS]\n\nCommands:\n  serve      Run HTTP and scheduling as one service\n  admin      Manage administrators, secrets, and backups\n  index      Build or update searchable article indexes\n  notify     Deliver recommendation notifications\n  push       Push tracking updates\n  scheduler  Validate or run scheduled tasks manually\n  openapi    Emit the generated OpenAPI document"
}

fn has_help(args: &[String]) -> bool {
    args.iter()
        .any(|argument| argument == "--help" || argument == "-h")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::run_with_executable;

    #[test]
    fn unknown_subcommands_fail_without_legacy_dispatch() {
        let error = run_with_executable(vec!["worker".to_string()], Path::new("litradar"))
            .expect_err("removed worker command should fail");

        assert!(error
            .to_string()
            .contains("unknown LitRadar subcommand: worker"));
    }

    #[test]
    fn synchronous_help_dispatch_does_not_require_a_tokio_runtime() {
        assert!(tokio::runtime::Handle::try_current().is_err());

        run_with_executable(
            vec!["index".to_string(), "--help".to_string()],
            Path::new("litradar"),
        )
        .expect("synchronous help should succeed");

        assert!(tokio::runtime::Handle::try_current().is_err());
    }
}
