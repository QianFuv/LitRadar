//! Real-binary tests for the unified LitRadar command tree.

mod support;

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

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
fn index_command_help_exposes_full_rescan_defaults_and_mode_relationships() {
    let output = run_litradar(&["index", "--help"]);
    let payload: Value = serde_json::from_slice(&output.stdout).expect("index help should be JSON");

    assert!(output.status.success());
    assert!(payload["usage"]
        .as_str()
        .is_some_and(|usage| usage.contains("--full-rescan|--no-full-rescan")));
    assert!(payload["usage"]
        .as_str()
        .is_some_and(|usage| usage.contains("--acknowledge-unknown-notify")));
    assert_eq!(payload["defaults"]["resume"], true);
    assert_eq!(payload["defaults"]["update"], false);
    assert_eq!(payload["defaults"]["full_rescan"], false);
    assert!(payload["modes"]["full_rescan"]
        .as_str()
        .is_some_and(|value| value.contains("mutually exclusive with --update")));
    assert!(payload["modes"]["resume"]
        .as_str()
        .is_some_and(|value| value.contains("compatible active project batch")));
    assert!(payload["modes"]["no_resume"]
        .as_str()
        .is_some_and(|value| value.contains("new batch from committed anchors")));
    assert!(payload["modes"]["file"]
        .as_str()
        .is_some_and(|value| value.contains("exactly one CSV")));
    assert!(payload["modes"]["acknowledge_unknown_notify"]
        .as_str()
        .is_some_and(|value| value.contains("ambiguous notify attempt")));
}

#[test]
fn admin_index_storage_optimizer_binary_reports_help_success_noop_and_failures_as_json() {
    let help = run_litradar(&["admin", "--help"]);
    let help_payload: Value =
        serde_json::from_slice(&help.stdout).expect("admin help should be JSON");
    assert!(help.status.success());
    assert!(help_payload["usage"]
        .as_array()
        .is_some_and(|lines| lines.iter().any(|line| {
            line.as_str().is_some_and(|line| {
                line.contains("index optimize-storage --confirm-index-maintenance")
            })
        })));

    let root = tempdir().expect("temporary project root should create");
    let config = litradar_storage::StorageConfig::from_project_root(root.path());
    fs::create_dir_all(config.index_dir()).expect("index directory should create");
    litradar_storage::migrate_index_database(config.index_dir().join("fixture.sqlite"), None)
        .expect("fixture index should initialize");

    let missing_confirmation = run_litradar_in(
        root.path(),
        &["admin", "index", "optimize-storage", "--project-root", "."],
    );
    let missing_payload: Value = serde_json::from_slice(&missing_confirmation.stdout)
        .expect("confirmation failure should be JSON");
    assert!(!missing_confirmation.status.success());
    assert_eq!(missing_payload["status"], "failed");
    assert_eq!(missing_payload["error"]["code"], "confirmation_required");

    let optimized = run_litradar_in(
        root.path(),
        &[
            "admin",
            "index",
            "optimize-storage",
            "--confirm-index-maintenance",
            "--project-root",
            ".",
        ],
    );
    let optimized_payload: Value =
        serde_json::from_slice(&optimized.stdout).expect("success should be JSON");
    assert!(optimized.status.success());
    assert_eq!(optimized_payload["status"], "optimized");
    assert_eq!(optimized_payload["report"]["database_count"], 1);

    let empty = tempdir().expect("empty project root should create");
    let noop = run_litradar_in(
        empty.path(),
        &[
            "admin",
            "index",
            "optimize-storage",
            "--confirm-index-maintenance",
            "--project-root",
            ".",
        ],
    );
    let noop_payload: Value = serde_json::from_slice(&noop.stdout).expect("no-op should be JSON");
    assert!(noop.status.success());
    assert_eq!(noop_payload["status"], "noop");
    assert!(!empty.path().join("data").exists());
}

#[test]
fn admin_index_storage_optimizer_binary_refuses_active_unsupported_and_stale_targets() {
    let active_root = tempdir().expect("active project root should create");
    let active_config = litradar_storage::StorageConfig::from_project_root(active_root.path());
    fs::create_dir_all(active_config.index_dir()).expect("index directory should create");
    litradar_storage::migrate_index_database(
        active_config.index_dir().join("fixture.sqlite"),
        None,
    )
    .expect("fixture index should initialize");
    litradar_storage::migrate_auth_database(active_config.auth_db_path())
        .expect("auth database should initialize");
    litradar_storage::record_service_heartbeat(
        active_config.auth_db_path(),
        litradar_storage::ServiceKind::Worker,
        "active-worker",
        current_epoch_seconds() as f64,
    )
    .expect("active heartbeat should persist");

    let active = run_litradar_in(
        active_root.path(),
        &[
            "admin",
            "index",
            "optimize-storage",
            "--confirm-index-maintenance",
            "--project-root",
            ".",
        ],
    );
    let active_payload: Value =
        serde_json::from_slice(&active.stdout).expect("active failure should be JSON");
    assert!(!active.status.success());
    assert_eq!(active_payload["error"]["code"], "active_target");

    let unsupported_root = tempdir().expect("unsupported project root should create");
    let unsupported_config =
        litradar_storage::StorageConfig::from_project_root(unsupported_root.path());
    fs::create_dir_all(unsupported_config.index_dir()).expect("index directory should create");
    let unsupported_path = unsupported_config.index_dir().join("fixture.sqlite");
    litradar_storage::migrate_index_database(&unsupported_path, None)
        .expect("fixture index should initialize");
    let connection = litradar_storage::open_sqlite_connection(&unsupported_path)
        .expect("fixture database should open");
    connection
        .pragma_update(
            None,
            "user_version",
            litradar_storage::INDEX_SCHEMA_VERSION + 1,
        )
        .expect("fixture schema version should update");
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode = DELETE;")
        .expect("fixture should checkpoint");
    drop(connection);

    let unsupported = run_litradar_in(
        unsupported_root.path(),
        &[
            "admin",
            "index",
            "optimize-storage",
            "--confirm-index-maintenance",
            "--project-root",
            ".",
        ],
    );
    let unsupported_payload: Value =
        serde_json::from_slice(&unsupported.stdout).expect("unsupported failure should be JSON");
    assert!(!unsupported.status.success());
    assert_eq!(unsupported_payload["error"]["code"], "unsupported_schema");

    let stale_root = tempdir().expect("stale project root should create");
    fs::create_dir_all(stale_root.path().join("data")).expect("data directory should create");
    fs::write(
        stale_root
            .path()
            .join("data")
            .join(".litradar-index-maintenance.json"),
        b"retained evidence\n",
    )
    .expect("stale marker should write");
    let stale = run_litradar_in(
        stale_root.path(),
        &[
            "admin",
            "index",
            "optimize-storage",
            "--confirm-index-maintenance",
            "--project-root",
            ".",
        ],
    );
    let stale_payload: Value =
        serde_json::from_slice(&stale.stdout).expect("stale failure should be JSON");
    assert!(!stale.status.success());
    assert_eq!(stale_payload["error"]["code"], "interrupted_state");
    assert!(stale_payload["error"]["recovery_paths"]["rollback"]
        .as_str()
        .is_some_and(|path| path.contains(".litradar-index-rollback")));
}

#[test]
fn index_command_mode_conflicts_fail_before_project_mutation() {
    for arguments in [
        vec!["index", "--project-root", ".", "--update", "--full-rescan"],
        vec!["index", "--project-root", ".", "--notify"],
        vec![
            "index",
            "--project-root",
            ".",
            "--acknowledge-unknown-notify",
        ],
        vec![
            "index",
            "--project-root",
            ".",
            "--update",
            "--notify",
            "--no-resume",
            "--acknowledge-unknown-notify",
        ],
    ] {
        let root = tempdir().expect("temporary project root should be created");
        let output = run_litradar_in(root.path(), &arguments);

        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!root.path().join("data").exists());
        assert!(log_events(&output)
            .iter()
            .any(|event| event["event"] == "process.failed"));
    }
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

    let root = tempdir().expect("temporary project root should be created");
    configure_logging(root.path(), "json", "off");
    let auth_db_path = root.path().join("data").join("auth.sqlite");
    let connection =
        litradar_storage::open_sqlite_connection(&auth_db_path).expect("auth database should open");
    connection
        .execute(
            "UPDATE runtime_settings SET value = 'pretty' WHERE key = 'log_format'",
            [],
        )
        .expect("invalid format fixture should update");
    let invalid_format = run_litradar_in(root.path(), &["--help"]);
    assert!(!invalid_format.status.success());
    assert_eq!(
        String::from_utf8(invalid_format.stderr).expect("error should be UTF-8"),
        "invalid LitRadar log format\n"
    );
    connection
        .execute(
            "UPDATE runtime_settings SET value = 'json' WHERE key = 'log_format'",
            [],
        )
        .expect("valid format fixture should update");
    connection
        .execute(
            "UPDATE runtime_settings SET value = '[' WHERE key = 'log_filter'",
            [],
        )
        .expect("invalid filter fixture should update");
    let invalid_filter = run_litradar_in(root.path(), &["--help"]);
    assert!(!invalid_filter.status.success());
    assert_eq!(
        String::from_utf8(invalid_filter.stderr).expect("error should be UTF-8"),
        "invalid LitRadar log filter\n"
    );
}

#[test]
fn compact_logging_is_plain_text_and_process_context_omits_raw_arguments() {
    let root = tempdir().expect("temporary project root should be created");
    let custom_auth_db_path = root.path().join("custom-auth.sqlite");
    configure_logging_database(
        &custom_auth_db_path,
        "compact",
        litradar_storage::DEFAULT_RUNTIME_LOG_FILTER,
    );
    let compact = run_litradar_in(
        root.path(),
        &[
            "--help",
            "--auth-db",
            custom_auth_db_path
                .to_str()
                .expect("temporary path should be valid UTF-8"),
        ],
    );
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

fn configure_logging(root: &Path, format: &str, filter: &str) {
    let auth_db_path = root.join("data").join("auth.sqlite");
    configure_logging_database(&auth_db_path, format, filter);
}

fn configure_logging_database(auth_db_path: &Path, format: &str, filter: &str) {
    litradar_storage::migrate_auth_database(auth_db_path).expect("auth database should migrate");
    litradar_storage::upsert_runtime_settings(
        auth_db_path,
        &litradar_storage::SecretCodec::from_key([43_u8; 32]),
        &HashMap::from([
            ("log_format".to_string(), Some(format.to_string())),
            ("log_filter".to_string(), Some(filter.to_string())),
        ]),
        &HashMap::new(),
    )
    .expect("logging settings should update");
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
            Some(r#"{"offline":"offline_fixture"}"#.to_string()),
        )]),
        &HashMap::new(),
    )
    .expect("offline provider route should write");
    let arguments = [
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
    ];
    let failed = run_litradar_in(root.path(), &arguments);
    assert!(!failed.status.success());
    let failed_events = log_events(&failed);
    assert!(failed_events
        .iter()
        .any(|event| event["event"] == "index.batch.admitted"));
    assert!(failed_events
        .iter()
        .any(|event| event["event"] == "cli.command.failed"));

    let batch_connection = litradar_storage::open_sqlite_connection(
        storage_config
            .index_control_dir()
            .join("index-batches.sqlite"),
    )
    .expect("batch database should open");
    let batch_id = batch_connection
        .query_row(
            "SELECT batch_id FROM index_batches WHERE status = 'active'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("active batch should exist");
    assert_eq!(
        batch_connection
            .execute(
                "UPDATE index_batch_catalogs
                 SET phase = 'completed', run_id = ?2, written_article_count = ?3,
                     source_attempt_count = ?4, updated_at = ?5, completed_at = ?5
                 WHERE batch_id = ?1 AND ordinal = 0 AND phase = 'indexing'",
                (&batch_id, "offline-complete-run", 0_i64, 0_i64, 1_i64),
            )
            .expect("fixture catalog should complete"),
        1
    );
    drop(batch_connection);

    let output = run_litradar_in(root.path(), &arguments);
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
fn notify_and_push_commands_preflight_current_indexes_without_full_integrity_scans() {
    let root = tempdir().expect("temporary project root should be created");
    let storage_config = litradar_storage::StorageConfig::from_project_root(root.path());
    let secret_key_file = root.path().join("secret.key");
    fs::write(&secret_key_file, [24_u8; 32]).expect("secret key should write");
    litradar_storage::migrate_storage(&storage_config).expect("storage should migrate");
    let index_path = storage_config.index_dir().join("fixture.sqlite");
    litradar_storage::migrate_index_database(&index_path, None)
        .expect("fixture index should migrate");
    let connection =
        litradar_storage::open_sqlite_connection(&index_path).expect("fixture index should open");
    connection
        .pragma_update(None, "foreign_keys", false)
        .expect("foreign key enforcement should be disabled for the corruption fixture");
    connection
        .execute(
            "INSERT INTO article_retraction_dois (article_id, retraction_doi)
             VALUES (999, '10.1000/orphan')",
            [],
        )
        .expect("foreign key violation should be installed with enforcement disabled");
    drop(connection);

    for command in ["notify", "push"] {
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
        assert!(log_events(&output)
            .iter()
            .any(|event| event["event"] == "delivery.workflow.completed"));
        assert!(log_events(&output).iter().any(|event| {
            event["event"] == "storage.index_preflight.completed"
                && event["validation_scope"] == "schema_only"
        }));
    }
    assert!(!root.path().join("data/push_state").exists());
    assert!(!root.path().join("data/folder_push_state").exists());
}

#[test]
fn notify_and_push_emit_terminal_json_before_nonzero_exit() {
    let root = tempdir().expect("temporary project root should be created");
    let storage_config = litradar_storage::StorageConfig::from_project_root(root.path());
    let secret_key_file = root.path().join("secret.key");
    fs::write(&secret_key_file, [29_u8; 32]).expect("secret key should write");
    litradar_storage::migrate_storage(&storage_config).expect("storage should migrate");
    litradar_storage::migrate_index_database(
        storage_config.index_dir().join("fixture.sqlite"),
        None,
    )
    .expect("fixture index should migrate");

    for (command, workflow, terminal_status) in [
        (
            "notify",
            litradar_storage::DeliveryWorkflow::Notify,
            litradar_storage::DeliveryRunStatus::Failed,
        ),
        (
            "notify",
            litradar_storage::DeliveryWorkflow::Notify,
            litradar_storage::DeliveryRunStatus::Unknown,
        ),
        (
            "push",
            litradar_storage::DeliveryWorkflow::Push,
            litradar_storage::DeliveryRunStatus::Failed,
        ),
        (
            "push",
            litradar_storage::DeliveryWorkflow::Push,
            litradar_storage::DeliveryRunStatus::Unknown,
        ),
    ] {
        let external_id = format!("terminal-{command}-{}", terminal_status.as_str());
        seed_terminal_delivery_run(
            storage_config.auth_db_path(),
            workflow,
            &external_id,
            terminal_status,
        );
        let manifest_name = format!("{external_id}.changes.json");
        fs::write(
            root.path().join(&manifest_name),
            serde_json::to_vec(&serde_json::json!({
                "db_name": "fixture.sqlite",
                "run_id": external_id,
                "changed_issue_keys": [],
                "changed_inpress_journal_ids": [],
                "notifiable_article_ids": [],
            }))
            .expect("manifest should serialize"),
        )
        .expect("manifest should write");

        let output = run_litradar_in(
            root.path(),
            &[
                command,
                "--project-root",
                ".",
                "--secret-key-file",
                "secret.key",
                "--changes-file",
                &manifest_name,
                "--no-dry-run",
            ],
        );
        let stdout = String::from_utf8(output.stdout.clone()).expect("stdout should be UTF-8");
        let payload: Value =
            serde_json::from_str(stdout.trim()).expect("terminal output should be JSON");

        assert!(!output.status.success(), "{command} should exit nonzero");
        assert_eq!(stdout.lines().count(), 1);
        assert_eq!(payload["workflow"], command);
        assert_eq!(payload["status"], terminal_status.as_str());
        assert_eq!(payload["databases"][0]["status"], terminal_status.as_str());
        let events = log_events(&output);
        assert!(events
            .iter()
            .any(|event| event["event"] == "cli.command.failed"));
        assert!(events
            .iter()
            .any(|event| event["event"] == "process.failed"));
    }
}

#[test]
fn notify_internal_handoff_emits_one_compact_attempt_contract() {
    let root = tempdir().expect("temporary project root should be created");
    let storage_config = litradar_storage::StorageConfig::from_project_root(root.path());
    let secret_key_file = root.path().join("secret.key");
    fs::write(&secret_key_file, [31_u8; 32]).expect("secret key should write");
    litradar_storage::migrate_storage(&storage_config).expect("storage should migrate");
    litradar_storage::migrate_index_database(
        storage_config.index_dir().join("fixture.sqlite"),
        None,
    )
    .expect("fixture index should migrate");

    for (suffix, terminal_status, should_succeed) in [
        (
            "completed",
            litradar_storage::DeliveryRunStatus::Completed,
            true,
        ),
        (
            "unknown",
            litradar_storage::DeliveryRunStatus::Unknown,
            false,
        ),
    ] {
        let source_run_id = format!("compact-{suffix}");
        let attempt_id = if should_succeed {
            "11111111111111111111111111111111"
        } else {
            "22222222222222222222222222222222"
        };
        seed_terminal_delivery_run(
            storage_config.auth_db_path(),
            litradar_storage::DeliveryWorkflow::Notify,
            &scheduled_attempt_external_id(&source_run_id, attempt_id),
            terminal_status,
        );
        let manifest_name = format!("{source_run_id}.changes.json");
        fs::write(
            root.path().join(&manifest_name),
            serde_json::to_vec(&serde_json::json!({
                "db_name": "fixture.sqlite",
                "run_id": source_run_id,
                "notifiable_article_ids": [],
            }))
            .expect("manifest should serialize"),
        )
        .expect("manifest should write");

        let output = run_litradar_in(
            root.path(),
            &[
                "notify",
                "--project-root",
                ".",
                "--secret-key-file",
                "secret.key",
                "--changes-file",
                &manifest_name,
                "--attempt-id",
                attempt_id,
                "--internal-handoff-json",
                "--no-dry-run",
            ],
        );
        let stdout = String::from_utf8(output.stdout.clone()).expect("stdout should be UTF-8");
        let payload: Value =
            serde_json::from_str(stdout.trim()).expect("compact handoff should be JSON");

        assert_eq!(output.status.success(), should_succeed);
        assert_eq!(stdout.lines().count(), 1);
        assert_eq!(payload["protocol_version"], 1);
        assert_eq!(payload["attempt_id"], attempt_id);
        assert_eq!(payload["workflow"], "notify");
        assert_eq!(payload["mode"], "execute");
        assert_eq!(payload["status"], terminal_status.as_str());
        assert_eq!(payload["db_name"], "fixture.sqlite");
        assert!(payload.get("databases").is_none());
        assert_eq!(
            payload
                .as_object()
                .expect("payload should be an object")
                .len(),
            6
        );
    }
}

fn scheduled_attempt_external_id(source_run_id: &str, attempt_id: &str) -> String {
    format!(
        "scheduled-run-{}",
        litradar_domain::stable_sqlite_id(
            format!("{source_run_id}:{attempt_id}"),
            "scheduled-delivery-attempt",
        )
    )
}

fn seed_terminal_delivery_run(
    auth_db_path: &Path,
    workflow: litradar_storage::DeliveryWorkflow,
    external_id: &str,
    terminal_status: litradar_storage::DeliveryRunStatus,
) {
    let queued = match litradar_storage::admit_delivery_run(
        auth_db_path,
        &litradar_storage::DeliveryRunCreate {
            external_id: external_id.to_string(),
            workflow,
            scope_key: "fixture.sqlite".to_string(),
            db_name: Some("fixture.sqlite".to_string()),
            trigger_kind: litradar_storage::DeliveryTriggerKind::Scheduled,
            mode: litradar_storage::DeliveryRunMode::Execute,
            user_id: None,
            deadline_at: None,
            created_at: 10.0,
        },
    )
    .expect("terminal fixture should admit")
    {
        litradar_storage::DeliveryRunAdmissionOutcome::Enqueued(run) => run,
        admission => panic!("unexpected terminal fixture admission: {admission:?}"),
    };
    let claimed = match litradar_storage::claim_delivery_run(
        auth_db_path,
        queued.id,
        "terminal-fixture-owner",
        queued.revision,
        11.0,
        60.0,
    )
    .expect("terminal fixture should claim")
    {
        litradar_storage::DeliveryRunClaimOutcome::Claimed(run) => run,
        claim => panic!("unexpected terminal fixture claim: {claim:?}"),
    };
    let running = litradar_storage::start_delivery_run(
        auth_db_path,
        claimed.id,
        "terminal-fixture-owner",
        claimed.revision,
        12.0,
    )
    .expect("terminal fixture should start");
    litradar_storage::finalize_delivery_run(
        auth_db_path,
        running.id,
        "terminal-fixture-owner",
        running.revision,
        terminal_status,
        None,
        Some("terminal_fixture"),
        13.0,
    )
    .expect("terminal fixture should finalize");
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
    assert_eq!(
        unchanged.last_status,
        litradar_domain::SchedulerRunState::Idle
    );
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
    assert_eq!(
        updated.last_status,
        litradar_domain::SchedulerRunState::Success
    );
    assert!(updated.last_run_at.is_some());
    assert!(!storage_config
        .project_root()
        .join("data")
        .join("push_state")
        .exists());
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

fn current_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .try_into()
        .unwrap_or(i64::MAX)
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
