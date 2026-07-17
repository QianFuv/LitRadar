//! Real-binary tests for the unified LitRadar command tree.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::tempdir;

fn run_litradar(args: &[&str]) -> Output {
    run_litradar_with_env(args, &[])
}

fn run_litradar_with_env(args: &[&str], environment: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_litradar"));
    command
        .args(args)
        .env_remove("LITRADAR_LOG_FILTER")
        .env_remove("LITRADAR_LOG_FORMAT")
        .env_remove("RUST_LOG");
    for (name, value) in environment {
        command.env(name, value);
    }
    command.output().expect("litradar binary should run")
}

fn log_events(output: &Output) -> Vec<Value> {
    let stderr = String::from_utf8(output.stderr.clone()).expect("logs should be UTF-8");
    stderr
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("each log line should be JSON"))
        .collect()
}

#[test]
fn help_exposes_exactly_the_unified_command_tree() {
    let output = run_litradar(&["--help"]);
    let stdout = String::from_utf8(output.stdout).expect("help should be UTF-8");
    let commands = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("  "))
        .filter_map(|line| line.split_whitespace().next())
        .collect::<Vec<_>>();

    assert!(output.status.success());
    assert_eq!(
        commands,
        [
            "serve",
            "admin",
            "index",
            "notify",
            "push",
            "scheduler",
            "openapi"
        ]
    );
}

#[test]
fn every_supported_subcommand_has_help_and_worker_is_rejected() {
    for subcommand in [
        "serve",
        "admin",
        "index",
        "notify",
        "push",
        "scheduler",
        "openapi",
    ] {
        let output = run_litradar(&[subcommand, "--help"]);
        assert!(output.status.success(), "{subcommand} help should succeed");
    }

    let removed = run_litradar(&["worker"]);
    assert!(!removed.status.success());
    assert!(log_events(&removed)
        .iter()
        .any(|event| event["event"] == "process.failed"));
}

#[test]
fn openapi_command_emits_and_writes_the_new_health_contract() {
    let stdout = run_litradar(&["openapi"]);
    let document: Value = serde_json::from_slice(&stdout.stdout).expect("OpenAPI should be JSON");
    assert!(stdout.status.success());
    assert!(document["paths"]["/health/live"].is_object());
    assert!(document["paths"]["/health/ready"].is_object());
    assert!(document["paths"]["/api/health"].is_null());

    let root = tempdir().expect("temporary output directory should be created");
    let output_path = root.path().join("openapi.json");
    let written = run_litradar(&[
        "openapi",
        "--output",
        output_path
            .to_str()
            .expect("temporary path should be valid UTF-8"),
    ]);
    assert!(written.status.success());
    assert_eq!(
        std::fs::read(output_path).expect("written document should be readable"),
        stdout.stdout
    );
}

#[test]
fn default_logging_is_json_and_flushes_short_lived_commands() {
    let output = run_litradar(&["--help"]);
    let stderr = String::from_utf8(output.stderr.clone()).expect("logs should be UTF-8");
    let events = log_events(&output);

    assert!(output.status.success());
    assert!(!stderr.contains('\u{1b}'));
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event"], "process.started");
    assert_eq!(events[0]["component"], "runtime");
    assert_eq!(events[0]["span"]["command"], "help");
    assert_eq!(events[1]["event"], "process.completed");
    assert_eq!(events[1]["outcome"], "success");
}

#[test]
fn new_logging_configuration_is_strict_and_ignores_rust_log() {
    let ignored_legacy = run_litradar_with_env(&["--help"], &[("RUST_LOG", "off")]);
    assert!(ignored_legacy.status.success());
    assert_eq!(log_events(&ignored_legacy).len(), 2);

    let invalid = run_litradar_with_env(&["--help"], &[("LITRADAR_LOG_FORMAT", "pretty")]);
    assert!(!invalid.status.success());
    assert_eq!(
        String::from_utf8(invalid.stderr).expect("error should be UTF-8"),
        "invalid LitRadar log format\n"
    );
}

#[test]
fn compact_logging_is_plain_text_and_process_context_omits_raw_arguments() {
    let compact = run_litradar_with_env(&["--help"], &[("LITRADAR_LOG_FORMAT", "compact")]);
    let compact_stderr = String::from_utf8(compact.stderr).expect("logs should be UTF-8");
    assert!(compact.status.success());
    assert!(compact_stderr.contains("process.started"));
    assert!(compact_stderr.contains("process.completed"));
    assert!(!compact_stderr.contains('\u{1b}'));

    let sentinel = "credential-sentinel-that-must-not-appear";
    let failed = run_litradar(&[sentinel]);
    let stderr = String::from_utf8(failed.stderr.clone()).expect("logs should be UTF-8");
    assert!(!failed.status.success());
    assert!(!stderr.contains(sentinel));
    assert!(log_events(&failed)
        .iter()
        .any(|event| event["span"]["command"] == "unknown"));
}

#[test]
fn cli_phase_events_preserve_stdout_and_do_not_duplicate_process_failures() {
    let help = run_litradar(&["admin", "--help"]);
    let help_stdout: Value =
        serde_json::from_slice(&help.stdout).expect("admin help should remain JSON");
    let help_events = log_events(&help);

    assert!(help.status.success());
    assert!(help_stdout["usage"][0]
        .as_str()
        .expect("admin usage should contain text")
        .starts_with("litradar admin"));
    assert_eq!(
        help_events
            .iter()
            .filter(|event| event["event"] == "cli.command.started")
            .count(),
        1
    );
    assert_eq!(
        help_events
            .iter()
            .filter(|event| event["event"] == "cli.command.completed")
            .count(),
        1
    );
    assert!(help_events
        .iter()
        .filter(|event| event["event"] == "cli.command.started")
        .all(|event| event["command"] == "admin"));

    let sentinel = "cli-private-argument-sentinel";
    let failed = run_litradar(&["index", sentinel]);
    let failed_stderr = String::from_utf8(failed.stderr.clone()).expect("logs should be UTF-8");
    let failed_events = log_events(&failed);

    assert!(!failed.status.success());
    assert!(failed.stdout.is_empty());
    assert!(!failed_stderr.contains(sentinel));
    assert_eq!(
        failed_events
            .iter()
            .filter(|event| event["event"] == "cli.command.failed")
            .count(),
        1
    );
    assert_eq!(
        failed_events
            .iter()
            .filter(|event| event["event"] == "process.failed")
            .count(),
        1
    );
}

#[test]
fn direct_output_macros_match_the_explicit_source_allowlist() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root should resolve");
    let mut source_files = Vec::new();
    for crate_entry in fs::read_dir(workspace.join("crates")).expect("crates should be readable") {
        let crate_path = crate_entry.expect("crate entry should load").path();
        let source_path = crate_path.join("src");
        if source_path.is_dir() {
            collect_rust_sources(&source_path, &mut source_files);
        }
    }

    let mut observed = BTreeMap::new();
    for source_file in source_files {
        let source = fs::read_to_string(&source_file).expect("Rust source should be readable");
        let counts = [
            macro_count(&source, "print"),
            macro_count(&source, "println"),
            macro_count(&source, "eprint"),
            macro_count(&source, "eprintln"),
        ];
        if counts != [0, 0, 0, 0] {
            let relative = source_file
                .strip_prefix(&workspace)
                .expect("source should be inside workspace")
                .to_string_lossy()
                .replace('\\', "/");
            observed.insert(relative, counts);
        }
    }

    let expected = BTreeMap::from([
        ("crates/litradar-cli/src/lib.rs".to_string(), [0, 2, 0, 0]),
        ("crates/litradar/src/lib.rs".to_string(), [0, 3, 0, 0]),
        ("crates/litradar/src/main.rs".to_string(), [0, 0, 0, 1]),
        (
            "crates/litradar/src/observability.rs".to_string(),
            [0, 0, 0, 3],
        ),
        ("crates/litradar/src/openapi.rs".to_string(), [1, 1, 0, 0]),
    ]);
    assert_eq!(observed, expected);
}

fn collect_rust_sources(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("source directory should be readable") {
        let path = entry.expect("source entry should load").path();
        if path.is_dir() {
            collect_rust_sources(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn macro_count(source: &str, macro_name: &str) -> usize {
    let pattern = format!("{macro_name}!");
    source
        .match_indices(&pattern)
        .filter(|(index, _)| {
            *index == 0
                || !source.as_bytes()[index - 1].is_ascii_alphanumeric()
                    && source.as_bytes()[index - 1] != b'_'
        })
        .count()
}
