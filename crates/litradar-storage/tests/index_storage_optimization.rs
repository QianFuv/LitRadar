//! Storage and schema measurements used by index optimization regression tests.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use litradar_storage::{migrate_index_database, open_sqlite_connection};
use rusqlite::Connection;
use tempfile::tempdir;

#[derive(Debug, PartialEq, Eq)]
struct StorageMeasurement {
    file_bytes: u64,
    page_bytes: u64,
    freelist_bytes: u64,
    fts_bytes: u64,
    objects: BTreeSet<String>,
}

fn measure_database(path: &Path) -> StorageMeasurement {
    let connection = Connection::open(path).expect("measurement database should open");
    let page_size = connection
        .query_row("PRAGMA page_size", [], |row| row.get::<_, u64>(0))
        .expect("page size should read");
    let page_count = connection
        .query_row("PRAGMA page_count", [], |row| row.get::<_, u64>(0))
        .expect("page count should read");
    let freelist_count = connection
        .query_row("PRAGMA freelist_count", [], |row| row.get::<_, u64>(0))
        .expect("freelist count should read");
    let fts_bytes = connection
        .query_row(
            "SELECT COALESCE(SUM(pgsize), 0) FROM dbstat WHERE name GLOB 'article_search*'",
            [],
            |row| row.get::<_, u64>(0),
        )
        .expect("FTS allocation should read");
    let mut statement = connection
        .prepare(
            "SELECT name FROM sqlite_schema
             WHERE name NOT LIKE 'sqlite_%'
             ORDER BY name",
        )
        .expect("schema object query should prepare");
    let objects = statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("schema objects should query")
        .collect::<rusqlite::Result<BTreeSet<_>>>()
        .expect("schema objects should collect");
    drop(statement);
    drop(connection);

    StorageMeasurement {
        file_bytes: fs::metadata(path)
            .expect("database metadata should read")
            .len(),
        page_bytes: page_size * page_count,
        freelist_bytes: page_size * freelist_count,
        fts_bytes,
        objects,
    }
}

#[test]
fn storage_measurement_reports_bytes_and_schema_objects_without_row_contents() {
    let directory = tempdir().expect("temporary directory should create");
    let path = directory.path().join("catalog.sqlite");
    migrate_index_database(&path, None).expect("index database should initialize");
    let connection = open_sqlite_connection(&path).expect("index database should open");
    connection
        .execute_batch(
            r#"
            INSERT INTO journals (
                journal_id, catalog_id, title, title_aliases_json, issns_json, area
            ) VALUES (1, 'fixture', 'Fixture Journal', '[]', '[]', 'Testing');

            INSERT INTO articles (
                article_id, journal_id, title, publication_year, date, authors_json,
                abstract_text, doi, pmid, open_access, in_press
            ) VALUES (
                10, 1, 'Measured Search Row', 2026, '2026-09-02', '["Author"]',
                'storage measurement token', '10.1000/measure', '10', 1, 0
            );

            INSERT INTO article_listing (
                article_id, journal_id, publication_year, date, open_access, in_press,
                doi, pmid, area
            ) VALUES (
                10, 1, 2026, '2026-09-02', 1, 0, '10.1000/measure', '10', 'Testing'
            );

            INSERT INTO article_search (
                rowid, article_id, title, abstract_text, doi, pmid, authors, journal_title
            ) VALUES (
                10, 10, 'Measured Search Row', 'storage measurement token',
                '10.1000/measure', '10', 'Author', 'Fixture Journal'
            );
            "#,
        )
        .expect("measurement fixture should insert");
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .expect("measurement database should checkpoint");
    drop(connection);

    let first = measure_database(&path);
    let second = measure_database(&path);

    assert_eq!(first, second);
    assert_eq!(first.file_bytes, first.page_bytes);
    assert_eq!(first.freelist_bytes, 0);
    assert!(first.fts_bytes > 0);
    assert!(first.objects.contains("articles"));
    assert!(first.objects.contains("article_listing"));
    assert!(first.objects.contains("article_search"));
}
