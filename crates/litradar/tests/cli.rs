//! Real-binary tests for the unified LitRadar command tree.

use std::process::{Command, Output};

use serde_json::Value;
use tempfile::tempdir;

fn run_litradar(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_litradar"))
        .args(args)
        .output()
        .expect("litradar binary should run")
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
    let stderr = String::from_utf8(removed.stderr).expect("error should be UTF-8");
    assert!(!removed.status.success());
    assert!(stderr.contains("unknown LitRadar subcommand: worker"));
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
