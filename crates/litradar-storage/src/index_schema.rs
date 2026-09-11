//! Shared content database schema definitions and structural validation.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use rusqlite::Connection;

/// Current provider-neutral content database schema version.
pub const INDEX_SCHEMA_VERSION: i64 = 8;

/// Oldest content database schema accepted without an explicit migration.
pub const MIN_SUPPORTED_INDEX_SCHEMA_VERSION: i64 = 6;

const VERSION_SIX_SEARCH_SCHEMA: &str = "createvirtualtablearticle_searchusingfts5(\
    article_idunindexed,title,abstract_text,doi,pmid,authors,journal_title,\
    tokenize='unicode61remove_diacritics2')";
const VERSION_SEVEN_SEARCH_SCHEMA: &str = "createvirtualtablearticle_searchusingfts5(\
    article_idunindexed,title,abstract_text,doi,pmid,authors,journal_title,\
    content='',contentless_delete=1,tokenize='unicode61remove_diacritics2')";

/// Current content tables, projections, and required indexes for a new database.
pub const INDEX_CONTENT_TABLES_SQL: &str = "
    CREATE TABLE journals (
        journal_id INTEGER PRIMARY KEY,
        catalog_id TEXT NOT NULL UNIQUE,
        title TEXT NOT NULL,
        title_aliases_json TEXT NOT NULL,
        issns_json TEXT NOT NULL,
        issn TEXT,
        eissn TEXT,
        area TEXT,
        utd_rank TEXT,
        utd_rating TEXT,
        abs_rank TEXT,
        abs_rating TEXT,
        fms_rank TEXT,
        fms_rating TEXT,
        fmscn_rank TEXT,
        fmscn_rating TEXT
    );

    CREATE TABLE journal_identity_keys (
        identity_kind TEXT NOT NULL CHECK (identity_kind IN ('catalog_id', 'issn')),
        identity_value TEXT NOT NULL,
        canonical_catalog_id TEXT NOT NULL,
        PRIMARY KEY (identity_kind, identity_value)
    );

    CREATE TABLE issues (
        issue_id INTEGER PRIMARY KEY,
        journal_id INTEGER NOT NULL,
        publication_year INTEGER,
        title TEXT,
        volume TEXT,
        number TEXT,
        date TEXT,
        FOREIGN KEY (journal_id) REFERENCES journals(journal_id) ON DELETE CASCADE
    );

    CREATE TABLE articles (
        article_id INTEGER PRIMARY KEY,
        journal_id INTEGER NOT NULL,
        issue_id INTEGER,
        title TEXT NOT NULL,
        publication_year INTEGER,
        date TEXT,
        authors_json TEXT NOT NULL,
        start_page TEXT,
        end_page TEXT,
        abstract_text TEXT,
        doi TEXT,
        pmid TEXT,
        open_access INTEGER,
        in_press INTEGER,
        FOREIGN KEY (journal_id) REFERENCES journals(journal_id) ON DELETE CASCADE,
        FOREIGN KEY (issue_id) REFERENCES issues(issue_id) ON DELETE SET NULL
    );

    CREATE TABLE article_retraction_dois (
        article_id INTEGER NOT NULL,
        retraction_doi TEXT NOT NULL,
        PRIMARY KEY (article_id, retraction_doi),
        FOREIGN KEY (article_id) REFERENCES articles(article_id) ON DELETE CASCADE
    );

    CREATE TABLE article_identity_keys (
        identity_kind TEXT NOT NULL CHECK (identity_kind IN ('doi', 'pmid', 'bibliographic')),
        identity_value TEXT NOT NULL,
        article_id INTEGER NOT NULL,
        PRIMARY KEY (identity_kind, identity_value),
        FOREIGN KEY (article_id) REFERENCES articles(article_id) ON DELETE CASCADE
    );

    CREATE TABLE article_listing (
        article_id INTEGER PRIMARY KEY,
        journal_id INTEGER NOT NULL,
        issue_id INTEGER,
        publication_year INTEGER,
        date TEXT,
        open_access INTEGER,
        in_press INTEGER,
        doi TEXT,
        pmid TEXT,
        area TEXT,
        FOREIGN KEY (article_id) REFERENCES articles(article_id) ON DELETE CASCADE,
        FOREIGN KEY (journal_id) REFERENCES journals(journal_id) ON DELETE CASCADE,
        FOREIGN KEY (issue_id) REFERENCES issues(issue_id) ON DELETE SET NULL
    );

    CREATE VIRTUAL TABLE article_search
    USING fts5(
        article_id UNINDEXED,
        title,
        abstract_text,
        doi,
        pmid,
        authors,
        journal_title,
        content = '',
        contentless_delete = 1,
        tokenize = 'unicode61 remove_diacritics 2'
    );

    CREATE TABLE article_change_events (
        event_id INTEGER PRIMARY KEY,
        content_revision TEXT NOT NULL,
        article_id INTEGER NOT NULL,
        change_kind TEXT NOT NULL CHECK (change_kind IN ('upsert', 'remove')),
        journal_id INTEGER NOT NULL,
        issue_id INTEGER,
        in_press INTEGER NOT NULL CHECK (in_press IN (0, 1)),
        created_at TEXT NOT NULL
    );

    CREATE INDEX idx_journals_issn ON journals(issn);
    CREATE INDEX idx_journals_eissn ON journals(eissn);
    CREATE INDEX idx_journal_identity_keys_catalog
        ON journal_identity_keys(canonical_catalog_id);
    CREATE INDEX idx_issues_journal_year ON issues(journal_id, publication_year);
    CREATE INDEX idx_articles_journal ON articles(journal_id);
    CREATE INDEX idx_articles_issue ON articles(issue_id);
    CREATE INDEX idx_articles_date_id ON articles(date, article_id);
    CREATE INDEX idx_articles_doi ON articles(doi);
    CREATE INDEX idx_articles_pmid ON articles(pmid);
    CREATE INDEX idx_article_retraction_dois_doi
        ON article_retraction_dois(retraction_doi);
    CREATE INDEX idx_article_identity_keys_article ON article_identity_keys(article_id);
    CREATE INDEX idx_article_listing_date_id ON article_listing(date, article_id);
    CREATE INDEX idx_article_listing_journal_date_id
        ON article_listing(journal_id, date, article_id);
    CREATE INDEX idx_article_listing_issue ON article_listing(issue_id);
    CREATE UNIQUE INDEX idx_article_change_events_revision
        ON article_change_events(
            content_revision, article_id, change_kind, journal_id,
            COALESCE(issue_id, -1), in_press
        );
";

/// SQLite failure or a mismatch with an exact supported content schema.
#[derive(Debug)]
pub enum IndexSchemaError {
    /// SQLite could not inspect the database structure.
    Sqlite(rusqlite::Error),
    /// The inspected structure does not match its declared version.
    InvalidStructure(String),
}

impl fmt::Display for IndexSchemaError {
    /// Format a structural validation failure without database contents.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::InvalidStructure(message) => formatter.write_str(message),
        }
    }
}

impl Error for IndexSchemaError {
    /// Return the underlying SQLite failure when present.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::InvalidStructure(_) => None,
        }
    }
}

impl From<rusqlite::Error> for IndexSchemaError {
    /// Preserve SQLite failures during structure inspection.
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

/// Validate exact content table, column, index, and FTS layouts without modifying data.
///
/// # Arguments
///
/// * `connection` - Connection used only for schema inspection.
/// * `version` - Declared content schema version, including supported migration sources.
///
/// # Returns
///
/// Success when the structure matches its version, or a diagnostic validation failure.
pub fn validate_index_schema_structure(
    connection: &Connection,
    version: i64,
) -> Result<(), IndexSchemaError> {
    if !(4..=INDEX_SCHEMA_VERSION).contains(&version) {
        return Err(IndexSchemaError::InvalidStructure(format!(
            "unsupported content schema version {version}"
        )));
    }
    let mut expected = [
        "article_change_events",
        "article_identity_keys",
        "article_listing",
        "article_retraction_dois",
        "article_search",
        "articles",
        "issues",
        "journal_identity_keys",
        "journals",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<BTreeSet<_>>();
    if version < 5 {
        expected.remove("journal_identity_keys");
    }
    if version < 6 {
        expected.remove("article_retraction_dois");
    }
    let mut statement = connection.prepare(
        "SELECT name
         FROM sqlite_schema
         WHERE type = 'table'
           AND name NOT LIKE 'sqlite_%'
           AND name NOT LIKE 'article_search_%'
         ORDER BY name",
    )?;
    let actual = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;
    if actual != expected {
        return Err(IndexSchemaError::InvalidStructure(format!(
            "table inventory mismatch: {actual:?}"
        )));
    }
    let expected_columns: &[(&str, &[&str])] = &[
        (
            "journals",
            &[
                "journal_id",
                "catalog_id",
                "title",
                "title_aliases_json",
                "issns_json",
                "issn",
                "eissn",
                "area",
                "utd_rank",
                "utd_rating",
                "abs_rank",
                "abs_rating",
                "fms_rank",
                "fms_rating",
                "fmscn_rank",
                "fmscn_rating",
            ],
        ),
        (
            "journal_identity_keys",
            &["identity_kind", "identity_value", "canonical_catalog_id"],
        ),
        (
            "issues",
            &[
                "issue_id",
                "journal_id",
                "publication_year",
                "title",
                "volume",
                "number",
                "date",
            ],
        ),
        (
            "articles",
            &[
                "article_id",
                "journal_id",
                "issue_id",
                "title",
                "publication_year",
                "date",
                "authors_json",
                "start_page",
                "end_page",
                "abstract_text",
                "doi",
                "pmid",
                "open_access",
                "in_press",
            ],
        ),
        ("article_retraction_dois", &["article_id", "retraction_doi"]),
        (
            "article_identity_keys",
            &["identity_kind", "identity_value", "article_id"],
        ),
        (
            "article_listing",
            &[
                "article_id",
                "journal_id",
                "issue_id",
                "publication_year",
                "date",
                "open_access",
                "in_press",
                "doi",
                "pmid",
                "area",
            ],
        ),
        (
            "article_search",
            &[
                "article_id",
                "title",
                "abstract_text",
                "doi",
                "pmid",
                "authors",
                "journal_title",
            ],
        ),
        (
            "article_change_events",
            &[
                "event_id",
                "content_revision",
                "article_id",
                "change_kind",
                "journal_id",
                "issue_id",
                "in_press",
                "created_at",
            ],
        ),
    ];
    for (table_name, expected) in expected_columns {
        if (*table_name == "journal_identity_keys" && version < 5)
            || (*table_name == "article_retraction_dois" && version < 6)
        {
            continue;
        }
        let mut expected = expected.to_vec();
        if *table_name == "articles" && version < 6 {
            expected.push("retraction_doi");
        }
        let mut statement = connection.prepare(&format!("PRAGMA table_info({table_name})"))?;
        let actual = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if actual != expected {
            return Err(IndexSchemaError::InvalidStructure(format!(
                "column inventory mismatch for {table_name}: {actual:?}"
            )));
        }
    }
    let mut expected_indexes = [
        "idx_article_change_events_revision",
        "idx_article_identity_keys_article",
        "idx_article_listing_date_id",
        "idx_article_listing_issue",
        "idx_article_listing_journal_date_id",
        "idx_article_retraction_dois_doi",
        "idx_articles_date_id",
        "idx_articles_doi",
        "idx_articles_issue",
        "idx_articles_journal",
        "idx_articles_pmid",
        "idx_issues_journal_year",
        "idx_journal_identity_keys_catalog",
        "idx_journals_eissn",
        "idx_journals_issn",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<BTreeSet<_>>();
    if version < 5 {
        expected_indexes.remove("idx_journal_identity_keys_catalog");
    }
    if version < 6 {
        expected_indexes.remove("idx_article_retraction_dois_doi");
    }
    if version < 8 {
        expected_indexes.insert("idx_article_change_events_order".to_string());
    }
    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_schema
         WHERE type = 'index' AND name NOT LIKE 'sqlite_%'
         ORDER BY name",
    )?;
    let actual_indexes = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;
    if actual_indexes != expected_indexes {
        return Err(IndexSchemaError::InvalidStructure(format!(
            "index inventory mismatch: {actual_indexes:?}"
        )));
    }
    validate_search_storage(connection, version)?;
    Ok(())
}

fn validate_search_storage(connection: &Connection, version: i64) -> Result<(), IndexSchemaError> {
    let schema_sql = connection.query_row(
        "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'article_search'",
        [],
        |row| row.get::<_, String>(0),
    )?;
    let compact_sql = schema_sql
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    let has_content_shadow = connection.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM sqlite_schema
             WHERE type = 'table' AND name = 'article_search_content'
         )",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    let is_valid_storage = match version {
        4..=6 => has_content_shadow && compact_sql == VERSION_SIX_SEARCH_SCHEMA,
        7..=INDEX_SCHEMA_VERSION => {
            !has_content_shadow && compact_sql == VERSION_SEVEN_SEARCH_SCHEMA
        }
        _ => false,
    };
    if !is_valid_storage {
        return Err(IndexSchemaError::InvalidStructure(
            "article_search storage options do not match the declared schema version".to_string(),
        ));
    }
    Ok(())
}
