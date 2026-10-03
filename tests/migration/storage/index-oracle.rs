//! Freeze genuine historical layouts using unchanged original Rust test constructors.
use litradar_storage::migrate_index_database;
use rusqlite::Connection;
use std::path::Path;

fn create_version_four_index_database(path: &Path, has_content: bool) {
    migrate_index_database(path).expect("current index fixture should initialize");
    let connection = open_fixture_connection(path).expect("version four fixture should open");
    connection
        .execute_batch(
            "DROP TABLE article_search;
             CREATE VIRTUAL TABLE article_search
             USING fts5(
                 article_id UNINDEXED,
                 title,
                 abstract_text,
                 doi,
                 pmid,
                 authors,
                 journal_title,
                 tokenize = 'unicode61 remove_diacritics 2'
             );
             DROP TABLE article_retraction_dois;
             ALTER TABLE articles ADD COLUMN retraction_doi TEXT;
             DROP TABLE journal_identity_keys;
             CREATE INDEX IF NOT EXISTS idx_article_change_events_order ON article_change_events(event_id);
             PRAGMA user_version = 4;",
        )
        .expect("current-only index objects should be removed");
    if has_content {
        connection
            .execute_batch(
                r#"
                INSERT INTO journals (
                    journal_id, catalog_id, title, title_aliases_json, issns_json,
                    issn, eissn, area, utd_rank, utd_rating, abs_rank, abs_rating,
                    fms_rank, fms_rating, fmscn_rank, fmscn_rating
                ) VALUES
                    (1, 'journal-1', 'Journal One', '[]', '["1234-5679","2049-3630"]',
                     '1234-5679', '2049-3630', 'Area One', NULL, NULL, NULL, NULL,
                     NULL, NULL, NULL, NULL),
                    (2, 'journal-2', 'Journal Two', '[]', '["2434-561X"]',
                     '2434-561X', NULL, 'Area Two', NULL, NULL, NULL, NULL,
                     NULL, NULL, NULL, NULL);

                INSERT INTO issues (
                    issue_id, journal_id, publication_year, title, volume, number, date
                ) VALUES (10, 1, 2026, 'Issue One', '1', '1', '2026-01-01');

                INSERT INTO articles (
                    article_id, journal_id, issue_id, title, publication_year, date,
                    authors_json, start_page, end_page, abstract_text, doi, pmid,
                    open_access, in_press, retraction_doi
                ) VALUES (
                    100, 1, 10, 'Article One', 2026, '2026-01-01', '["Author"]',
                    '1', '9', 'Abstract', '10.1000/article-one', NULL, 1, 0, NULL
                );

                INSERT INTO article_identity_keys (
                    identity_kind, identity_value, article_id
                ) VALUES
                    ('doi', '10.1000/article-one', 100),
                    ('bibliographic', 'journal-1|2026|article one|1', 100);

                INSERT INTO article_listing (
                    article_id, journal_id, issue_id, publication_year, date,
                    open_access, in_press, doi, pmid, area
                ) VALUES (
                    100, 1, 10, 2026, '2026-01-01', 1, 0,
                    '10.1000/article-one', NULL, 'Area One'
                );

                INSERT INTO article_search (
                    rowid, article_id, title, abstract_text, doi, pmid, authors, journal_title
                ) VALUES (
                    100, 100, 'Article One', 'Abstract', '10.1000/article-one', '',
                    'Author', 'Journal One'
                );

                INSERT INTO article_change_events (
                    event_id, content_revision, article_id, change_kind, journal_id,
                    issue_id, in_press, created_at
                ) VALUES (
                    1000, 'fixture:revision', 100, 'upsert', 1, 10, 0,
                    '2026-07-20T00:00:00Z'
                );
                "#,
            )
            .expect("version four content should be inserted");
    }
}

fn create_version_five_index_database(path: &Path) {
    create_version_four_index_database(path, true);
    let connection = open_fixture_connection(path).expect("version five fixture should open");
    connection
        .execute_batch(
            "CREATE TABLE journal_identity_keys (
                 identity_kind TEXT NOT NULL CHECK (identity_kind IN ('catalog_id', 'issn')),
                 identity_value TEXT NOT NULL,
                 canonical_catalog_id TEXT NOT NULL,
                 PRIMARY KEY (identity_kind, identity_value)
             );
             CREATE INDEX idx_journal_identity_keys_catalog
                 ON journal_identity_keys(canonical_catalog_id);
             INSERT INTO journal_identity_keys (
                 identity_kind, identity_value, canonical_catalog_id
             ) VALUES
                 ('catalog_id', 'journal-1', 'journal-1'),
                 ('catalog_id', 'journal-2', 'journal-2'),
                 ('issn', '1234-5679', 'journal-1'),
                 ('issn', '2049-3630', 'journal-1'),
                 ('issn', '2434-561X', 'journal-2');
             UPDATE articles SET retraction_doi = '10.1000/legacy-relation';
             PRAGMA user_version = 5;",
        )
        .expect("version five identity and scalar state should be installed");
}

fn create_version_six_index_database(path: &Path) {
    create_version_five_index_database(path);
    migrate_index_database(path).expect("fixture should migrate to current version");
    let connection = open_fixture_connection(path).expect("version six fixture should open");
    connection
        .execute_batch(
            "DROP TABLE article_search;
             CREATE VIRTUAL TABLE article_search
             USING fts5(
                 article_id UNINDEXED,
                 title,
                 abstract_text,
                 doi,
                 pmid,
                 authors,
                 journal_title,
                 tokenize = 'unicode61 remove_diacritics 2'
             );
             INSERT INTO article_search (
                 rowid, article_id, title, abstract_text, doi, pmid, authors, journal_title
             )
             SELECT
                 articles.article_id,
                 articles.article_id,
                 articles.title,
                 articles.abstract_text,
                 articles.doi,
                 articles.pmid,
                 COALESCE((
                     SELECT group_concat(
                         CASE authors.type
                             WHEN 'object' THEN json_extract(authors.value, '$.display_name')
                             ELSE CAST(authors.value AS TEXT)
                         END,
                         '; '
                     )
                     FROM json_each(articles.authors_json) AS authors
                 ), ''),
                 journals.title
             FROM articles
             JOIN journals ON journals.journal_id = articles.journal_id
             ORDER BY articles.article_id;
             CREATE INDEX IF NOT EXISTS idx_article_change_events_order ON article_change_events(event_id);
             PRAGMA user_version = 6;",
        )
        .expect("version six search storage should install");
}

fn index_content_snapshot(path: &Path) -> Vec<Vec<String>> {
    [
        "SELECT json_array(journal_id, catalog_id, title, title_aliases_json, issns_json, issn, eissn, area, utd_rank, utd_rating, abs_rank, abs_rating, fms_rank, fms_rating, fmscn_rank, fmscn_rating) FROM journals ORDER BY journal_id",
        "SELECT json_array(issue_id, journal_id, publication_year, title, volume, number, date) FROM issues ORDER BY issue_id",
        "SELECT json_array(article_id, journal_id, issue_id, title, publication_year, date, authors_json, start_page, end_page, abstract_text, doi, pmid, open_access, in_press) FROM articles ORDER BY article_id",
        "SELECT json_array(identity_kind, identity_value, article_id) FROM article_identity_keys ORDER BY identity_kind, identity_value",
        "SELECT json_array(article_id, journal_id, issue_id, publication_year, date, open_access, in_press, doi, pmid, area) FROM article_listing ORDER BY article_id",
        "SELECT json_array(event_id, content_revision, article_id, change_kind, journal_id, issue_id, in_press, created_at) FROM article_change_events ORDER BY event_id",
    ]
    .into_iter()
    .map(|query| query_text_rows(path, query))
    .collect()
}

fn index_search_snapshot(path: &Path) -> Vec<Vec<String>> {
    [
        "article",
        "abstract",
        "doi:article",
        "authors:Author",
        "journal_title:\"Journal One\"",
    ]
    .into_iter()
    .map(|query| {
        let connection = open_fixture_connection(path).expect("database should open for FTS query");
        let query = litradar_storage::search_text::prepare_search_query(
            query,
            litradar_storage::search_text::uses_simple_search(&connection).unwrap(),
            litradar_domain::ArticleSearchMode::Advanced,
        );
        let mut statement = connection
            .prepare(
                "SELECT CAST(rowid AS TEXT) FROM article_search
                 WHERE article_search MATCH ?1
                 ORDER BY rowid",
            )
            .expect("FTS snapshot query should prepare");
        statement
            .query_map([query.as_ref()], |row| row.get::<_, String>(0))
            .expect("FTS snapshot rows should query")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("FTS snapshot rows should collect")
    })
    .collect()
}

fn query_text_rows(path: &Path, query: &str) -> Vec<String> {
    let connection = open_fixture_connection(path).expect("database should open for text query");
    let mut statement = connection
        .prepare(query)
        .expect("text query should prepare");
    statement
        .query_map([], |row| row.get(0))
        .expect("text rows should query")
        .collect::<Result<Vec<_>, _>>()
        .expect("text rows should collect")
}

fn open_fixture_connection(path: impl AsRef<std::path::Path>) -> rusqlite::Result<Connection> {
    let connection = Connection::open(path)?;
    litradar_storage::sqlite::load_index_tokenizer(&connection)?;
    Ok(connection)
}

/// Generate independent historical databases and snapshots using the original Rust fixtures.
fn main() {
    let destination =
        std::path::PathBuf::from(std::env::args().nth(1).expect("fixture destination"));
    std::fs::create_dir_all(&destination).unwrap();
    let mut observations = Vec::new();
    for version in 4..=9 {
        let path = destination.join(format!("content-v{version}.sqlite.fixture"));
        assert!(!path.exists(), "fixture output already exists");
        match version {
            4 => create_version_four_index_database(&path, true),
            5 => create_version_five_index_database(&path),
            _ => create_version_six_index_database(&path),
        }
        if version == 7 || version == 8 {
            let connection = open_fixture_connection(&path).unwrap();
            connection.execute_batch("DROP TABLE article_search; CREATE VIRTUAL TABLE article_search USING fts5(article_id UNINDEXED,title,abstract_text,doi,pmid,authors,journal_title,content='',contentless_delete=1,tokenize='unicode61 remove_diacritics 2');").unwrap();
            litradar_storage::search_text::rebuild_article_search(&connection).unwrap();
            if version == 8 {
                connection
                    .execute_batch("DROP INDEX idx_article_change_events_order")
                    .unwrap();
            }
            connection
                .pragma_update(None, "user_version", version)
                .unwrap();
        }
        if version == 9 {
            migrate_index_database(&path).unwrap();
        }
        {
            let connection = open_fixture_connection(&path).unwrap();
            connection
                .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
                .unwrap();
        }
        observations.push(serde_json::json!({"version":version,"file":format!("content-v{version}.sqlite.fixture"),"canonical":index_content_snapshot(&path),"search":index_search_snapshot(&path)}));
    }
    println!("{}", serde_json::to_string_pretty(&observations).unwrap());
}
