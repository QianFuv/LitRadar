//! Real-binary tests for the unified LitRadar command tree.

mod support;

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use litradar_domain::{ScheduledDeliveryJob, ScheduledJobSpec};
use serde_json::Value;
use tempfile::tempdir;

use support::{log_events, run_litradar, run_litradar_in, run_litradar_with_env};

const LOCAL_CATALOG: &str = "catalog_id,catalog_aliases,title,issn,eissn,all_issns,title_aliases,area,utd_rank,utd_rating,abs_rank,abs_rating,fms_rank,fms_rating,fmscn_rank,fmscn_rating\nissn-0001-3072,,Abacus,0001-3072,1467-6281,0001-3072;1467-6281,,Accounting & Auditing,,,7,3,7,B,,\n";

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
fn admin_secret_rotation_reencrypts_values_and_keeps_output_private() {
    let root = tempdir().expect("temporary project root should be created");
    let storage_config = litradar_storage::StorageConfig::from_project_root(root.path());
    let old_key_file = root.path().join("old.key");
    let new_key_file = root.path().join("new.key");
    fs::write(&old_key_file, [21_u8; 32]).expect("old key should write");
    fs::write(&new_key_file, [22_u8; 32]).expect("new key should write");
    litradar_storage::migrate_storage(&storage_config).expect("storage should migrate");
    let old_codec =
        litradar_storage::SecretCodec::load(&old_key_file).expect("old secret codec should load");
    let secret_value = "rotation-secret-sentinel";
    litradar_storage::upsert_runtime_settings(
        storage_config.auth_db_path(),
        &old_codec,
        &HashMap::from([(
            "openalex_api_key_pool".to_string(),
            Some(secret_value.to_string()),
        )]),
        &HashMap::new(),
    )
    .expect("encrypted runtime setting should write");

    let output = run_litradar_in(
        root.path(),
        &[
            "admin",
            "secrets",
            "rotate",
            "--project-root",
            ".",
            "--old-key-file",
            "old.key",
            "--new-key-file",
            "new.key",
        ],
    );
    let payload: Value =
        serde_json::from_slice(&output.stdout).expect("rotation output should be JSON");
    let stderr = String::from_utf8(output.stderr.clone()).expect("logs should be UTF-8");

    assert!(output.status.success());
    assert_eq!(payload["status"], "rotated");
    assert_eq!(payload["rotated"], 1);
    assert!(!payload.to_string().contains(secret_value));
    assert!(!stderr.contains(secret_value));
    assert!(log_events(&output)
        .iter()
        .any(|event| event["event"] == "cli.command.completed"));
    let new_codec =
        litradar_storage::SecretCodec::load(&new_key_file).expect("new secret codec should load");
    let settings =
        litradar_storage::load_runtime_settings(storage_config.auth_db_path(), &new_codec)
            .expect("rotated settings should decrypt with the new key");
    assert_eq!(
        settings
            .iter()
            .find(|setting| setting.field == "openalex_api_key_pool")
            .expect("rotated setting should exist")
            .value,
        secret_value
    );
    assert!(
        litradar_storage::load_runtime_settings(storage_config.auth_db_path(), &old_codec,)
            .is_err()
    );
    let raw_value: String = litradar_storage::open_sqlite_connection(storage_config.auth_db_path())
        .expect("auth database should open")
        .query_row(
            "SELECT value FROM runtime_settings WHERE key = 'openalex_api_key_pool'",
            [],
            |row| row.get(0),
        )
        .expect("encrypted runtime value should load");
    assert!(raw_value.starts_with("litradarenc:v1:"));
    assert!(!raw_value.contains(secret_value));
}

#[test]
fn index_command_resumes_a_local_catalog_without_network_access() {
    let root = tempdir().expect("temporary project root should be created");
    let storage_config = litradar_storage::StorageConfig::from_project_root(root.path());
    let secret_key_file = root.path().join("secret.key");
    fs::write(&secret_key_file, [23_u8; 32]).expect("secret key should write");
    fs::create_dir_all(storage_config.meta_dir()).expect("metadata directory should be created");
    fs::write(storage_config.meta_dir().join("offline.csv"), LOCAL_CATALOG)
        .expect("local catalog should write");
    litradar_storage::migrate_storage(&storage_config).expect("storage should migrate");
    let codec =
        litradar_storage::SecretCodec::load(&secret_key_file).expect("secret codec should load");
    litradar_storage::upsert_runtime_settings(
        storage_config.auth_db_path(),
        &codec,
        &HashMap::from([(
            "index_provider_routes".to_string(),
            Some(r#"{"offline":"cnki"}"#.to_string()),
        )]),
        &HashMap::new(),
    )
    .expect("offline provider route should write");
    let control = litradar_index::control::open_control_db(
        storage_config.index_control_dir().join("offline.sqlite"),
    )
    .expect("offline control database should open");
    litradar_index::control::write_checkpoint(
        &control,
        "offline",
        "cnki",
        &litradar_index::control::CheckpointScope::Journal {
            catalog_id: "issn-0001-3072".to_string(),
        },
        r#"{"state":"complete"}"#,
        "2026-07-22T00:00:00Z",
    )
    .expect("complete local checkpoint should write");
    drop(control);

    let output = run_litradar_in(
        root.path(),
        &[
            "index",
            "--project-root",
            ".",
            "--secret-key-file",
            "secret.key",
            "--file",
            "offline.csv",
            "--workers",
            "1",
            "--processes",
            "1",
            "--issue-batch",
            "1",
            "--timeout",
            "1",
        ],
    );
    let stdout = String::from_utf8(output.stdout.clone()).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr.clone()).expect("stderr should be UTF-8");
    assert!(output.status.success(), "index should succeed: {stderr}");
    let payload: Value = serde_json::from_str(stdout.trim()).expect("index output should be JSON");

    assert_eq!(stdout.lines().count(), 1);
    assert_eq!(payload["status"], "succeeded");
    assert_eq!(payload["csvs"][0]["status"], "succeeded");
    assert_eq!(payload["csvs"][0]["journal_count"], 1);
    assert_eq!(payload["csvs"][0]["source_attempt_count"], 0);
    assert_eq!(payload["effective_concurrency"]["workers"], 1);
    assert_eq!(payload["effective_concurrency"]["processes"], 1);
    assert_eq!(payload["effective_concurrency"]["issue_batch"], 1);
    let index_path = storage_config.index_dir().join("offline.sqlite");
    let control_path = storage_config.index_control_dir().join("offline.sqlite");
    assert!(index_path.is_file());
    assert!(control_path.is_file());
    let schema_version: i64 = litradar_storage::open_sqlite_connection(index_path)
        .expect("offline index should open")
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("index schema version should load");
    assert_eq!(schema_version, litradar_storage::INDEX_SCHEMA_VERSION);
    assert!(log_events(&output)
        .iter()
        .any(|event| event["event"] == "cli.command.completed"));
}

#[test]
fn notify_and_push_commands_complete_with_local_idle_state() {
    let root = tempdir().expect("temporary project root should be created");
    let storage_config = litradar_storage::StorageConfig::from_project_root(root.path());
    let secret_key_file = root.path().join("secret.key");
    fs::write(&secret_key_file, [24_u8; 32]).expect("secret key should write");
    litradar_storage::migrate_storage(&storage_config).expect("storage should migrate");
    litradar_storage::migrate_index_database(
        storage_config.index_dir().join("fixture.sqlite"),
        None,
    )
    .expect("fixture index should migrate");

    for (command, state_directory) in [("notify", "push_state"), ("push", "folder_push_state")] {
        let output = run_litradar_in(
            root.path(),
            &[
                command,
                "--project-root",
                ".",
                "--secret-key-file",
                "secret.key",
                "--db",
                "fixture.sqlite",
                "--no-dry-run",
            ],
        );
        let payload: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("{command} output should be JSON: {error}"));

        assert!(output.status.success(), "{command} should succeed");
        assert_eq!(payload["workflow"], command);
        assert_eq!(payload["mode"], "execute");
        assert_eq!(payload["status"], "idle");
        assert_eq!(payload["databases"][0]["db_name"], "fixture.sqlite");
        assert_eq!(payload["databases"][0]["status"], "idle");
        assert_eq!(
            payload["databases"][0]["subscribers"],
            Value::Array(Vec::new())
        );
        assert!(root.path().join("data").join(state_directory).is_dir());
        assert!(log_events(&output)
            .iter()
            .any(|event| event["event"] == "delivery.workflow.completed"));
    }
}

#[test]
fn scheduler_dry_run_and_run_once_use_the_real_child_boundary() {
    let root = tempdir().expect("temporary project root should be created");
    let storage_config = litradar_storage::StorageConfig::from_project_root(root.path());
    let secret_key_file = root.path().join("secret.key");
    fs::write(&secret_key_file, [25_u8; 32]).expect("secret key should write");
    litradar_storage::migrate_storage(&storage_config).expect("storage should migrate");
    litradar_storage::migrate_index_database(
        storage_config.index_dir().join("fixture.sqlite"),
        None,
    )
    .expect("fixture index should migrate");
    let job = ScheduledJobSpec::Notify(ScheduledDeliveryJob {
        database: Some("fixture.sqlite".to_string()),
        max_candidates: Some(5),
    });
    let task = litradar_storage::create_scheduled_task(
        storage_config.auth_db_path(),
        litradar_storage::ScheduledTaskCreateParams {
            name: "fixture notify",
            job: &job,
            cron: "0 0 * * *",
            timezone: "UTC",
            timeout_seconds: 30,
            coalesce: true,
            enabled: true,
        },
    )
    .expect("scheduled task should be created");
    let task_id = task.id.to_string();

    let dry_run = run_litradar_in(
        root.path(),
        &[
            "scheduler",
            "dry-run-once",
            &task_id,
            "--project-root",
            ".",
            "--secret-key-file",
            "secret.key",
        ],
    );
    let dry_payload: Value =
        serde_json::from_slice(&dry_run.stdout).expect("dry-run output should be JSON");
    let unchanged = litradar_storage::get_scheduled_task(storage_config.auth_db_path(), task.id)
        .expect("task should load")
        .expect("task should remain present");

    assert!(dry_run.status.success());
    assert_eq!(dry_payload["found"], true);
    assert_eq!(dry_payload["did_execute"], false);
    assert_eq!(dry_payload["status"], Value::Null);
    assert_eq!(unchanged.last_status, "");
    assert!(unchanged.last_run_at.is_none());

    let executed = run_litradar_in(
        root.path(),
        &[
            "scheduler",
            "run-once",
            &task_id,
            "--project-root",
            ".",
            "--secret-key-file",
            "secret.key",
        ],
    );
    let executed_payload: Value =
        serde_json::from_slice(&executed.stdout).expect("run-once output should be JSON");
    let updated = litradar_storage::get_scheduled_task(storage_config.auth_db_path(), task.id)
        .expect("task should load")
        .expect("task should remain present");

    assert!(executed.status.success());
    assert_eq!(executed_payload["found"], true);
    assert_eq!(executed_payload["did_execute"], true);
    assert_eq!(executed_payload["status"], "success");
    assert_eq!(updated.last_status, "success");
    assert!(updated.last_run_at.is_some());
    assert!(storage_config
        .project_root()
        .join("data")
        .join("push_state")
        .is_dir());
    assert!(log_events(&executed)
        .iter()
        .any(|event| event["event"] == "scheduler.run.completed"));
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
        (
            "crates/litradar-worker/src/scheduler.rs".to_string(),
            [0, 1, 0, 1],
        ),
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
