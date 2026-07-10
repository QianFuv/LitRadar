//! Cross-domain index repository test fixture.

use std::{fs, path::PathBuf};

use rusqlite::Connection;
use serde_json::Value as JsonValue;
use tempfile::{tempdir, TempDir};

use super::*;

pub(super) struct IndexFixture {
    pub(super) _project_root: TempDir,
    pub(super) config: StorageConfig,
    pub(super) secret_codec: SecretCodec,
    pub(super) db_name: String,
}

impl IndexFixture {
    pub(super) fn new(is_listing_ready: bool) -> Self {
        let project_root = tempdir().expect("project root should be created");
        let config = StorageConfig::from_project_root(project_root.path());
        fs::create_dir_all(config.index_dir()).expect("index dir should be created");
        crate::initialize_auth_database(config.auth_db_path())
            .expect("auth database should initialize");
        create_fixture_user(&config);
        let db_name = "fixture.sqlite".to_string();
        let connection = Connection::open(config.index_dir().join(&db_name))
            .expect("fixture database should be created");
        create_fixture_schema(&connection, is_listing_ready);
        Self {
            _project_root: project_root,
            config,
            secret_codec: SecretCodec::from_key([12_u8; 32]),
            db_name,
        }
    }
}
pub(super) fn article_filter_params() -> ArticleListParams {
    ArticleListParams {
        suppressed: Some(false),
        sort: Some("date:desc".to_string()),
        limit: 20,
        ..ArticleListParams::default()
    }
}

pub(super) fn article_ids(page: &ArticlePage) -> Vec<i64> {
    page.items
        .iter()
        .map(|article| article.article_id.value())
        .collect()
}

pub(super) fn value_counts(values: Vec<ValueCount>) -> Vec<(String, i64)> {
    values
        .into_iter()
        .map(|value| (value.value, value.count))
        .collect()
}

pub(super) fn candidate_ids(candidates: &[ArticleCandidateInfo]) -> Vec<i64> {
    candidates
        .iter()
        .map(|candidate| candidate.article_id)
        .collect()
}

pub(super) fn weekly_article_ids(articles: &[WeeklyArticleRecord]) -> Vec<i64> {
    articles
        .iter()
        .map(|article| article.article_id.value())
        .collect()
}

pub(super) fn fixture_db_path(fixture: &IndexFixture) -> PathBuf {
    fixture.config.index_dir().join(&fixture.db_name)
}

pub(super) fn write_weekly_manifest(config: &StorageConfig, filename: &str, payload: JsonValue) {
    let push_state_dir = config.project_root().join("data").join("push_state");
    fs::create_dir_all(&push_state_dir).expect("push state dir should be created");
    fs::write(
        push_state_dir.join(filename),
        serde_json::to_string(&payload).expect("manifest should serialize"),
    )
    .expect("manifest should be written");
}

pub(super) fn create_fixture_user(config: &StorageConfig) {
    let connection = Connection::open(config.auth_db_path()).expect("auth database should open");
    connection
        .execute_batch(
            "
                INSERT INTO users
                    (id, username, password_hash, salt, is_admin, created_at, updated_at)
                VALUES
                    (1, 'fixture', 'hash', 'salt', 1, 0.0, 0.0);
                ",
        )
        .expect("fixture user should be inserted");
}

pub(super) fn create_fixture_schema(connection: &Connection, is_listing_ready: bool) {
    let listing_status = if is_listing_ready { "ready" } else { "stale" };
    connection
        .execute_batch(&format!(
            r#"
                CREATE TABLE journals (
                    journal_id INTEGER PRIMARY KEY,
                    library_id TEXT NOT NULL,
                    platform_journal_id TEXT,
                    title TEXT,
                    issn TEXT,
                    eissn TEXT,
                    scimago_rank REAL,
                    cover_url TEXT,
                    available INTEGER,
                    toc_data_approved_and_live INTEGER,
                    has_articles INTEGER
                );

                CREATE TABLE journal_meta (
                    journal_id INTEGER PRIMARY KEY,
                    source_csv TEXT,
                    area TEXT,
                    csv_title TEXT,
                    csv_issn TEXT,
                    csv_library TEXT
                );

                CREATE TABLE issues (
                    issue_id INTEGER PRIMARY KEY,
                    journal_id INTEGER NOT NULL,
                    publication_year INTEGER,
                    title TEXT,
                    volume TEXT,
                    number TEXT,
                    date TEXT,
                    is_valid_issue INTEGER,
                    suppressed INTEGER,
                    embargoed INTEGER,
                    within_subscription INTEGER
                );

                CREATE TABLE articles (
                    article_id INTEGER PRIMARY KEY,
                    journal_id INTEGER NOT NULL,
                    issue_id INTEGER,
                    title TEXT,
                    date TEXT,
                    authors TEXT,
                    start_page TEXT,
                    end_page TEXT,
                    abstract TEXT,
                    doi TEXT,
                    pmid TEXT,
                    permalink TEXT,
                    suppressed INTEGER,
                    in_press INTEGER,
                    open_access INTEGER,
                    platform_id TEXT,
                    retraction_doi TEXT,
                    within_library_holdings INTEGER,
                    content_location TEXT,
                    full_text_file TEXT
                );

                CREATE TABLE article_listing (
                    article_id INTEGER PRIMARY KEY,
                    journal_id INTEGER,
                    issue_id INTEGER,
                    title TEXT,
                    date TEXT,
                    authors TEXT,
                    abstract TEXT,
                    doi TEXT,
                    pmid TEXT,
                    permalink TEXT,
                    suppressed INTEGER,
                    in_press INTEGER,
                    open_access INTEGER,
                    platform_id TEXT,
                    retraction_doi TEXT,
                    within_library_holdings INTEGER,
                    content_location TEXT,
                    full_text_file TEXT,
                    journal_title TEXT,
                    volume TEXT,
                    number TEXT,
                    area TEXT,
                    publication_year INTEGER
                );

                CREATE TABLE listing_state (
                    id INTEGER PRIMARY KEY,
                    status TEXT NOT NULL
                );

                CREATE VIRTUAL TABLE article_search
                USING fts5(article_id UNINDEXED, title, abstract, doi);

                INSERT INTO journals
                    (journal_id, library_id, title, issn, eissn, scimago_rank, cover_url,
                     available, toc_data_approved_and_live, has_articles)
                VALUES
                    (1, 'scholarly', 'Alpha Journal', '1111-1111', '2222-2222', 10.5,
                     'https://covers.example/alpha.png', 1, 1, 1),
                    (2, 'cnki', 'Beta CNKI', '3333-3333', NULL, 3.0, NULL, 1, 1, 1),
                    (3, 'scholarly', 'Gamma Hidden', NULL, NULL, NULL, NULL, 0, 0, 0);

                INSERT INTO journal_meta
                    (journal_id, source_csv, area, csv_title, csv_issn, csv_library)
                VALUES
                    (1, 'english.csv', 'Medicine', 'Alpha CSV', '1111-1111', 'scholarly'),
                    (2, 'cnki.csv', 'Engineering', 'Beta CSV', '3333-3333', 'cnki'),
                    (3, 'english.csv', 'Medicine', 'Gamma CSV', '', 'scholarly');

                INSERT INTO issues
                    (issue_id, journal_id, publication_year, title, volume, number, date,
                     is_valid_issue, suppressed, embargoed, within_subscription)
                VALUES
                    (10, 1, 2026, 'January Issue', '12', '1', '2026-01-05', 1, 0, 0, 1),
                    (11, 1, 2025, 'Suppressed Issue', '11', '4', '2025-12-20', 1, 1, 0, 1),
                    (20, 2, 2026, 'CNKI Issue', '1', '2', '2026-02-01', 1, 0, 0, 0);

                INSERT INTO articles
                    (article_id, journal_id, issue_id, title, date, authors, start_page,
                     end_page, abstract, doi, pmid, permalink, suppressed, in_press,
                     open_access, platform_id, retraction_doi, within_library_holdings,
                     content_location, full_text_file)
                VALUES
                    (1001, 1, 10, 'Genome Methods', '2026-01-05', 'Alice; Bob', '1', '10',
                     'Genome sequencing precision study', '10.1000/genome', 'PMID-1001',
                     'https://example.test/articles/1001', 0, 0, 1, 'A-1001', NULL, 1,
                     'remote', 'https://files.example/fulltext.pdf'),
                    (1002, 1, 10, 'Clinical Data Mining', '2026-01-04', 'Carol', '11', '20',
                     'Clinical data search study', '10.1000/clinical', 'PMID-1002',
                     'https://example.test/articles/1002', 0, 0, 0, 'A-1002', NULL, 1,
                     'remote', NULL),
                    (1003, 2, 20, 'CNKI Protected Knowledge', '2026-02-01', 'Dan', NULL, NULL,
                     'CNKI protected article', NULL, NULL,
                     'https://oversea.cnki.net/kcms/detail/abc?foo=bar', 0, 0, 0, 'C-1003',
                     NULL, 0, 'remote',
                     'https://o.oversea.cnki.net/barnew/download/order?id=abc'),
                    (1004, 1, NULL, 'Accepted Genome Preview', '2026-01-06', 'Eve', NULL, NULL,
                     'Genome in press preview', '10.1000/preview', NULL,
                     'https://example.test/articles/1004', 0, 1, 0, 'A-1004', NULL, 1,
                     'remote', NULL),
                    (1005, 1, 10, 'DOI Only Article', '2026-01-03', 'Frank', '21', '22',
                     'DOI fallback study', '10.1000/doi-only', 'PMID-1005', NULL, 0, 0, 0,
                     'A-1005', NULL, 0, 'remote', NULL),
                    (1006, 1, 11, 'Suppressed Genome', '2025-12-20', 'Grace', '23', '24',
                     'Genome suppressed study', '10.1000/suppressed', NULL,
                     'https://example.test/articles/1006', 1, 0, 1, 'A-1006', NULL, 1,
                     'remote', NULL),
                    (1008, 1, 10, 'No Link Article', '2026-01-02', 'Heidi', '25', '26',
                     'Article without any outbound link', NULL, NULL, NULL, 0, 0, 0,
                     'A-1008', NULL, 0, 'remote', NULL);

                INSERT INTO article_listing
                    (article_id, journal_id, issue_id, title, date, authors, abstract, doi,
                     pmid, permalink, suppressed, in_press, open_access, platform_id,
                     retraction_doi, within_library_holdings, content_location, full_text_file,
                     journal_title, volume, number, area, publication_year)
                SELECT
                    a.article_id, a.journal_id, a.issue_id, a.title, a.date, a.authors,
                    a.abstract, a.doi, a.pmid, a.permalink, a.suppressed, a.in_press,
                    a.open_access, a.platform_id, a.retraction_doi, a.within_library_holdings,
                    a.content_location, a.full_text_file, j.title, i.volume, i.number,
                    m.area, i.publication_year
                FROM articles a
                JOIN journals j ON j.journal_id = a.journal_id
                JOIN journal_meta m ON m.journal_id = a.journal_id
                LEFT JOIN issues i ON i.issue_id = a.issue_id;

                INSERT INTO listing_state (id, status) VALUES (1, '{listing_status}');

                INSERT INTO article_search(rowid, article_id, title, abstract, doi)
                VALUES
                    (1001, 1001, 'Genome Methods', 'Genome sequencing precision study',
                     '10.1000/genome'),
                    (1002, 1002, 'Clinical Data Mining', 'indexedonly token stored in FTS',
                     '10.1000/clinical'),
                    (1003, 1003, 'CNKI Protected Knowledge', 'CNKI protected article', ''),
                    (1004, 1004, 'Accepted Genome Preview', 'Genome in press preview',
                     '10.1000/preview'),
                    (1005, 1005, 'DOI Only Article', 'DOI fallback study', '10.1000/doi-only'),
                    (1006, 1006, 'Suppressed Genome', 'Genome suppressed study',
                     '10.1000/suppressed'),
                    (1008, 1008, 'No Link Article', 'Article without any outbound link', '');
                "#
        ))
        .expect("fixture schema and data should be created");
    connection
        .pragma_update(None, "user_version", crate::INDEX_SCHEMA_VERSION)
        .expect("fixture schema version should be set");
}
