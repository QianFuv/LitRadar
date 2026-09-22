//! Persistence, ownership, migration and refresh failure regression tests for CFPs.

use std::sync::{Arc, Barrier};

use litradar_domain::cfp::{CfpEmptyJournal, CfpSeed, CfpSource};
use litradar_storage::business::cfp::{
    begin_cfp_refresh, fail_cfp_refresh, import_cfp_seed, load_cfp_journals,
    publish_cfp_full_text_refresh, publish_cfp_refresh, CfpPublication, CfpRepositoryError,
};
use litradar_storage::{
    create_backup, migrate_auth_database, restore_backup, verify_backup, BackupCreateOptions,
    BackupRestoreOptions, StorageConfig, AUTH_SCHEMA_VERSION,
};
use rusqlite::Connection;
use tempfile::tempdir;

const SHIPPED: &[u8] = include_bytes!("../../litradar-sources/assets/cfp-seed.json");

fn source(key: &str) -> CfpSource {
    serde_json::from_value(serde_json::json!({"catalogIds":[key],"journalTitle":"Example Journal","title":"Original special issue","typeText":"Special Issue","dateText":"Submission deadline: 31 December 2026","sourceUrl":"https://example.org/cfp","checkedOn":"2026-09-15"})).unwrap()
}

fn payload(sources: Vec<CfpSource>) -> Vec<u8> {
    serde_json::to_vec(&CfpSeed {
        format_version: 1,
        sources,
        empty_journals: Vec::new(),
        expected_journals: None,
        expected_notices: None,
    })
    .unwrap()
}

fn publication(sources: Vec<CfpSource>) -> CfpPublication {
    CfpPublication {
        capture: "<main>Original verified CFP</main>".into(),
        capture_format: "html".into(),
        source_url: "https://example.org/cfp".into(),
        config_version: 1,
        sources,
        empty_journals: Vec::new(),
    }
}

#[test]
fn cfp_shipped_seed_preserves_all_originals_memberships_and_empty_statements() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("auth.sqlite");
    migrate_auth_database(&path).unwrap();
    let imported = import_cfp_seed(&path, "bundled-v1", SHIPPED).unwrap();
    assert!(imported.did_import);
    assert_eq!((imported.journals, imported.notices), (467, 1622));
    assert!(
        !import_cfp_seed(&path, "bundled-v1", SHIPPED)
            .unwrap()
            .did_import
    );
    let journals = load_cfp_journals(&path).unwrap();
    assert_eq!(journals.len(), 467);
    assert_eq!(
        journals
            .iter()
            .map(|journal| journal.notices.len())
            .sum::<usize>(),
        1622
    );
    assert_eq!(
        journals
            .iter()
            .filter(|journal| journal.notices.is_empty() && journal.source_statement.is_some())
            .count(),
        16
    );
    assert!(journals.iter().any(|journal|journal.notices.iter().any(|notice|notice.requirements.contains("Manuscripts should not be published or currently submitted for publication elsewhere."))));
    let connection = Connection::open(&path).unwrap();
    let original: String=connection.query_row("SELECT source_json FROM cfp_notices WHERE json_extract(source_json,'$.rawDateText') != '' LIMIT 1",[],|row|row.get(0)).unwrap();
    assert!(!serde_json::from_str::<CfpSource>(&original)
        .unwrap()
        .raw_date_text
        .is_empty());
}

#[test]
fn cfp_import_rejects_partial_conflicting_and_corrupt_snapshots_atomically() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("auth.sqlite");
    migrate_auth_database(&path).unwrap();
    let mut first = source("journal-a");
    first.catalog_ids.push("alias-a".into());
    let mut second = source("journal-b");
    second.catalog_ids.push("alias-a".into());
    assert!(import_cfp_seed(&path, "conflict", &payload(vec![first.clone(), second])).is_err());
    assert!(load_cfp_journals(&path).unwrap().is_empty());
    assert!(import_cfp_seed(&path, "corrupt", b"{invalid}").is_err());
    import_cfp_seed(&path, "one", &payload(vec![first])).unwrap();
    let journals = load_cfp_journals(&path).unwrap();
    assert_eq!(journals[0].catalog_ids, vec!["alias-a", "journal-a"]);
    assert!(import_cfp_seed(
        &path,
        "collision",
        &payload(vec![source("journal-b"), source("journal-a")])
    )
    .is_err());
    assert_eq!(load_cfp_journals(&path).unwrap().len(), 1);
    assert!(import_cfp_seed(&path, "one", &payload(vec![source("journal-c")])).is_err());
}

#[test]
fn cfp_refresh_rejects_late_writers_and_failed_partial_results_retain_last_good_data() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("auth.sqlite");
    migrate_auth_database(&path).unwrap();
    let seed = payload(vec![source("journal-a")]);
    import_cfp_seed(&path, "one", &seed).unwrap();
    let first = begin_cfp_refresh(&path, "journal:journal-a", 100, 30).unwrap();
    let second = begin_cfp_refresh(&path, "journal:journal-a", 101, 30).unwrap();
    let mut revised = source("journal-a");
    revised.title = "Newly discovered original call".into();
    publish_cfp_refresh(&path, &second, &publication(vec![revised.clone()]), 102).unwrap();
    assert!(matches!(
        publish_cfp_refresh(&path, &first, &publication(vec![source("journal-a")]), 103),
        Err(CfpRepositoryError::StaleRefresh)
    ));
    assert!(fail_cfp_refresh(&path, &first, "Late failure", false).is_err());
    assert!(!import_cfp_seed(&path, "one", &seed).unwrap().did_import);
    let failed = begin_cfp_refresh(&path, "journal:journal-a", 104, 30).unwrap();
    assert!(publish_cfp_refresh(&path, &failed, &publication(Vec::new()), 105).is_err());
    fail_cfp_refresh(
        &path,
        &failed,
        "Challenge page; no verified original",
        false,
    )
    .unwrap();
    let journals = load_cfp_journals(&path).unwrap();
    assert_eq!(journals[0].notices[0].title, revised.title);
    assert_eq!(journals[0].sources[0].status, "failed");
    assert_eq!(journals[0].sources[0].last_success, Some(102));
    assert_eq!(journals[0].sources[0].revision, 2);
    let expired = begin_cfp_refresh(&path, "journal:journal-a", 200, 1).unwrap();
    assert!(matches!(
        publish_cfp_refresh(&path, &expired, &publication(vec![revised]), 202),
        Err(CfpRepositoryError::StaleRefresh)
    ));
    let verified = begin_cfp_refresh(&path, "journal:journal-a", 300, 30).unwrap();
    let mut empty = publication(Vec::new());
    empty.empty_journals.push(CfpEmptyJournal {
        catalog_ids: vec!["journal-a".into()],
        journal_title: "Example Journal".into(),
        checked_on: "2026-09-15".into(),
        source_url: "https://example.org/cfp".into(),
        source_statement: "No open special issues. Regular submissions remain open.".into(),
        notices: Vec::new(),
    });
    publish_cfp_refresh(&path, &verified, &empty, 301).unwrap();
    let journals = load_cfp_journals(&path).unwrap();
    assert!(journals[0].notices.is_empty());
    assert_eq!(
        journals[0].source_statement.as_deref(),
        Some("No open special issues. Regular submissions remain open.")
    );
}

#[test]
fn cfp_partial_full_text_keeps_unresolved_notices_and_complete_refresh_freshness() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("auth.sqlite");
    migrate_auth_database(&path).unwrap();
    let mut first = source("journal-a");
    first.scope = "Previous original excerpt.".into();
    let mut second = source("journal-a");
    second.title = "Another original call".into();
    second.scope = "Unresolved original scope.".into();
    import_cfp_seed(&path, "one", &payload(vec![first.clone(), second.clone()])).unwrap();
    let initial = begin_cfp_refresh(&path, "journal:journal-a", 100, 30).unwrap();
    publish_cfp_refresh(
        &path,
        &initial,
        &publication(vec![first.clone(), second.clone()]),
        101,
    )
    .unwrap();
    first.scope = "Complete original text with every research topic.".into();
    first.requirements = "Complete original submission requirements.".into();
    let lease = begin_cfp_refresh(&path, "journal:journal-a", 110, 30).unwrap();
    assert!(publish_cfp_full_text_refresh(
        &path,
        &lease,
        &publication(vec![first.clone()]),
        111,
        1
    )
    .is_err());
    publish_cfp_full_text_refresh(
        &path,
        &lease,
        &publication(vec![first.clone(), second.clone()]),
        111,
        1,
    )
    .unwrap();
    let journals = load_cfp_journals(&path).unwrap();
    assert_eq!(journals[0].notices.len(), 2);
    assert_eq!(journals[0].notices[0].scope, first.scope);
    assert_eq!(journals[0].notices[1].scope, second.scope);
    assert_eq!(journals[0].sources[0].status, "failed");
    assert_eq!(journals[0].sources[0].last_success, Some(101));
    assert!(journals[0].sources[0]
        .last_error
        .as_ref()
        .unwrap()
        .contains("1 notices"));
    assert!(matches!(
        publish_cfp_full_text_refresh(&path, &lease, &publication(vec![first, second]), 112, 0),
        Err(CfpRepositoryError::StaleRefresh)
    ));
}

#[test]
fn cfp_version_seventeen_concurrent_startup_preserves_business_rows_and_imports_once() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("auth.sqlite");
    migrate_auth_database(&path).unwrap();
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch("DROP TABLE cfp_notices; DROP TABLE cfp_source_journals; DROP TABLE cfp_sources; DROP TABLE cfp_journal_aliases; DROP TABLE cfp_journals; DROP TABLE cfp_seed_imports; PRAGMA user_version=17; INSERT INTO users(id,username,password_hash,salt,created_at,updated_at) VALUES(1,'preserved','hash','salt',1,1);").unwrap();
    drop(connection);
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                migrate_auth_database(&path).unwrap();
                import_cfp_seed(&path, "one", &payload(vec![source("journal-a")]))
                    .unwrap()
                    .did_import
            })
        })
        .collect();
    assert_eq!(
        handles
            .into_iter()
            .map(|handle| usize::from(handle.join().unwrap()))
            .sum::<usize>(),
        1
    );
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        AUTH_SCHEMA_VERSION
    );
    assert_eq!(
        connection
            .query_row("SELECT username FROM users WHERE id=1", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "preserved"
    );
}

#[test]
fn cfp_existing_backup_round_trip_preserves_snapshots_and_refresh_revisions() {
    let directory = tempdir().unwrap();
    let config = StorageConfig::from_project_root(directory.path().join("source"));
    migrate_auth_database(config.auth_db_path()).unwrap();
    import_cfp_seed(
        config.auth_db_path(),
        "one",
        &payload(vec![source("journal-a")]),
    )
    .unwrap();
    let lease = begin_cfp_refresh(config.auth_db_path(), "journal:journal-a", 100, 30).unwrap();
    publish_cfp_refresh(
        config.auth_db_path(),
        &lease,
        &publication(vec![source("journal-a")]),
        101,
    )
    .unwrap();
    let backup = directory.path().join("backup");
    create_backup(&BackupCreateOptions {
        storage_config: config.clone(),
        auth_db_path: config.auth_db_path().into(),
        output_dir: backup.clone(),
        include_index_databases: false,
        include_push_state: false,
    })
    .unwrap();
    verify_backup(&backup).unwrap();
    let restored = StorageConfig::from_project_root(directory.path().join("restored"));
    restore_backup(&BackupRestoreOptions {
        storage_config: restored.clone(),
        auth_db_path: restored.auth_db_path().into(),
        backup_dir: backup,
    })
    .unwrap();
    let journals = load_cfp_journals(restored.auth_db_path()).unwrap();
    assert_eq!(journals[0].notices[0].title, "Original special issue");
    assert_eq!(journals[0].sources[0].revision, 2);
    assert!(
        !import_cfp_seed(
            restored.auth_db_path(),
            "one",
            &payload(vec![source("journal-a")])
        )
        .unwrap()
        .did_import
    );
}
