//! Cross-domain provider-neutral index repository test fixture.

use std::{fs, path::PathBuf};

use rusqlite::Connection;
use serde_json::Value as JsonValue;
use tempfile::{tempdir, TempDir};

use super::*;

pub(super) struct IndexFixture {
    pub(super) _project_root: TempDir,
    pub(super) config: StorageConfig,
    pub(super) db_name: String,
}

impl IndexFixture {
    pub(super) fn new(_is_listing_ready: bool) -> Self {
        let project_root = tempdir().expect("project root should be created");
        let config = StorageConfig::from_project_root(project_root.path());
        fs::create_dir_all(config.index_dir()).expect("index dir should be created");
        crate::initialize_auth_database(config.auth_db_path())
            .expect("auth database should initialize");
        create_fixture_user(&config);
        let db_name = "fixture.sqlite".to_string();
        let connection = Connection::open(config.index_dir().join(&db_name))
            .expect("fixture database should be created");
        create_fixture_schema(&connection);
        Self {
            _project_root: project_root,
            config,
            db_name,
        }
    }
}

pub(super) fn article_filter_params() -> ArticleListParams {
    ArticleListParams {
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

pub(super) fn create_fixture_schema(connection: &Connection) {
    connection
        .execute_batch(crate::migrations::INDEX_CONTENT_TABLES_SQL)
        .expect("exact content schema should be created");
    connection
        .execute_batch(
            r#"
            INSERT INTO journals (
                journal_id, catalog_id, title, title_aliases_json, issns_json,
                issn, eissn, area, utd_rank, utd_rating, abs_rank, abs_rating,
                fms_rank, fms_rating, fmscn_rank, fmscn_rating
            ) VALUES
                (1, 'alpha-journal', 'Alpha Journal', '["Alpha J."]', '["1234-5679"]',
                 '1234-5679', NULL, 'Medicine', '1', 'A', NULL, NULL, NULL, NULL, NULL, NULL),
                (2, 'beta-journal', 'Beta Journal', '[]', '["2049-3630"]',
                 '2049-3630', NULL, 'Engineering', NULL, NULL, '2', 'A', NULL, NULL, NULL, NULL),
                (3, 'gamma-journal', 'Gamma Journal', '[]', '[]',
                 NULL, NULL, 'Medicine', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL);

            INSERT INTO journal_identity_keys (
                identity_kind, identity_value, canonical_catalog_id
            ) VALUES
                ('catalog_id', 'alpha-journal', 'alpha-journal'),
                ('issn', '1234-5679', 'alpha-journal'),
                ('catalog_id', 'beta-journal', 'beta-journal'),
                ('issn', '2049-3630', 'beta-journal'),
                ('catalog_id', 'gamma-journal', 'gamma-journal');

            INSERT INTO issues
                (issue_id, journal_id, publication_year, title, volume, number, date)
            VALUES
                (10, 1, 2026, 'January Issue', '12', '1', '2026-01-05'),
                (11, 1, 2025, 'December Issue', '11', '4', '2025-12-20'),
                (20, 2, 2026, 'February Issue', '1', '2', '2026-02-01');

            INSERT INTO articles (
                article_id, journal_id, issue_id, title, publication_year, date,
                authors_json, start_page, end_page, abstract_text, doi, pmid,
                open_access, in_press
            ) VALUES
                (1001, 1, 10, 'Genome Methods', 2026, '2026-01-05',
                 '[{"display_name":"Alice"},{"display_name":"Bob"}]',
                 '1', '10', 'Genome sequencing precision study', '10.1000/genome', '1001', 1, 0),
                (1002, 1, 10, 'Clinical Data Mining', 2026, '2026-01-04', '["Carol"]',
                 '11', '20', 'Clinical data search study', '10.1000/clinical', '1002', 0, 0),
                (1003, 2, 20, 'Canonical Knowledge', 2026, '2026-02-01', '["Dan"]',
                 NULL, NULL, 'Canonical article', NULL, NULL, 0, 0),
                (1004, 1, NULL, 'Accepted Genome Preview', 2026, '2026-01-06', '["Eve"]',
                 NULL, NULL, 'Genome in press preview', '10.1000/preview', NULL, 0, 1),
                (1005, 1, 10, 'DOI Only Article', 2026, '2026-01-03', '["Frank"]',
                 '21', '22', 'DOI fallback study', '10.1000/doi-only', '1005', 0, 0),
                (1008, 1, 10, 'Bibliographic Article', 2026, '2026-01-02', '["Heidi"]',
                 '25', '26', 'Article without an external identifier', NULL, NULL, 0, 0);

            INSERT INTO article_retraction_dois (article_id, retraction_doi) VALUES
                (1001, '10.1000/retraction-b'),
                (1001, '10.1000/retraction-a');

            INSERT INTO article_listing (
                article_id, journal_id, issue_id, publication_year, date,
                open_access, in_press, doi, pmid, area
            )
            SELECT
                a.article_id, a.journal_id, a.issue_id, a.publication_year, a.date,
                a.open_access, a.in_press, a.doi, a.pmid, j.area
            FROM articles a JOIN journals j ON j.journal_id = a.journal_id;

            INSERT INTO article_search(
                rowid, article_id, title, abstract_text, doi, pmid, authors, journal_title
            ) VALUES
                (1001, 1001, 'Genome Methods', 'Genome sequencing precision study',
                 '10.1000/genome', '1001', 'Alice Bob', 'Alpha Journal'),
                (1002, 1002, 'Clinical Data Mining', 'indexedonly token stored in FTS',
                 '10.1000/clinical', '1002', 'Carol', 'Alpha Journal'),
                (1003, 1003, 'Canonical Knowledge', 'Canonical article', '', '', 'Dan', 'Beta Journal'),
                (1004, 1004, 'Accepted Genome Preview', 'Genome in press preview',
                 '10.1000/preview', '', 'Eve', 'Alpha Journal'),
                (1005, 1005, 'DOI Only Article', 'DOI fallback study',
                 '10.1000/doi-only', '1005', 'Frank', 'Alpha Journal'),
                (1008, 1008, 'Bibliographic Article', 'Article without an external identifier',
                 '', '', 'Heidi', 'Alpha Journal');
            "#,
        )
        .expect("fixture schema and data should be created");
    connection
        .pragma_update(None, "user_version", crate::INDEX_SCHEMA_VERSION)
        .expect("fixture schema version should be set");
}
