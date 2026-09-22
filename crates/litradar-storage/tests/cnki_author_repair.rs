//! Author-only repair invariants across supported tokenizer versions and failure paths.

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use litradar_storage::cnki_author_repair::{
    apply_cnki_author_repair, plan_cnki_author_repair, verify_cnki_author_repair, AuthorCorrection,
};
use litradar_storage::{migrate_auth_database, migrate_index_database, StorageConfig};
use rusqlite::Connection;

fn fixture(version: i64) -> (tempfile::TempDir, StorageConfig, Vec<AuthorCorrection>) {
    let root = tempfile::tempdir().unwrap();
    let config = StorageConfig::from_project_root(root.path());
    migrate_auth_database(config.auth_db_path()).unwrap();
    fs::create_dir_all(config.index_dir()).unwrap();
    let path = config.index_dir().join("chinese_journals.sqlite");
    migrate_index_database(&path).unwrap();
    let connection = Connection::open(&path).unwrap();
    litradar_storage::sqlite::load_index_tokenizer(&connection).unwrap();
    if version == 7 {
        connection.execute_batch("DROP TABLE article_search;
            CREATE VIRTUAL TABLE article_search USING fts5(article_id UNINDEXED,title,abstract_text,doi,pmid,authors,journal_title,content='',contentless_delete=1,tokenize='unicode61 remove_diacritics 2');
            CREATE INDEX idx_article_change_events_order ON article_change_events(event_id);
            PRAGMA user_version=7;").unwrap();
    }
    connection.execute_batch("INSERT INTO journals(journal_id,catalog_id,title,title_aliases_json,issns_json) VALUES(1,'chinese','Café Journal','[]','[]');
        INSERT INTO articles(article_id,journal_id,title,authors_json,abstract_text,doi,pmid,in_press) VALUES
        (1,1,'Genome study','[{\"display_name\":\"张三1\"},{\"display_name\":\"2\"}]','Original abstract','10.1000/one','123',0),
        (9007199254740993,1,'Other study','[\"Élodie2\",\"Team 2\"]',NULL,NULL,NULL,1);
        INSERT INTO article_listing(article_id,journal_id,in_press) VALUES(1,1,0),(9007199254740993,1,1);
        INSERT INTO article_identity_keys(identity_kind,identity_value,article_id) VALUES('doi','10.1000/one',1);").unwrap();
    litradar_storage::search_text::rebuild_article_search(&connection).unwrap();
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE")
        .unwrap();
    let corrections = vec![
        AuthorCorrection {
            article_id: "1".into(),
            before: r#"[{"display_name":"张三1"},{"display_name":"2"}]"#.into(),
            after: r#"[{"display_name":"张三"}]"#.into(),
            reason: "Affiliation superscripts".into(),
        },
        AuthorCorrection {
            article_id: "9007199254740993".into(),
            before: r#"["Élodie2","Team 2"]"#.into(),
            after: r#"["Élodie","Team 2"]"#.into(),
            reason: "Reviewed individual affiliation; preserve group digit".into(),
        },
    ];
    (root, config, corrections)
}

fn search(connection: &Connection, query: &str) -> Vec<i64> {
    let query = litradar_storage::search_text::prepare_search_query(
        query,
        litradar_storage::search_text::uses_simple_search(connection).unwrap(),
        litradar_domain::ArticleSearchMode::Advanced,
    );
    connection
        .prepare("SELECT rowid FROM article_search WHERE article_search MATCH ?1 ORDER BY rowid")
        .unwrap()
        .query_map([query.as_ref()], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn repairs_authors_and_fts_preserving_ids_shapes_and_all_other_data() {
    for version in [7, 9] {
        let (root, config, corrections) = fixture(version);
        let path = config.index_dir().join("chinese_journals.sqlite");
        let bytes = fs::read(&path).unwrap();
        let plan = plan_cnki_author_repair(&config, &corrections).unwrap();
        assert_eq!(fs::read(&path).unwrap(), bytes, "dry run must not write");
        assert_eq!(plan.schema_version, version);
        assert_eq!(plan.corrections.len(), 2);
        let backup = root.path().join("before.sqlite");
        apply_cnki_author_repair(&config, &plan, &backup).unwrap();
        verify_cnki_author_repair(&config, &plan, &backup).unwrap();
        let repeated = plan_cnki_author_repair(&config, &corrections).unwrap();
        assert!(repeated.corrections.is_empty());
        assert_eq!(repeated.protected_sha256, plan.protected_sha256);
        assert_eq!(repeated.authors_before_sha256, plan.authors_after_sha256);
        let connection = Connection::open(&path).unwrap();
        litradar_storage::sqlite::load_index_tokenizer(&connection).unwrap();
        assert!(search(&connection, "authors:Élodie2").is_empty());
        assert_eq!(search(&connection, "authors:Élodie"), [9007199254740993]);
        assert_eq!(search(&connection, "title:Genome"), [1]);
        assert_eq!(search(&connection, "abstract_text:Original"), [1]);
        assert_eq!(
            search(&connection, "journal_title:Cafe"),
            [1, 9007199254740993]
        );
        let original = Connection::open(backup).unwrap();
        assert_eq!(
            original
                .query_row(
                    "SELECT authors_json FROM articles WHERE article_id=1",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            corrections[0].before
        );
    }
}

#[test]
fn stale_non_author_data_or_corrections_are_rejected_before_backup() {
    let (root, config, mut corrections) = fixture(7);
    let plan = plan_cnki_author_repair(&config, &corrections).unwrap();
    let path = config.index_dir().join("chinese_journals.sqlite");
    Connection::open(&path)
        .unwrap()
        .execute("UPDATE articles SET title='Updated' WHERE article_id=1", [])
        .unwrap();
    let backup = root.path().join("before.sqlite");
    assert!(apply_cnki_author_repair(&config, &plan, &backup).is_err());
    assert!(!backup.exists());
    corrections[0].before = "[]".into();
    assert!(plan_cnki_author_repair(&config, &corrections).is_err());
}

#[test]
fn verification_rejects_wrong_search_text_even_when_rowids_are_unchanged() {
    let (root, config, corrections) = fixture(9);
    let plan = plan_cnki_author_repair(&config, &corrections).unwrap();
    let backup = root.path().join("before.sqlite");
    apply_cnki_author_repair(&config, &plan, &backup).unwrap();
    let connection = Connection::open(config.index_dir().join("chinese_journals.sqlite")).unwrap();
    litradar_storage::sqlite::load_index_tokenizer(&connection).unwrap();
    connection.execute_batch("DELETE FROM article_search WHERE rowid=1; INSERT INTO article_search(rowid,article_id,title,authors) VALUES(1,1,'Wrong title','Old author1')").unwrap();
    assert!(verify_cnki_author_repair(&config, &plan, &backup).is_err());
}

#[test]
fn verification_protects_search_tokens_of_unaffected_articles() {
    let (root, config, corrections) = fixture(7);
    let plan = plan_cnki_author_repair(&config, &corrections[..1]).unwrap();
    let backup = root.path().join("before.sqlite");
    apply_cnki_author_repair(&config, &plan, &backup).unwrap();
    let connection = Connection::open(config.index_dir().join("chinese_journals.sqlite")).unwrap();
    connection.execute_batch("DELETE FROM article_search WHERE rowid=9007199254740993; INSERT INTO article_search(rowid,article_id,title,authors) VALUES(9007199254740993,9007199254740993,'Wrong title','Old author')").unwrap();
    assert!(verify_cnki_author_repair(&config, &plan, &backup).is_err());
}

#[test]
fn backup_cannot_occupy_live_database_directories_or_sidecars() {
    let (_root, config, corrections) = fixture(7);
    let plan = plan_cnki_author_repair(&config, &corrections).unwrap();
    for path in [
        config.index_dir().join("chinese_journals.sqlite-journal"),
        config.index_dir().join("unexpected.sqlite"),
        config.auth_db_path().with_file_name("auth.sqlite-journal"),
    ] {
        assert!(apply_cnki_author_repair(&config, &plan, &path).is_err());
        assert!(!path.exists());
    }
}

#[test]
fn late_failure_rolls_back_authors_and_search_and_preserves_backup() {
    for trigger in [
        "CREATE TRIGGER fail_second BEFORE UPDATE OF authors_json ON articles WHEN NEW.article_id=9007199254740993 BEGIN SELECT RAISE(ABORT,'injected'); END",
        "CREATE TRIGGER mutate_title AFTER UPDATE OF authors_json ON articles BEGIN UPDATE articles SET title='unexpected' WHERE article_id=NEW.article_id; END",
    ] {
        let (root, config, corrections) = fixture(7);
        let path = config.index_dir().join("chinese_journals.sqlite");
        Connection::open(&path).unwrap().execute_batch(trigger).unwrap();
        let plan = plan_cnki_author_repair(&config, &corrections).unwrap();
        let backup = root.path().join("before.sqlite");
        assert!(apply_cnki_author_repair(&config, &plan, &backup).is_err());
        let after = plan_cnki_author_repair(&config, &corrections).unwrap();
        assert_eq!(after.authors_before_sha256, plan.authors_before_sha256);
        assert_eq!(after.protected_sha256, plan.protected_sha256);
        assert!(backup.exists());
        assert_eq!(search(&Connection::open(&path).unwrap(),"authors:Élodie2"),[9007199254740993]);
    }
}

#[test]
fn active_lease_existing_backup_and_interrupted_maintenance_block_writes() {
    let (root, config, corrections) = fixture(7);
    let plan = plan_cnki_author_repair(&config, &corrections).unwrap();
    let backup = root.path().join("before.sqlite");
    fs::write(&backup, "preserved backup").unwrap();
    assert!(apply_cnki_author_repair(&config, &plan, &backup).is_err());
    assert_eq!(fs::read_to_string(&backup).unwrap(), "preserved backup");
    fs::create_dir_all(config.index_control_dir()).unwrap();
    let control =
        Connection::open(config.index_control_dir().join("chinese_journals.sqlite")).unwrap();
    control
        .execute_batch("CREATE TABLE provider_leases(expires_at INTEGER)")
        .unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    control
        .execute("INSERT INTO provider_leases VALUES(?1)", [now + 300])
        .unwrap();
    assert!(apply_cnki_author_repair(&config, &plan, &root.path().join("active.sqlite")).is_err());
    assert!(!root.path().join("active.sqlite").exists());
    control.execute("DELETE FROM provider_leases", []).unwrap();
    fs::create_dir(config.project_root().join("data/.litradar-index-staging")).unwrap();
    assert!(
        apply_cnki_author_repair(&config, &plan, &root.path().join("interrupted.sqlite")).is_err()
    );
    assert!(!root.path().join("interrupted.sqlite").exists());
}

#[cfg(windows)]
#[test]
fn backup_rejects_case_variants_of_auth_sidecars_on_windows() {
    let (_root, config, corrections) = fixture(7);
    let plan = plan_cnki_author_repair(&config, &corrections).unwrap();
    let path = config.auth_db_path().with_file_name("AUTH.SQLITE-JOURNAL");
    assert!(apply_cnki_author_repair(&config, &plan, &path).is_err());
    assert!(!path.exists());
}
