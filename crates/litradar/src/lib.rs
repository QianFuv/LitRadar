//! Unified LitRadar application composition root.

mod config;
mod openapi;
mod runtime;

use std::error::Error;
use std::path::Path;

/// Run the application command selected by process arguments.
///
/// # Arguments
///
/// * `args` - Command arguments without the executable name.
///
/// # Returns
///
/// Result indicating whether the selected command completed successfully.
pub async fn run(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let application_executable = std::env::current_exe()?;
    run_with_executable(args, &application_executable).await
}

async fn run_with_executable(
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
            runtime::run_service(config).await
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

    #[tokio::test]
    async fn unknown_subcommands_fail_without_legacy_dispatch() {
        let error = run_with_executable(vec!["worker".to_string()], Path::new("litradar"))
            .await
            .expect_err("removed worker command should fail");

        assert!(error
            .to_string()
            .contains("unknown LitRadar subcommand: worker"));
    }
}
