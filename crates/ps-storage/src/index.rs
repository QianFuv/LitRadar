//! Typed repositories for index database read routes.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ps_domain::{
    ArticleAccessAction, ArticleAccessResponse, ArticleCandidateInfo, ArticleId, ArticlePage,
    ArticleRecord, IssuePage, IssueRecord, JournalId, JournalOption, JournalPage, JournalRecord,
    PageMeta, UserId, ValueCount, WeeklyArticleRecord, WeeklyDatabaseUpdate, WeeklyJournalUpdate,
    WeeklyUpdatesResponse, YearSummary,
};
use rusqlite::types::Value as SqlValue;
use rusqlite::{params_from_iter, Connection, OptionalExtension};
use serde_json::Value as JsonValue;

use crate::cnki::{get_cnki_session_status, CnkiRepositoryError};
use crate::{open_sqlite_connection, try_load_extension, DatabaseResolutionError, StorageConfig};

const MAX_LIMIT: i64 = 200;
const SIMPLE_TOKENIZER_ENV: &str = "SIMPLE_TOKENIZER_PATH";
const DETAIL_LABEL: &str = "查看详情";
const CNKI_DETAIL_LABEL: &str = "查看摘要/详情";
const FULLTEXT_LABEL: &str = "获取全文";
const DETAIL_PROVIDER: &str = "detail_url";
const DOI_PROVIDER: &str = "doi";
const STORED_FULLTEXT_PROVIDER: &str = "stored_url";
const ZJLIB_CNKI_PROVIDER: &str = "zjlib_cnki";
const CNKI_SOURCE: &str = "cnki";
const CNKI_PROTECTED_FULLTEXT_HOST: &str = "o.oversea.cnki.net";
const CNKI_PROTECTED_FULLTEXT_PATH: &str = "/barnew/download/order";
const CNKI_PDF_REPLAY_PATH_ENV: &str = "PAPER_SCANNER_CNKI_PDF_REPLAY_PATH";
const CNKI_PDF_REPLAY_FILENAME_ENV: &str = "PAPER_SCANNER_CNKI_PDF_REPLAY_FILENAME";
const CNKI_PDF_REPLAY_MODE_ENV: &str = "PAPER_SCANNER_CNKI_PDF_REPLAY_MODE";
const CNKI_PDF_REPLAY_MISMATCH: &str = "mismatch";

/// Repository errors for index read routes.
#[derive(Debug)]
pub enum IndexRepositoryError {
    /// SQLite returned an error.
    Sqlite(rusqlite::Error),
    /// Filesystem access failed.
    Io(std::io::Error),
    /// JSON parsing failed.
    Json(serde_json::Error),
    /// Database selection failed.
    DatabaseResolution(DatabaseResolutionError),
    /// CNKI session state could not be read.
    Cnki(CnkiRepositoryError),
    /// Sort field is not supported.
    UnsupportedSortField(String),
    /// Article sort is outside the compatibility surface.
    UnsupportedArticleSort,
    /// Cursor parsing failed.
    InvalidCursor,
    /// Pagination input is outside the supported range.
    InvalidPagination(&'static str),
    /// Requested row was not found.
    NotFound(&'static str),
}

impl fmt::Display for IndexRepositoryError {
    /// Format the repository error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Json(error) => write!(formatter, "{error}"),
            Self::DatabaseResolution(error) => write!(formatter, "{error}"),
            Self::Cnki(error) => write!(formatter, "{error}"),
            Self::UnsupportedSortField(field) => {
                write!(formatter, "Unsupported sort field: {field}")
            }
            Self::UnsupportedArticleSort => {
                formatter.write_str("Articles only support sort=date:desc or date:asc")
            }
            Self::InvalidCursor => formatter.write_str("Invalid cursor"),
            Self::InvalidPagination(message) => formatter.write_str(message),
            Self::NotFound(message) => formatter.write_str(message),
        }
    }
}

impl Error for IndexRepositoryError {
    /// Return the underlying source error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::DatabaseResolution(error) => Some(error),
            Self::Cnki(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for IndexRepositoryError {
    /// Convert SQLite errors into repository errors.
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<std::io::Error> for IndexRepositoryError {
    /// Convert IO errors into repository errors.
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for IndexRepositoryError {
    /// Convert JSON errors into repository errors.
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<DatabaseResolutionError> for IndexRepositoryError {
    /// Convert database resolution errors into repository errors.
    fn from(error: DatabaseResolutionError) -> Self {
        Self::DatabaseResolution(error)
    }
}

impl From<CnkiRepositoryError> for IndexRepositoryError {
    /// Convert CNKI repository errors into index repository errors.
    fn from(error: CnkiRepositoryError) -> Self {
        Self::Cnki(error)
    }
}

/// Journal list filters.
#[derive(Debug, Clone, Default)]
pub struct JournalListParams {
    /// Area filter.
    pub area: Option<String>,
    /// Library identifier filter.
    pub library_id: Option<String>,
    /// Available filter.
    pub available: Option<bool>,
    /// Has-articles filter.
    pub has_articles: Option<bool>,
    /// Publication year filter.
    pub year: Option<i64>,
    /// Minimum Scimago rank.
    pub scimago_min: Option<f64>,
    /// Maximum Scimago rank.
    pub scimago_max: Option<f64>,
    /// Sort string.
    pub sort: Option<String>,
    /// Limit.
    pub limit: i64,
    /// Offset.
    pub offset: i64,
}

/// Issue list filters.
#[derive(Debug, Clone, Default)]
pub struct IssueListParams {
    /// Journal identifier filter.
    pub journal_id: Option<i64>,
    /// Publication year filter.
    pub year: Option<i64>,
    /// Valid issue filter.
    pub is_valid_issue: Option<bool>,
    /// Suppressed filter.
    pub suppressed: Option<bool>,
    /// Embargoed filter.
    pub embargoed: Option<bool>,
    /// Subscription filter.
    pub within_subscription: Option<bool>,
    /// Sort string.
    pub sort: Option<String>,
    /// Limit.
    pub limit: i64,
    /// Offset.
    pub offset: i64,
}

/// Article list filters.
#[derive(Debug, Clone)]
pub struct ArticleListParams {
    /// Journal identifiers.
    pub journal_id: Vec<i64>,
    /// Issue identifier.
    pub issue_id: Option<i64>,
    /// Publication year.
    pub year: Option<i64>,
    /// Journal areas.
    pub area: Vec<String>,
    /// In-press filter.
    pub in_press: Option<bool>,
    /// Open-access filter.
    pub open_access: Option<bool>,
    /// Suppressed filter.
    pub suppressed: Option<bool>,
    /// Library holdings filter.
    pub within_library_holdings: Option<bool>,
    /// Minimum date.
    pub date_from: Option<String>,
    /// Maximum date.
    pub date_to: Option<String>,
    /// DOI filter.
    pub doi: Option<String>,
    /// PMID filter.
    pub pmid: Option<String>,
    /// FTS query.
    pub q: Option<String>,
    /// Sort string.
    pub sort: Option<String>,
    /// Limit.
    pub limit: i64,
    /// Offset.
    pub offset: i64,
    /// Cursor string.
    pub cursor: Option<String>,
    /// Whether to include total count.
    pub include_total: bool,
}

impl Default for ArticleListParams {
    /// Build default Python-compatible article list parameters.
    fn default() -> Self {
        Self {
            journal_id: Vec::new(),
            issue_id: None,
            year: None,
            area: Vec::new(),
            in_press: None,
            open_access: None,
            suppressed: None,
            within_library_holdings: None,
            date_from: None,
            date_to: None,
            doi: None,
            pmid: None,
            q: None,
            sort: Some("date:desc".to_string()),
            limit: 50,
            offset: 0,
            cursor: None,
            include_total: true,
        }
    }
}

/// Collect article counts grouped by journal and issue.
///
/// # Arguments
///
/// * `index_db_path` - Path to the selected index database.
///
/// # Returns
///
/// Snapshot map keyed by `journal_id:issue_id`.
pub fn collect_issue_article_counts(
    index_db_path: impl AsRef<Path>,
) -> Result<BTreeMap<String, i64>, IndexRepositoryError> {
    let connection = Connection::open(index_db_path)?;
    let mut statement = connection.prepare(
        "SELECT journal_id, issue_id, COUNT(*) FROM articles \
         WHERE issue_id IS NOT NULL GROUP BY journal_id, issue_id",
    )?;
    let rows = statement.query_map([], |row| {
        let journal_id = row.get::<_, i64>(0)?;
        let issue_id = row.get::<_, i64>(1)?;
        let count = row.get::<_, i64>(2)?;
        Ok((build_issue_key(journal_id, issue_id), count))
    })?;
    collect_rows(rows).map(|items| items.into_iter().collect())
}

/// Collect in-press article counts grouped by journal.
///
/// # Arguments
///
/// * `index_db_path` - Path to the selected index database.
///
/// # Returns
///
/// Snapshot map keyed by journal id.
pub fn collect_inpress_article_counts(
    index_db_path: impl AsRef<Path>,
) -> Result<BTreeMap<String, i64>, IndexRepositoryError> {
    let connection = Connection::open(index_db_path)?;
    let mut statement = connection.prepare(
        "SELECT journal_id, COUNT(*) FROM articles \
         WHERE issue_id IS NULL AND COALESCE(in_press, 0) = 1 GROUP BY journal_id",
    )?;
    let rows = statement.query_map([], |row| {
        let journal_id = row.get::<_, i64>(0)?;
        let count = row.get::<_, i64>(1)?;
        Ok((journal_id.to_string(), count))
    })?;
    collect_rows(rows).map(|items| items.into_iter().collect())
}

/// Fetch visible article candidates for issue keys.
///
/// # Arguments
///
/// * `index_db_path` - Path to the selected index database.
/// * `issue_keys` - Pending issue keys.
///
/// # Returns
///
/// Candidate articles ordered like the Python notification query.
pub fn fetch_candidates_for_issue_keys(
    index_db_path: impl AsRef<Path>,
    issue_keys: &[String],
) -> Result<Vec<ArticleCandidateInfo>, IndexRepositoryError> {
    if issue_keys.is_empty() {
        return Ok(Vec::new());
    }
    let mut issue_ids = issue_keys
        .iter()
        .map(|key| parse_issue_key(key).map(|(_, issue_id)| issue_id))
        .collect::<Result<Vec<_>, _>>()?;
    issue_ids.sort_unstable();
    issue_ids.dedup();
    fetch_candidates_for_issue_ids(index_db_path, &issue_ids)
}

/// Fetch visible in-press candidates for journal keys.
///
/// # Arguments
///
/// * `index_db_path` - Path to the selected index database.
/// * `inpress_keys` - Pending in-press journal keys.
///
/// # Returns
///
/// Candidate articles ordered like the Python notification query.
pub fn fetch_candidates_for_inpress_keys(
    index_db_path: impl AsRef<Path>,
    inpress_keys: &[String],
) -> Result<Vec<ArticleCandidateInfo>, IndexRepositoryError> {
    if inpress_keys.is_empty() {
        return Ok(Vec::new());
    }
    let mut journal_ids = inpress_keys
        .iter()
        .map(|key| {
            key.parse::<i64>()
                .map_err(|_| IndexRepositoryError::InvalidCursor)
        })
        .collect::<Result<Vec<_>, _>>()?;
    journal_ids.sort_unstable();
    journal_ids.dedup();
    fetch_candidates_for_inpress_journal_ids(index_db_path, &journal_ids)
}

/// Full-text route target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArticleFulltextTarget {
    /// Browser redirect target.
    Redirect(String),
    /// Replay PDF response target.
    Pdf {
        /// Download filename.
        filename: String,
        /// HTTP content type.
        content_type: String,
        /// PDF bytes.
        content: Vec<u8>,
    },
}

/// List available index database filenames.
///
/// # Arguments
///
/// * `config` - Storage paths.
///
/// # Returns
///
/// Sorted database filenames.
pub fn list_index_database_names(
    config: &StorageConfig,
) -> Result<Vec<String>, IndexRepositoryError> {
    Ok(config
        .list_index_databases()?
        .into_iter()
        .filter_map(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .map(str::to_string)
        })
        .collect())
}

/// List journal areas.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
///
/// # Returns
///
/// Area counts.
pub fn list_areas(
    config: &StorageConfig,
    db_name: Option<&str>,
) -> Result<Vec<ValueCount>, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    let mut statement = connection.prepare(
        "SELECT area AS value, COUNT(*) AS count FROM journal_meta \
         WHERE area IS NOT NULL AND area != '' GROUP BY area ORDER BY value ASC",
    )?;
    let rows = statement.query_map([], value_count_from_row)?;
    collect_rows(rows)
}

/// List journal options.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
///
/// # Returns
///
/// Journal options.
pub fn list_journal_options(
    config: &StorageConfig,
    db_name: Option<&str>,
) -> Result<Vec<JournalOption>, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    let mut statement =
        connection.prepare("SELECT journal_id, title FROM journals ORDER BY title ASC")?;
    let rows = statement.query_map([], |row| {
        Ok(JournalOption {
            journal_id: JournalId(row.get(0)?),
            title: row.get(1)?,
        })
    })?;
    collect_rows(rows)
}

/// List metadata sources.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
///
/// # Returns
///
/// Source counts.
pub fn list_sources(
    config: &StorageConfig,
    db_name: Option<&str>,
) -> Result<Vec<ValueCount>, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    let mut statement = connection.prepare(
        "SELECT csv_library AS value, COUNT(*) AS count FROM journal_meta \
         WHERE csv_library IS NOT NULL AND csv_library != '' \
         GROUP BY csv_library ORDER BY count DESC, value ASC",
    )?;
    let rows = statement.query_map([], value_count_from_row)?;
    collect_rows(rows)
}

/// List publication year summaries.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
///
/// # Returns
///
/// Year summaries.
pub fn list_years(
    config: &StorageConfig,
    db_name: Option<&str>,
) -> Result<Vec<YearSummary>, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    let mut statement = connection.prepare(
        "SELECT CAST(strftime('%Y', date) AS INTEGER) AS year, \
         COUNT(DISTINCT issue_id) AS issue_count, COUNT(DISTINCT journal_id) AS journal_count \
         FROM issues WHERE date IS NOT NULL GROUP BY year ORDER BY year DESC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(YearSummary {
            year: row.get(0)?,
            issue_count: row.get(1)?,
            journal_count: row.get(2)?,
        })
    })?;
    collect_rows(rows)
}

/// List journals with filters.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
/// * `params` - Journal filters.
///
/// # Returns
///
/// Paginated journal response.
pub fn list_journals(
    config: &StorageConfig,
    db_name: Option<&str>,
    params: &JournalListParams,
) -> Result<JournalPage, IndexRepositoryError> {
    validate_limit_offset(params.limit, params.offset)?;
    let connection = open_index_connection(config, db_name)?;
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    push_optional_text_filter(&mut clauses, &mut values, "m.area = ?", &params.area);
    push_optional_text_filter(
        &mut clauses,
        &mut values,
        "j.library_id = ?",
        &params.library_id,
    );
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "j.available = ?",
        params.available,
    );
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "j.has_articles = ?",
        params.has_articles,
    );
    if let Some(value) = params.scimago_min {
        clauses.push("j.scimago_rank >= ?".to_string());
        values.push(SqlValue::Real(value));
    }
    if let Some(value) = params.scimago_max {
        clauses.push("j.scimago_rank <= ?".to_string());
        values.push(SqlValue::Real(value));
    }
    if let Some(year) = params.year {
        clauses.push(
            "EXISTS (SELECT 1 FROM issues i WHERE i.journal_id = j.journal_id AND i.publication_year = ?)"
                .to_string(),
        );
        values.push(SqlValue::Integer(year));
    }
    let where_sql = where_sql(&clauses);
    let order_sql = sort_sql(
        params.sort.as_deref().unwrap_or("scimago_rank:desc"),
        &[
            ("journal_id", "j.journal_id"),
            ("title", "j.title"),
            ("issn", "j.issn"),
            ("eissn", "j.eissn"),
            ("scimago_rank", "j.scimago_rank"),
            ("available", "j.available"),
            ("has_articles", "j.has_articles"),
        ],
    )?;
    let total: i64 = connection.query_row(
        &format!(
            "SELECT COUNT(*) FROM journals j LEFT JOIN journal_meta m ON j.journal_id = m.journal_id {where_sql}"
        ),
        params_from_iter(values.iter()),
        |row| row.get(0),
    )?;
    let mut page_values = values.clone();
    page_values.push(SqlValue::Integer(params.limit));
    page_values.push(SqlValue::Integer(params.offset));
    let mut statement = connection.prepare(&format!(
        "SELECT j.journal_id, j.library_id, j.platform_journal_id, j.title, j.issn, j.eissn, \
         j.scimago_rank, j.cover_url, j.available, j.toc_data_approved_and_live, j.has_articles, \
         m.source_csv, m.area, m.csv_title, m.csv_issn, m.csv_library \
         FROM journals j LEFT JOIN journal_meta m ON j.journal_id = m.journal_id \
         {where_sql} {order_sql} LIMIT ? OFFSET ?"
    ))?;
    let rows = statement.query_map(params_from_iter(page_values.iter()), journal_from_row)?;
    Ok(JournalPage {
        items: collect_rows(rows)?,
        page: page_meta(Some(total), params.limit, params.offset, None, None),
    })
}

/// Get one journal.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
/// * `journal_id` - Journal identifier.
///
/// # Returns
///
/// Journal record.
pub fn get_journal(
    config: &StorageConfig,
    db_name: Option<&str>,
    journal_id: i64,
) -> Result<JournalRecord, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    connection
        .query_row(
            "SELECT j.journal_id, j.library_id, j.platform_journal_id, j.title, j.issn, j.eissn, \
             j.scimago_rank, j.cover_url, j.available, j.toc_data_approved_and_live, j.has_articles, \
             m.source_csv, m.area, m.csv_title, m.csv_issn, m.csv_library \
             FROM journals j LEFT JOIN journal_meta m ON j.journal_id = m.journal_id \
             WHERE j.journal_id = ?",
            [journal_id],
            journal_from_row,
        )
        .optional()?
        .ok_or(IndexRepositoryError::NotFound("Journal not found"))
}

/// List issues with filters.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
/// * `params` - Issue filters.
///
/// # Returns
///
/// Paginated issue response.
pub fn list_issues(
    config: &StorageConfig,
    db_name: Option<&str>,
    params: &IssueListParams,
) -> Result<IssuePage, IndexRepositoryError> {
    validate_limit_offset(params.limit, params.offset)?;
    let connection = open_index_connection(config, db_name)?;
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    push_optional_int_filter(
        &mut clauses,
        &mut values,
        "i.journal_id = ?",
        params.journal_id,
    );
    push_optional_int_filter(
        &mut clauses,
        &mut values,
        "i.publication_year = ?",
        params.year,
    );
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "i.is_valid_issue = ?",
        params.is_valid_issue,
    );
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "i.suppressed = ?",
        params.suppressed,
    );
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "i.embargoed = ?",
        params.embargoed,
    );
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "i.within_subscription = ?",
        params.within_subscription,
    );
    let where_sql = where_sql(&clauses);
    let order_sql = sort_sql(
        params.sort.as_deref().unwrap_or("publication_year:desc"),
        &[
            ("issue_id", "i.issue_id"),
            ("publication_year", "i.publication_year"),
            ("title", "i.title"),
            ("date", "i.date"),
            ("volume", "i.volume"),
            ("number", "i.number"),
        ],
    )?;
    let total: i64 = connection.query_row(
        &format!("SELECT COUNT(*) FROM issues i {where_sql}"),
        params_from_iter(values.iter()),
        |row| row.get(0),
    )?;
    let mut page_values = values.clone();
    page_values.push(SqlValue::Integer(params.limit));
    page_values.push(SqlValue::Integer(params.offset));
    let mut statement = connection.prepare(&format!(
        "SELECT i.issue_id, i.journal_id, i.publication_year, i.title, i.volume, i.number, \
         i.date, i.is_valid_issue, i.suppressed, i.embargoed, i.within_subscription \
         FROM issues i {where_sql} {order_sql} LIMIT ? OFFSET ?"
    ))?;
    let rows = statement.query_map(params_from_iter(page_values.iter()), issue_from_row)?;
    Ok(IssuePage {
        items: collect_rows(rows)?,
        page: page_meta(Some(total), params.limit, params.offset, None, None),
    })
}

/// Get one issue.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
/// * `issue_id` - Issue identifier.
///
/// # Returns
///
/// Issue record.
pub fn get_issue(
    config: &StorageConfig,
    db_name: Option<&str>,
    issue_id: i64,
) -> Result<IssueRecord, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    connection
        .query_row(
            "SELECT issue_id, journal_id, publication_year, title, volume, number, date, \
             is_valid_issue, suppressed, embargoed, within_subscription \
             FROM issues WHERE issue_id = ?",
            [issue_id],
            issue_from_row,
        )
        .optional()?
        .ok_or(IndexRepositoryError::NotFound("Issue not found"))
}

/// List articles with filters.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
/// * `params` - Article filters.
///
/// # Returns
///
/// Paginated article response.
pub fn list_articles(
    config: &StorageConfig,
    db_name: Option<&str>,
    params: &ArticleListParams,
) -> Result<ArticlePage, IndexRepositoryError> {
    validate_limit_offset(params.limit, params.offset)?;
    let connection = open_index_connection(config, db_name)?;
    let use_simple_query = should_use_simple_query(
        params.q.as_deref(),
        article_search_uses_simple(&connection)?,
    );
    if is_article_listing_ready(&connection) {
        list_articles_from_listing(&connection, params, use_simple_query)
    } else {
        list_articles_from_articles(&connection, params, use_simple_query)
    }
}

/// Get one article.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
/// * `article_id` - Article identifier.
///
/// # Returns
///
/// Article record.
pub fn get_article(
    config: &StorageConfig,
    db_name: Option<&str>,
    article_id: i64,
) -> Result<ArticleRecord, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    get_article_from_connection(&connection, article_id)?
        .ok_or(IndexRepositoryError::NotFound("Article not found"))
}

/// Return article access capabilities.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
/// * `article_id` - Article identifier.
/// * `user_id` - Current user identifier.
///
/// # Returns
///
/// Article access response.
pub fn get_article_access(
    config: &StorageConfig,
    db_name: Option<&str>,
    article_id: i64,
    user_id: UserId,
) -> Result<ArticleAccessResponse, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    let row = get_article_access_row(&connection, article_id)?
        .ok_or(IndexRepositoryError::NotFound("Article not found"))?;
    Ok(article_access_response(&row, config, user_id))
}

/// Return the full-text redirect URL for an article.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
/// * `article_id` - Article identifier.
/// * `user_id` - Current user identifier.
///
/// # Returns
///
/// Redirect URL.
pub fn article_fulltext_redirect_url(
    config: &StorageConfig,
    db_name: Option<&str>,
    article_id: i64,
    user_id: UserId,
) -> Result<String, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    let row = get_article_access_row(&connection, article_id)?
        .ok_or(IndexRepositoryError::NotFound("Article not found"))?;
    if is_cnki_article_row(&row) {
        if is_cnki_session_active(config, user_id)? {
            return Err(IndexRepositoryError::NotFound(
                "CNKI full-text download is not migrated yet",
            ));
        }
        if let Some(permalink) = nonempty(row.permalink.as_deref()) {
            return Ok(with_cnki_chinese_language(permalink));
        }
        return Err(IndexRepositoryError::NotFound("Full text not available"));
    }
    if let Some(full_text_file) = nonempty(row.full_text_file.as_deref()) {
        if !is_cnki_protected_fulltext_url(full_text_file) {
            return Ok(with_cnki_chinese_language(full_text_file));
        }
    }
    if let Some(permalink) = nonempty(row.permalink.as_deref()) {
        return Ok(with_cnki_chinese_language(permalink));
    }
    if let Some(doi) = nonempty(row.doi.as_deref()) {
        return Ok(format!("https://doi.org/{doi}"));
    }
    Err(IndexRepositoryError::NotFound("Full text not available"))
}

/// Return the full-text route target for an article.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
/// * `article_id` - Article identifier.
/// * `user_id` - Current user identifier.
///
/// # Returns
///
/// Redirect or PDF response target.
pub fn article_fulltext_target(
    config: &StorageConfig,
    db_name: Option<&str>,
    article_id: i64,
    user_id: UserId,
) -> Result<ArticleFulltextTarget, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    let row = get_article_access_row(&connection, article_id)?
        .ok_or(IndexRepositoryError::NotFound("Article not found"))?;
    if is_cnki_article_row(&row) && is_cnki_session_active(config, user_id)? {
        return cnki_replay_pdf_target(config, user_id);
    }
    Ok(ArticleFulltextTarget::Redirect(
        article_fulltext_redirect_url(config, db_name, article_id, user_id)?,
    ))
}

/// Return weekly updates grouped by database and journal.
///
/// # Arguments
///
/// * `config` - Storage paths.
///
/// # Returns
///
/// Weekly updates response.
pub fn get_weekly_updates(
    config: &StorageConfig,
) -> Result<WeeklyUpdatesResponse, IndexRepositoryError> {
    let now = current_utc_iso_text();
    let manifests = load_weekly_manifests(config)?;
    if manifests.is_empty() {
        let window_start = iso_minus_days(&now, 7).unwrap_or_else(|| now.clone());
        return Ok(WeeklyUpdatesResponse {
            generated_at: now.clone(),
            window_start,
            window_end: now,
            databases: Vec::new(),
        });
    }
    let window_end = manifests
        .iter()
        .map(|manifest| manifest.generated_at.clone())
        .max()
        .unwrap_or_else(|| now.clone());
    let window_start = iso_minus_days(&window_end, 7).unwrap_or_else(|| window_end.clone());
    let mut by_db: HashMap<String, WeeklyBucket> = HashMap::new();
    for manifest in manifests {
        let bucket = by_db
            .entry(manifest.db_name.clone())
            .or_insert(WeeklyBucket {
                generated_at: manifest.generated_at.clone(),
                run_id: manifest.run_id.clone(),
                article_ids: Vec::new(),
                seen: HashSet::new(),
            });
        for article_id in manifest.article_ids {
            if bucket.seen.insert(article_id) {
                bucket.article_ids.push(article_id);
            }
        }
    }
    let mut databases = Vec::new();
    for (db_name, bucket) in by_db {
        let db_path = config.index_dir().join(&db_name);
        if !db_path.exists() || bucket.article_ids.is_empty() {
            continue;
        }
        let connection = open_sqlite_connection(db_path)?;
        let articles = fetch_weekly_articles(&connection, &bucket.article_ids)?;
        if articles.is_empty() {
            continue;
        }
        databases.push(WeeklyDatabaseUpdate {
            db_name,
            run_id: bucket.run_id,
            generated_at: bucket.generated_at,
            new_article_count: articles.len(),
            journals: group_weekly_articles_by_journal(articles),
        });
    }
    databases.sort_by(|left, right| {
        right
            .generated_at
            .cmp(&left.generated_at)
            .then_with(|| right.db_name.cmp(&left.db_name))
    });
    Ok(WeeklyUpdatesResponse {
        generated_at: now,
        window_start,
        window_end,
        databases,
    })
}

fn list_articles_from_listing(
    connection: &Connection,
    params: &ArticleListParams,
    use_simple_query: bool,
) -> Result<ArticlePage, IndexRepositoryError> {
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    push_int_list_filter(
        &mut clauses,
        &mut values,
        "l.journal_id",
        &params.journal_id,
    );
    push_optional_int_filter(&mut clauses, &mut values, "l.issue_id = ?", params.issue_id);
    push_string_list_filter(&mut clauses, &mut values, "l.area", &params.area);
    push_optional_bool_filter(&mut clauses, &mut values, "l.in_press = ?", params.in_press);
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "l.open_access = ?",
        params.open_access,
    );
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "l.suppressed = ?",
        params.suppressed,
    );
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "l.within_library_holdings = ?",
        params.within_library_holdings,
    );
    push_optional_text_filter(&mut clauses, &mut values, "l.date >= ?", &params.date_from);
    push_optional_text_filter(&mut clauses, &mut values, "l.date <= ?", &params.date_to);
    push_optional_text_filter(&mut clauses, &mut values, "l.doi = ?", &params.doi);
    push_optional_text_filter(&mut clauses, &mut values, "l.pmid = ?", &params.pmid);
    push_optional_int_filter(
        &mut clauses,
        &mut values,
        "l.publication_year = ?",
        params.year,
    );
    push_fts_filter(
        &mut clauses,
        &mut values,
        "l.article_id",
        &params.q,
        use_simple_query,
    );
    let direction = article_sort_direction(params.sort.as_deref().unwrap_or("date:desc"))?;
    push_cursor_filter(
        &mut clauses,
        &mut values,
        "l",
        direction,
        params.cursor.as_deref(),
    )?;
    let where_sql = where_sql(&clauses);
    let total = article_total(
        connection,
        params.include_total,
        "article_listing l",
        "",
        &where_sql,
        &values,
    )?;
    let id_rows = article_id_rows(
        connection,
        ArticleIdQuery {
            table_sql: "article_listing l",
            join_sql: "",
            where_sql: &where_sql,
            alias: "l",
            direction,
            values: &values,
            params,
        },
    )?;
    article_page_from_ids(connection, id_rows, total, params)
}

fn list_articles_from_articles(
    connection: &Connection,
    params: &ArticleListParams,
    use_simple_query: bool,
) -> Result<ArticlePage, IndexRepositoryError> {
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    push_int_list_filter(
        &mut clauses,
        &mut values,
        "a.journal_id",
        &params.journal_id,
    );
    push_optional_int_filter(&mut clauses, &mut values, "a.issue_id = ?", params.issue_id);
    push_string_list_filter(&mut clauses, &mut values, "m.area", &params.area);
    push_optional_bool_filter(&mut clauses, &mut values, "a.in_press = ?", params.in_press);
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "a.open_access = ?",
        params.open_access,
    );
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "a.suppressed = ?",
        params.suppressed,
    );
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "a.within_library_holdings = ?",
        params.within_library_holdings,
    );
    push_optional_text_filter(&mut clauses, &mut values, "a.date >= ?", &params.date_from);
    push_optional_text_filter(&mut clauses, &mut values, "a.date <= ?", &params.date_to);
    push_optional_text_filter(&mut clauses, &mut values, "a.doi = ?", &params.doi);
    push_optional_text_filter(&mut clauses, &mut values, "a.pmid = ?", &params.pmid);
    push_optional_int_filter(
        &mut clauses,
        &mut values,
        "i.publication_year = ?",
        params.year,
    );
    if let Some(query) = nonempty(params.q.as_deref()) {
        let matcher = if use_simple_query {
            "simple_query(?)"
        } else {
            "?"
        };
        clauses.push(format!("article_search MATCH {matcher}"));
        values.push(SqlValue::Text(query.to_string()));
    }
    let mut joins = Vec::new();
    if params.year.is_some() {
        joins.push("JOIN issues i ON i.issue_id = a.issue_id");
    }
    if !params.area.is_empty() {
        joins.push("JOIN journal_meta m ON m.journal_id = a.journal_id");
    }
    if nonempty(params.q.as_deref()).is_some() {
        joins.push("JOIN article_search ON article_search.article_id = a.article_id");
    }
    let join_sql = joins.join(" ");
    let direction = article_sort_direction(params.sort.as_deref().unwrap_or("date:desc"))?;
    push_cursor_filter(
        &mut clauses,
        &mut values,
        "a",
        direction,
        params.cursor.as_deref(),
    )?;
    let where_sql = where_sql(&clauses);
    let total = article_total(
        connection,
        params.include_total,
        "articles a",
        &join_sql,
        &where_sql,
        &values,
    )?;
    let id_rows = article_id_rows(
        connection,
        ArticleIdQuery {
            table_sql: "articles a",
            join_sql: &join_sql,
            where_sql: &where_sql,
            alias: "a",
            direction,
            values: &values,
            params,
        },
    )?;
    article_page_from_ids(connection, id_rows, total, params)
}

fn article_total(
    connection: &Connection,
    include_total: bool,
    table_sql: &str,
    join_sql: &str,
    where_sql: &str,
    values: &[SqlValue],
) -> Result<Option<i64>, IndexRepositoryError> {
    if !include_total {
        return Ok(None);
    }
    Ok(Some(connection.query_row(
        &format!("SELECT COUNT(*) FROM {table_sql} {join_sql} {where_sql}"),
        params_from_iter(values.iter()),
        |row| row.get(0),
    )?))
}

fn article_id_rows(
    connection: &Connection,
    query: ArticleIdQuery<'_>,
) -> Result<Vec<(i64, Option<String>)>, IndexRepositoryError> {
    let mut page_values = query.values.to_vec();
    page_values.push(SqlValue::Integer(query.params.limit));
    let pagination_sql = if query.params.cursor.is_none() {
        page_values.push(SqlValue::Integer(query.params.offset));
        "LIMIT ? OFFSET ?"
    } else {
        "LIMIT ?"
    };
    let order_direction = query.direction.sql();
    let mut statement = connection.prepare(&format!(
        "SELECT {alias}.article_id, {alias}.date FROM {table_sql} {join_sql} {where_sql} \
         ORDER BY {alias}.date {order_direction}, {alias}.article_id {order_direction} {pagination_sql}",
        alias = query.alias,
        table_sql = query.table_sql,
        join_sql = query.join_sql,
        where_sql = query.where_sql,
    ))?;
    let rows = statement.query_map(params_from_iter(page_values.iter()), |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    collect_rows(rows)
}

fn article_page_from_ids(
    connection: &Connection,
    id_rows: Vec<(i64, Option<String>)>,
    total: Option<i64>,
    params: &ArticleListParams,
) -> Result<ArticlePage, IndexRepositoryError> {
    let has_more = id_rows.len() as i64 == params.limit;
    let next_cursor = if has_more {
        id_rows
            .last()
            .and_then(|(article_id, date)| date.as_ref().map(|date| format!("{date}|{article_id}")))
    } else {
        None
    };
    let article_ids = id_rows
        .iter()
        .map(|(article_id, _)| *article_id)
        .collect::<Vec<_>>();
    let items = fetch_articles_by_ids(connection, &article_ids)?;
    Ok(ArticlePage {
        items,
        page: page_meta(
            total,
            params.limit,
            params.offset,
            next_cursor.clone(),
            Some(has_more && next_cursor.is_some()),
        ),
    })
}

fn fetch_articles_by_ids(
    connection: &Connection,
    article_ids: &[i64],
) -> Result<Vec<ArticleRecord>, IndexRepositoryError> {
    if article_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = placeholders(article_ids.len());
    let values = article_ids
        .iter()
        .copied()
        .map(SqlValue::Integer)
        .collect::<Vec<_>>();
    let mut statement = connection.prepare(&format!(
        "SELECT a.article_id, a.journal_id, a.issue_id, a.title, a.date, a.authors, \
         a.start_page, a.end_page, a.abstract, a.doi, a.pmid, a.permalink, a.suppressed, \
         a.in_press, a.open_access, a.platform_id, a.retraction_doi, \
         a.within_library_holdings, a.content_location, a.full_text_file, \
         j.title AS journal_title, i.volume, i.number \
         FROM articles a LEFT JOIN issues i ON i.issue_id = a.issue_id \
         JOIN journals j ON j.journal_id = a.journal_id \
         WHERE a.article_id IN ({placeholders})"
    ))?;
    let rows = statement.query_map(params_from_iter(values.iter()), article_from_row)?;
    let mut by_id = collect_rows(rows)?
        .into_iter()
        .map(|article: ArticleRecord| (article.article_id.value(), article))
        .collect::<HashMap<_, _>>();
    Ok(article_ids
        .iter()
        .filter_map(|article_id| by_id.remove(article_id))
        .collect())
}

fn get_article_from_connection(
    connection: &Connection,
    article_id: i64,
) -> Result<Option<ArticleRecord>, IndexRepositoryError> {
    let rows = fetch_articles_by_ids(connection, &[article_id])?;
    Ok(rows.into_iter().next())
}

fn get_article_access_row(
    connection: &Connection,
    article_id: i64,
) -> Result<Option<ArticleAccessRow>, IndexRepositoryError> {
    connection
        .query_row(
            "SELECT a.doi, a.full_text_file, a.permalink, j.library_id \
             FROM articles a \
             JOIN journals j ON j.journal_id = a.journal_id WHERE a.article_id = ?",
            [article_id],
            |row| {
                Ok(ArticleAccessRow {
                    doi: row.get(0)?,
                    full_text_file: row.get(1)?,
                    permalink: row.get(2)?,
                    library_id: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(IndexRepositoryError::from)
}

fn article_access_response(
    row: &ArticleAccessRow,
    config: &StorageConfig,
    user_id: UserId,
) -> ArticleAccessResponse {
    ArticleAccessResponse {
        detail: detail_access_action(row),
        fulltext: fulltext_access_action(row, config, user_id),
    }
}

fn detail_access_action(row: &ArticleAccessRow) -> ArticleAccessAction {
    if let Some(permalink) = nonempty(row.permalink.as_deref()) {
        return ArticleAccessAction {
            available: true,
            label: if is_cnki_article_row(row) {
                CNKI_DETAIL_LABEL.to_string()
            } else {
                DETAIL_LABEL.to_string()
            },
            provider: Some(DETAIL_PROVIDER.to_string()),
            url: Some(with_cnki_chinese_language(permalink)),
            requires_login: false,
            message: None,
        };
    }
    if let Some(doi) = nonempty(row.doi.as_deref()) {
        return ArticleAccessAction {
            available: true,
            label: DETAIL_LABEL.to_string(),
            provider: Some(DOI_PROVIDER.to_string()),
            url: Some(format!("https://doi.org/{doi}")),
            requires_login: false,
            message: None,
        };
    }
    ArticleAccessAction {
        available: false,
        label: DETAIL_LABEL.to_string(),
        provider: None,
        url: None,
        requires_login: false,
        message: Some("Article detail is not available".to_string()),
    }
}

fn fulltext_access_action(
    row: &ArticleAccessRow,
    config: &StorageConfig,
    user_id: UserId,
) -> ArticleAccessAction {
    if let Some(full_text_file) = nonempty(row.full_text_file.as_deref()) {
        if !is_cnki_protected_fulltext_url(full_text_file) {
            return ArticleAccessAction {
                available: true,
                label: FULLTEXT_LABEL.to_string(),
                provider: Some(STORED_FULLTEXT_PROVIDER.to_string()),
                url: None,
                requires_login: false,
                message: None,
            };
        }
    }
    if is_cnki_article_row(row) {
        let is_active = is_cnki_session_active(config, user_id).unwrap_or(false);
        return ArticleAccessAction {
            available: is_active,
            label: FULLTEXT_LABEL.to_string(),
            provider: Some(ZJLIB_CNKI_PROVIDER.to_string()),
            url: None,
            requires_login: !is_active,
            message: (!is_active).then(|| "需要先在设置中完成浙江图书馆扫码登录".to_string()),
        };
    }
    ArticleAccessAction {
        available: false,
        label: FULLTEXT_LABEL.to_string(),
        provider: None,
        url: None,
        requires_login: false,
        message: Some("Full text is not available".to_string()),
    }
}

fn fetch_weekly_articles(
    connection: &Connection,
    article_ids: &[i64],
) -> Result<Vec<WeeklyArticleRecord>, IndexRepositoryError> {
    if article_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut by_id = HashMap::new();
    for chunk in article_ids.chunks(500) {
        let placeholders = placeholders(chunk.len());
        let values = chunk
            .iter()
            .copied()
            .map(SqlValue::Integer)
            .collect::<Vec<_>>();
        let mut statement = connection.prepare(&format!(
            "SELECT a.article_id, a.journal_id, a.issue_id, a.title, a.date, a.authors, \
             a.abstract, a.doi, a.platform_id, a.permalink, a.full_text_file, a.open_access, \
             a.in_press, j.title AS journal_title, i.volume, i.number \
             FROM articles a LEFT JOIN issues i ON i.issue_id = a.issue_id \
             JOIN journals j ON j.journal_id = a.journal_id \
             WHERE a.article_id IN ({placeholders})"
        ))?;
        let rows = statement.query_map(params_from_iter(values.iter()), weekly_article_from_row)?;
        by_id.extend(
            collect_rows(rows)?
                .into_iter()
                .map(|article: WeeklyArticleRecord| (article.article_id.value(), article)),
        );
    }
    Ok(article_ids
        .iter()
        .filter_map(|article_id| by_id.remove(article_id))
        .collect())
}

fn group_weekly_articles_by_journal(
    articles: Vec<WeeklyArticleRecord>,
) -> Vec<WeeklyJournalUpdate> {
    let mut by_journal: HashMap<i64, Vec<WeeklyArticleRecord>> = HashMap::new();
    for article in articles {
        by_journal
            .entry(article.journal_id.value())
            .or_default()
            .push(article);
    }
    let mut journals = by_journal
        .into_iter()
        .map(|(journal_id, articles)| {
            let journal_title = articles
                .first()
                .and_then(|article| article.journal_title.clone());
            WeeklyJournalUpdate {
                journal_id: JournalId(journal_id),
                journal_title,
                new_article_count: articles.len(),
                articles,
            }
        })
        .collect::<Vec<_>>();
    journals.sort_by(|left, right| {
        right
            .new_article_count
            .cmp(&left.new_article_count)
            .then_with(|| {
                left.journal_title
                    .clone()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .cmp(
                        &right
                            .journal_title
                            .clone()
                            .unwrap_or_default()
                            .to_ascii_lowercase(),
                    )
            })
            .then_with(|| left.journal_id.value().cmp(&right.journal_id.value()))
    });
    journals
}

fn load_weekly_manifests(
    config: &StorageConfig,
) -> Result<Vec<WeeklyManifest>, IndexRepositoryError> {
    let push_state_dir = config.project_root().join("data").join("push_state");
    if !push_state_dir.exists() {
        return Ok(Vec::new());
    }
    let mut manifests = Vec::new();
    for entry in fs::read_dir(push_state_dir)? {
        let path = entry?.path();
        if !path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.ends_with(".changes.json"))
        {
            continue;
        }
        let payload = serde_json::from_str::<JsonValue>(&fs::read_to_string(path)?)?;
        if let Some(manifest) = parse_weekly_manifest(&payload) {
            manifests.push(manifest);
        }
    }
    manifests.sort_by(|left, right| {
        right
            .generated_at
            .cmp(&left.generated_at)
            .then_with(|| right.db_name.cmp(&left.db_name))
    });
    Ok(manifests)
}

fn parse_weekly_manifest(payload: &JsonValue) -> Option<WeeklyManifest> {
    let db_name = payload
        .get("db_name")
        .and_then(JsonValue::as_str)
        .or_else(|| payload.get("db_path").and_then(JsonValue::as_str))
        .and_then(normalize_db_name)?;
    let mut seen = HashSet::new();
    let mut article_ids = Vec::new();
    for item in payload
        .get("notifiable_article_ids")
        .and_then(JsonValue::as_array)?
        .iter()
        .filter_map(JsonValue::as_i64)
    {
        if seen.insert(item) {
            article_ids.push(item);
        }
    }
    if article_ids.is_empty() {
        return None;
    }
    let generated_at = payload
        .get("generated_at")
        .and_then(JsonValue::as_str)
        .or_else(|| payload.get("run_id").and_then(JsonValue::as_str))
        .and_then(normalize_iso_datetime)
        .unwrap_or_else(current_utc_iso_text);
    let run_id = payload
        .get("run_id")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    Some(WeeklyManifest {
        db_name,
        run_id,
        generated_at,
        article_ids,
    })
}

fn open_index_connection(
    config: &StorageConfig,
    db_name: Option<&str>,
) -> Result<Connection, IndexRepositoryError> {
    let db_path = config.resolve_index_db_path(db_name)?;
    let connection = open_sqlite_connection(db_path)?;
    let extension_path = resolve_simple_tokenizer_path(config);
    try_load_extension(&connection, extension_path.as_deref())?;
    ensure_journal_platform_id_column(&connection)?;
    Ok(connection)
}

fn ensure_journal_platform_id_column(connection: &Connection) -> Result<(), IndexRepositoryError> {
    let columns = table_columns(connection, "journals")?;
    if !columns.iter().any(|column| column == "platform_journal_id") {
        connection.execute(
            "ALTER TABLE journals ADD COLUMN platform_journal_id TEXT",
            [],
        )?;
    }
    Ok(())
}

fn is_article_listing_ready(connection: &Connection) -> bool {
    let status = connection
        .query_row("SELECT status FROM listing_state WHERE id = 1", [], |row| {
            row.get::<_, String>(0)
        })
        .optional();
    if !matches!(status, Ok(Some(value)) if value == "ready") {
        return false;
    }
    connection
        .query_row("SELECT 1 FROM article_listing LIMIT 1", [], |row| {
            row.get::<_, i64>(0)
        })
        .is_ok()
}

fn table_columns(
    connection: &Connection,
    table_name: &str,
) -> Result<Vec<String>, IndexRepositoryError> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table_name})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    collect_rows(rows)
}

fn value_count_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ValueCount> {
    Ok(ValueCount {
        value: row.get(0)?,
        count: row.get(1)?,
    })
}

fn journal_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<JournalRecord> {
    Ok(JournalRecord {
        journal_id: JournalId(row.get(0)?),
        library_id: row.get(1)?,
        platform_journal_id: row.get(2)?,
        title: row.get(3)?,
        issn: row.get(4)?,
        eissn: row.get(5)?,
        scimago_rank: row.get(6)?,
        cover_url: row.get(7)?,
        available: row.get(8)?,
        toc_data_approved_and_live: row.get(9)?,
        has_articles: row.get(10)?,
        source_csv: row.get(11)?,
        area: row.get(12)?,
        csv_title: row.get(13)?,
        csv_issn: row.get(14)?,
        csv_library: row.get(15)?,
    })
}

fn issue_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<IssueRecord> {
    Ok(IssueRecord {
        issue_id: row.get(0)?,
        journal_id: JournalId(row.get(1)?),
        publication_year: row.get(2)?,
        title: row.get(3)?,
        volume: row.get(4)?,
        number: row.get(5)?,
        date: row.get(6)?,
        is_valid_issue: row.get(7)?,
        suppressed: row.get(8)?,
        embargoed: row.get(9)?,
        within_subscription: row.get(10)?,
    })
}

fn article_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArticleRecord> {
    Ok(ArticleRecord {
        article_id: ArticleId(row.get(0)?),
        journal_id: JournalId(row.get(1)?),
        issue_id: row.get(2)?,
        title: row.get(3)?,
        date: row.get(4)?,
        authors: row.get(5)?,
        start_page: row.get(6)?,
        end_page: row.get(7)?,
        abstract_text: row.get(8)?,
        doi: row.get(9)?,
        pmid: row.get(10)?,
        permalink: row.get(11)?,
        suppressed: row.get(12)?,
        in_press: row.get(13)?,
        open_access: row.get(14)?,
        platform_id: row.get(15)?,
        retraction_doi: row.get(16)?,
        within_library_holdings: row.get(17)?,
        content_location: row.get(18)?,
        full_text_file: row.get(19)?,
        journal_title: row.get(20)?,
        volume: row.get(21)?,
        number: row.get(22)?,
    })
}

fn weekly_article_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WeeklyArticleRecord> {
    Ok(WeeklyArticleRecord {
        article_id: ArticleId(row.get(0)?),
        journal_id: JournalId(row.get(1)?),
        issue_id: row.get(2)?,
        title: row.get(3)?,
        date: row.get(4)?,
        authors: row.get(5)?,
        abstract_text: row.get(6)?,
        doi: row.get(7)?,
        platform_id: row.get(8)?,
        permalink: row.get(9)?,
        full_text_file: row.get(10)?,
        open_access: row.get(11)?,
        in_press: row.get(12)?,
        journal_title: row.get(13)?,
        volume: row.get(14)?,
        number: row.get(15)?,
    })
}

fn page_meta(
    total: Option<i64>,
    limit: i64,
    offset: i64,
    next_cursor: Option<String>,
    has_more: Option<bool>,
) -> PageMeta {
    PageMeta {
        total,
        limit,
        offset,
        next_cursor,
        has_more,
    }
}

fn sort_sql(sort: &str, allowed: &[(&str, &str)]) -> Result<String, IndexRepositoryError> {
    let specs = sort_specs(sort, allowed)?;
    if specs.is_empty() {
        return Ok(String::new());
    }
    Ok(format!(
        "ORDER BY {}",
        specs
            .into_iter()
            .map(|spec| format!("{} {}", spec.column, spec.direction.sql()))
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

fn sort_specs(sort: &str, allowed: &[(&str, &str)]) -> Result<Vec<SortSpec>, IndexRepositoryError> {
    let mut specs = Vec::new();
    for raw_part in sort.split(',') {
        let part = raw_part.trim();
        if part.is_empty() {
            continue;
        }
        let (field, direction) = if let Some(field) = part.strip_prefix('-') {
            (field.trim(), SortDirection::Desc)
        } else if let Some((field, raw_direction)) = part.split_once(':') {
            let direction = if raw_direction.trim().eq_ignore_ascii_case("desc") {
                SortDirection::Desc
            } else {
                SortDirection::Asc
            };
            (field.trim(), direction)
        } else {
            (part, SortDirection::Asc)
        };
        let Some((_, column)) = allowed.iter().find(|(name, _)| *name == field) else {
            return Err(IndexRepositoryError::UnsupportedSortField(
                field.to_string(),
            ));
        };
        specs.push(SortSpec {
            column: column.to_string(),
            direction,
        });
    }
    Ok(specs)
}

fn article_sort_direction(sort: &str) -> Result<SortDirection, IndexRepositoryError> {
    let specs = sort_specs(sort, &[("date", "date")])?;
    if specs.len() != 1 {
        return Err(IndexRepositoryError::UnsupportedArticleSort);
    }
    Ok(specs[0].direction)
}

fn push_cursor_filter(
    clauses: &mut Vec<String>,
    values: &mut Vec<SqlValue>,
    alias: &str,
    direction: SortDirection,
    cursor: Option<&str>,
) -> Result<(), IndexRepositoryError> {
    let Some(cursor) = cursor else {
        return Ok(());
    };
    let (date, article_id) = parse_article_cursor(cursor)?;
    let operator = if direction == SortDirection::Desc {
        "<"
    } else {
        ">"
    };
    clauses.push(format!(
        "({alias}.date {operator} ? OR ({alias}.date = ? AND {alias}.article_id {operator} ?))"
    ));
    values.push(SqlValue::Text(date.clone()));
    values.push(SqlValue::Text(date));
    values.push(SqlValue::Integer(article_id));
    Ok(())
}

fn parse_article_cursor(cursor: &str) -> Result<(String, i64), IndexRepositoryError> {
    let Some((date, article_id)) = cursor.split_once('|') else {
        return Err(IndexRepositoryError::InvalidCursor);
    };
    if date.is_empty() {
        return Err(IndexRepositoryError::InvalidCursor);
    }
    let article_id = article_id
        .parse::<i64>()
        .map_err(|_| IndexRepositoryError::InvalidCursor)?;
    Ok((date.to_string(), article_id))
}

fn push_fts_filter(
    clauses: &mut Vec<String>,
    values: &mut Vec<SqlValue>,
    column: &str,
    q: &Option<String>,
    use_simple_query: bool,
) {
    if let Some(query) = nonempty(q.as_deref()) {
        let matcher = if use_simple_query {
            "simple_query(?)"
        } else {
            "?"
        };
        clauses.push(format!(
            "{column} IN (SELECT rowid FROM article_search WHERE article_search MATCH {matcher})"
        ));
        values.push(SqlValue::Text(query.to_string()));
    }
}

fn push_int_list_filter(
    clauses: &mut Vec<String>,
    values: &mut Vec<SqlValue>,
    column: &str,
    items: &[i64],
) {
    if items.is_empty() {
        return;
    }
    clauses.push(format!("{column} IN ({})", placeholders(items.len())));
    values.extend(items.iter().copied().map(SqlValue::Integer));
}

fn push_string_list_filter(
    clauses: &mut Vec<String>,
    values: &mut Vec<SqlValue>,
    column: &str,
    items: &[String],
) {
    if items.is_empty() {
        return;
    }
    clauses.push(format!("{column} IN ({})", placeholders(items.len())));
    values.extend(items.iter().cloned().map(SqlValue::Text));
}

fn push_optional_int_filter(
    clauses: &mut Vec<String>,
    values: &mut Vec<SqlValue>,
    clause: &str,
    value: Option<i64>,
) {
    if let Some(value) = value {
        clauses.push(clause.to_string());
        values.push(SqlValue::Integer(value));
    }
}

fn push_optional_bool_filter(
    clauses: &mut Vec<String>,
    values: &mut Vec<SqlValue>,
    clause: &str,
    value: Option<bool>,
) {
    if let Some(value) = value {
        clauses.push(clause.to_string());
        values.push(SqlValue::Integer(value as i64));
    }
}

fn push_optional_text_filter(
    clauses: &mut Vec<String>,
    values: &mut Vec<SqlValue>,
    clause: &str,
    value: &Option<String>,
) {
    if let Some(value) = nonempty(value.as_deref()) {
        clauses.push(clause.to_string());
        values.push(SqlValue::Text(value.to_string()));
    }
}

fn where_sql(clauses: &[String]) -> String {
    if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    }
}

fn placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(", ")
}

fn validate_limit_offset(limit: i64, offset: i64) -> Result<(), IndexRepositoryError> {
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(IndexRepositoryError::InvalidPagination(
            "limit must be between 1 and 200",
        ));
    }
    if offset < 0 {
        return Err(IndexRepositoryError::InvalidPagination(
            "offset must be greater than or equal to 0",
        ));
    }
    Ok(())
}

fn resolve_simple_tokenizer_path(config: &StorageConfig) -> Option<PathBuf> {
    if let Ok(value) = std::env::var(SIMPLE_TOKENIZER_ENV) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    let libs_dir = config.project_root().join("libs");
    if cfg!(windows) {
        Some(
            libs_dir
                .join("simple-windows")
                .join("libsimple-windows-x64")
                .join("simple.dll"),
        )
        .filter(|path| path.exists())
    } else if cfg!(target_os = "linux") {
        Some(
            libs_dir
                .join("simple-linux")
                .join("libsimple-linux-ubuntu-latest")
                .join("libsimple.so"),
        )
        .filter(|path| path.exists())
    } else {
        None
    }
}

fn article_search_uses_simple(connection: &Connection) -> Result<bool, IndexRepositoryError> {
    let sql = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name = 'article_search'",
            [],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten()
        .unwrap_or_default()
        .to_ascii_lowercase();
    Ok(sql.contains("tokenize") && sql.contains("simple"))
}

fn should_use_simple_query(q: Option<&str>, simple_enabled: bool) -> bool {
    simple_enabled && nonempty(q).is_some_and(|query| !contains_cjk(query))
}

fn contains_cjk(value: &str) -> bool {
    value
        .chars()
        .any(|character| ('\u{4e00}'..='\u{9fff}').contains(&character))
}

fn is_cnki_article_row(row: &ArticleAccessRow) -> bool {
    row.library_id.trim().eq_ignore_ascii_case(CNKI_SOURCE)
}

fn is_cnki_session_active(
    config: &StorageConfig,
    user_id: UserId,
) -> Result<bool, IndexRepositoryError> {
    if !config.auth_db_path().exists() {
        return Ok(false);
    }
    let status = get_cnki_session_status(config.auth_db_path(), user_id)?;
    Ok(status.status == "active")
}

fn cnki_replay_pdf_target(
    config: &StorageConfig,
    user_id: UserId,
) -> Result<ArticleFulltextTarget, IndexRepositoryError> {
    if std::env::var(CNKI_PDF_REPLAY_MODE_ENV)
        .ok()
        .is_some_and(|value| value.trim() == CNKI_PDF_REPLAY_MISMATCH)
    {
        return Err(IndexRepositoryError::NotFound(
            "No exact CNKI full-text match found",
        ));
    }
    let path = std::env::var(CNKI_PDF_REPLAY_PATH_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or(IndexRepositoryError::NotFound(
            "CNKI full-text download fixture is not configured",
        ))?;
    let content = fs::read(path)?;
    let filename = std::env::var(CNKI_PDF_REPLAY_FILENAME_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "cnki.pdf".to_string());
    crate::touch_cnki_session_used(config.auth_db_path(), user_id)
        .map_err(|_| IndexRepositoryError::NotFound("CNKI login is required"))?;
    Ok(ArticleFulltextTarget::Pdf {
        filename,
        content_type: "application/pdf".to_string(),
        content,
    })
}

fn is_cnki_protected_fulltext_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains(CNKI_PROTECTED_FULLTEXT_HOST) && lower.contains(CNKI_PROTECTED_FULLTEXT_PATH)
}

fn with_cnki_chinese_language(url: &str) -> String {
    if !url.to_ascii_lowercase().contains("oversea.cnki.net") || url.contains("language=chs") {
        return url.to_string();
    }
    if url.contains('?') {
        format!("{url}&language=chs")
    } else {
        format!("{url}?language=chs")
    }
}

fn fetch_candidates_for_issue_ids(
    index_db_path: impl AsRef<Path>,
    issue_ids: &[i64],
) -> Result<Vec<ArticleCandidateInfo>, IndexRepositoryError> {
    if issue_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = repeat_placeholders(issue_ids.len());
    let sql = format!(
        "SELECT a.article_id, a.journal_id, a.issue_id, a.title, a.abstract, a.date, \
         a.open_access, a.in_press, a.within_library_holdings, a.doi, a.full_text_file, \
         a.permalink, j.title AS journal_title \
         FROM articles a JOIN journals j ON j.journal_id = a.journal_id \
         WHERE a.issue_id IN ({placeholders}) AND COALESCE(a.suppressed, 0) = 0 \
         ORDER BY a.date DESC, a.article_id DESC"
    );
    let connection = Connection::open(index_db_path)?;
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(issue_ids.iter()), candidate_from_row)?;
    collect_rows(rows)
}

fn fetch_candidates_for_inpress_journal_ids(
    index_db_path: impl AsRef<Path>,
    journal_ids: &[i64],
) -> Result<Vec<ArticleCandidateInfo>, IndexRepositoryError> {
    if journal_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = repeat_placeholders(journal_ids.len());
    let sql = format!(
        "SELECT a.article_id, a.journal_id, a.issue_id, a.title, a.abstract, a.date, \
         a.open_access, a.in_press, a.within_library_holdings, a.doi, a.full_text_file, \
         a.permalink, j.title AS journal_title \
         FROM articles a JOIN journals j ON j.journal_id = a.journal_id \
         WHERE a.issue_id IS NULL AND COALESCE(a.in_press, 0) = 1 \
           AND a.journal_id IN ({placeholders}) AND COALESCE(a.suppressed, 0) = 0 \
         ORDER BY a.date DESC, a.article_id DESC"
    );
    let connection = Connection::open(index_db_path)?;
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(journal_ids.iter()), candidate_from_row)?;
    collect_rows(rows)
}

fn candidate_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArticleCandidateInfo> {
    Ok(ArticleCandidateInfo {
        article_id: row.get(0)?,
        journal_id: row.get(1)?,
        issue_id: row.get(2)?,
        title: nonempty_owned(row.get::<_, Option<String>>(3)?)
            .unwrap_or_else(|| "Untitled article".to_string()),
        abstract_text: nonempty_owned(row.get::<_, Option<String>>(4)?).unwrap_or_default(),
        date: nonempty_owned(row.get(5)?),
        open_access: row.get::<_, Option<i64>>(6)?.unwrap_or(0) != 0,
        in_press: row.get::<_, Option<i64>>(7)?.unwrap_or(0) != 0,
        within_library_holdings: row.get::<_, Option<i64>>(8)?.unwrap_or(0) != 0,
        doi: nonempty_owned(row.get(9)?),
        full_text_file: nonempty_owned(row.get(10)?),
        permalink: nonempty_owned(row.get(11)?),
        journal_title: nonempty_owned(row.get::<_, Option<String>>(12)?)
            .unwrap_or_else(|| "Unknown journal".to_string()),
    })
}

fn repeat_placeholders(count: usize) -> String {
    (0..count).map(|_| "?").collect::<Vec<_>>().join(", ")
}

fn build_issue_key(journal_id: i64, issue_id: i64) -> String {
    format!("{journal_id}:{issue_id}")
}

fn parse_issue_key(key: &str) -> Result<(i64, i64), IndexRepositoryError> {
    let (journal_id, issue_id) = key
        .split_once(':')
        .ok_or(IndexRepositoryError::InvalidCursor)?;
    let journal_id = journal_id
        .parse::<i64>()
        .map_err(|_| IndexRepositoryError::InvalidCursor)?;
    let issue_id = issue_id
        .parse::<i64>()
        .map_err(|_| IndexRepositoryError::InvalidCursor)?;
    Ok((journal_id, issue_id))
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn nonempty_owned(value: Option<String>) -> Option<String> {
    value
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
}

fn normalize_db_name(value: &str) -> Option<String> {
    let filename = Path::new(value.trim()).file_name()?.to_str()?;
    if filename.is_empty() {
        None
    } else if filename.ends_with(".sqlite") {
        Some(filename.to_string())
    } else {
        Some(format!("{filename}.sqlite"))
    }
}

fn current_utc_iso_text() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_secs() as i64;
    format_unix_seconds(seconds)
}

fn normalize_iso_datetime(value: &str) -> Option<String> {
    parse_iso_utc_seconds(value).map(format_unix_seconds)
}

fn iso_minus_days(value: &str, days: i64) -> Option<String> {
    parse_iso_utc_seconds(value).map(|seconds| format_unix_seconds(seconds - days * 86_400))
}

fn parse_iso_utc_seconds(value: &str) -> Option<i64> {
    let text = value
        .trim()
        .strip_suffix('Z')
        .unwrap_or_else(|| value.trim())
        .strip_suffix("+00:00")
        .unwrap_or_else(|| {
            value
                .trim()
                .strip_suffix('Z')
                .unwrap_or_else(|| value.trim())
        });
    let (date, time) = text.split_once('T')?;
    let mut date_parts = date.split('-');
    let year = date_parts.next()?.parse::<i64>().ok()?;
    let month = date_parts.next()?.parse::<i64>().ok()?;
    let day = date_parts.next()?.parse::<i64>().ok()?;
    if date_parts.next().is_some() {
        return None;
    }
    let mut time_parts = time.split(':');
    let hour = time_parts.next()?.parse::<i64>().ok()?;
    let minute = time_parts.next()?.parse::<i64>().ok()?;
    let second_text = time_parts.next()?;
    if time_parts.next().is_some() {
        return None;
    }
    let second = second_text
        .split_once('.')
        .map_or(second_text, |(seconds, _)| seconds)
        .parse::<i64>()
        .ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
    {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

fn format_unix_seconds(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = day_seconds / 3_600;
    let minute = (day_seconds % 3_600) / 60;
    let second = day_seconds % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    (year, month, day)
}

fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> Result<Vec<T>, IndexRepositoryError> {
    let mut items = Vec::new();
    for row in rows {
        items.push(row?);
    }
    Ok(items)
}

#[derive(Debug, Clone)]
struct SortSpec {
    column: String,
    direction: SortDirection,
}

#[derive(Debug, Clone, Copy)]
struct ArticleIdQuery<'a> {
    table_sql: &'a str,
    join_sql: &'a str,
    where_sql: &'a str,
    alias: &'a str,
    direction: SortDirection,
    values: &'a [SqlValue],
    params: &'a ArticleListParams,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortDirection {
    Asc,
    Desc,
}

impl SortDirection {
    fn sql(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

#[derive(Debug, Clone)]
struct ArticleAccessRow {
    doi: Option<String>,
    full_text_file: Option<String>,
    permalink: Option<String>,
    library_id: String,
}

#[derive(Debug, Clone)]
struct WeeklyManifest {
    db_name: String,
    run_id: Option<String>,
    generated_at: String,
    article_ids: Vec<i64>,
}

#[derive(Debug, Clone)]
struct WeeklyBucket {
    generated_at: String,
    run_id: Option<String>,
    article_ids: Vec<i64>,
    seen: HashSet<i64>,
}

#[cfg(test)]
mod tests {
    use std::fs;

    use ps_domain::UserId;
    use rusqlite::Connection;
    use serde_json::json;
    use tempfile::{tempdir, TempDir};

    use super::*;

    struct IndexFixture {
        _project_root: TempDir,
        config: StorageConfig,
        db_name: String,
    }

    impl IndexFixture {
        fn new(is_listing_ready: bool) -> Self {
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
                db_name,
            }
        }
    }

    #[test]
    fn journal_metadata_filters_cover_available_query_options() {
        let fixture = IndexFixture::new(true);

        assert_eq!(
            list_index_database_names(&fixture.config).expect("databases should list"),
            ["fixture.sqlite"]
        );
        assert_eq!(
            value_counts(list_areas(&fixture.config, Some(&fixture.db_name)).expect("areas")),
            [("Engineering".to_string(), 1), ("Medicine".to_string(), 2)]
        );
        assert_eq!(
            value_counts(list_sources(&fixture.config, Some(&fixture.db_name)).expect("sources")),
            [("scholarly".to_string(), 2), ("cnki".to_string(), 1)]
        );

        let years = list_years(&fixture.config, Some(&fixture.db_name)).expect("years");
        assert_eq!(years[0].year, 2026);
        assert_eq!(years[0].issue_count, 2);
        assert_eq!(years[0].journal_count, 2);
        assert_eq!(years[1].year, 2025);

        let options =
            list_journal_options(&fixture.config, Some(&fixture.db_name)).expect("options");
        assert_eq!(
            options
                .iter()
                .map(|option| option.title.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some("Alpha Journal"),
                Some("Beta CNKI"),
                Some("Gamma Hidden")
            ]
        );

        let page = list_journals(
            &fixture.config,
            Some(&fixture.db_name),
            &JournalListParams {
                area: Some("Medicine".to_string()),
                library_id: Some("scholarly".to_string()),
                available: Some(true),
                has_articles: Some(true),
                year: Some(2026),
                scimago_min: Some(5.0),
                scimago_max: Some(11.0),
                sort: Some("title:asc".to_string()),
                limit: 10,
                offset: 0,
            },
        )
        .expect("journal filters should apply");
        assert_eq!(page.page.total, Some(1));
        assert_eq!(page.items[0].journal_id.value(), 1);
        assert_eq!(page.items[0].platform_journal_id, None);

        let unavailable = list_journals(
            &fixture.config,
            Some(&fixture.db_name),
            &JournalListParams {
                area: Some("Medicine".to_string()),
                available: Some(false),
                sort: Some("title:asc".to_string()),
                limit: 10,
                offset: 0,
                ..JournalListParams::default()
            },
        )
        .expect("availability filter should apply");
        assert_eq!(unavailable.items[0].journal_id.value(), 3);

        let issue_page = list_issues(
            &fixture.config,
            Some(&fixture.db_name),
            &IssueListParams {
                journal_id: Some(1),
                year: Some(2026),
                is_valid_issue: Some(true),
                suppressed: Some(false),
                embargoed: Some(false),
                within_subscription: Some(true),
                sort: Some("date:desc".to_string()),
                limit: 10,
                offset: 0,
            },
        )
        .expect("issue filters should apply");
        assert_eq!(issue_page.page.total, Some(1));
        assert_eq!(issue_page.items[0].issue_id, 10);

        let sort_error = list_journals(
            &fixture.config,
            Some(&fixture.db_name),
            &JournalListParams {
                sort: Some("unknown:asc".to_string()),
                limit: 10,
                offset: 0,
                ..JournalListParams::default()
            },
        )
        .expect_err("unsupported journal sort should fail");
        assert!(matches!(
            sort_error,
            IndexRepositoryError::UnsupportedSortField(field) if field == "unknown"
        ));

        let limit_error = list_journals(
            &fixture.config,
            Some(&fixture.db_name),
            &JournalListParams {
                limit: 0,
                offset: 0,
                ..JournalListParams::default()
            },
        )
        .expect_err("invalid limit should fail");
        assert!(matches!(
            limit_error,
            IndexRepositoryError::InvalidPagination("limit must be between 1 and 200")
        ));
    }

    #[test]
    fn article_listing_filters_cover_fts5_and_supported_expressions() {
        let fixture = IndexFixture::new(true);
        let cases = vec![
            (
                "journal ids",
                ArticleListParams {
                    journal_id: vec![1],
                    ..article_filter_params()
                },
                vec![1004, 1001, 1002, 1005, 1008],
            ),
            (
                "issue id",
                ArticleListParams {
                    issue_id: Some(10),
                    ..article_filter_params()
                },
                vec![1001, 1002, 1005, 1008],
            ),
            (
                "publication year",
                ArticleListParams {
                    year: Some(2026),
                    ..article_filter_params()
                },
                vec![1003, 1001, 1002, 1005, 1008],
            ),
            (
                "area",
                ArticleListParams {
                    area: vec!["Engineering".to_string()],
                    ..article_filter_params()
                },
                vec![1003],
            ),
            (
                "in press",
                ArticleListParams {
                    in_press: Some(true),
                    ..article_filter_params()
                },
                vec![1004],
            ),
            (
                "open access",
                ArticleListParams {
                    open_access: Some(true),
                    ..article_filter_params()
                },
                vec![1001],
            ),
            (
                "library holdings",
                ArticleListParams {
                    within_library_holdings: Some(false),
                    ..article_filter_params()
                },
                vec![1003, 1005, 1008],
            ),
            (
                "date range",
                ArticleListParams {
                    date_from: Some("2026-01-03".to_string()),
                    date_to: Some("2026-01-05".to_string()),
                    ..article_filter_params()
                },
                vec![1001, 1002, 1005],
            ),
            (
                "doi",
                ArticleListParams {
                    doi: Some("10.1000/doi-only".to_string()),
                    ..article_filter_params()
                },
                vec![1005],
            ),
            (
                "pmid",
                ArticleListParams {
                    pmid: Some("PMID-1002".to_string()),
                    ..article_filter_params()
                },
                vec![1002],
            ),
            (
                "fts5 title and abstract",
                ArticleListParams {
                    q: Some("genome".to_string()),
                    ..article_filter_params()
                },
                vec![1004, 1001],
            ),
            (
                "fts5 indexed-only token",
                ArticleListParams {
                    q: Some("indexedonly".to_string()),
                    ..article_filter_params()
                },
                vec![1002],
            ),
            (
                "combined fts5 and structured filters",
                ArticleListParams {
                    area: vec!["Medicine".to_string()],
                    open_access: Some(true),
                    within_library_holdings: Some(true),
                    q: Some("genome".to_string()),
                    ..article_filter_params()
                },
                vec![1001],
            ),
            (
                "suppressed",
                ArticleListParams {
                    suppressed: Some(true),
                    q: Some("genome".to_string()),
                    ..article_filter_params()
                },
                vec![1006],
            ),
        ];

        for (name, params, expected_ids) in cases {
            let page = list_articles(&fixture.config, Some(&fixture.db_name), &params)
                .unwrap_or_else(|error| panic!("{name} should query successfully: {error}"));
            assert_eq!(article_ids(&page), expected_ids, "{name}");
            assert_eq!(page.page.total, Some(expected_ids.len() as i64), "{name}");
        }
    }

    #[test]
    fn article_listing_cursor_and_sort_expression_errors_are_checked() {
        let fixture = IndexFixture::new(true);
        let first_page_params = ArticleListParams {
            limit: 2,
            ..article_filter_params()
        };

        let first_page = list_articles(&fixture.config, Some(&fixture.db_name), &first_page_params)
            .expect("first page should query");

        assert_eq!(article_ids(&first_page), [1003, 1004]);
        assert_eq!(first_page.page.total, Some(6));
        assert_eq!(first_page.page.has_more, Some(true));
        assert_eq!(
            first_page.page.next_cursor.as_deref(),
            Some("2026-01-06|1004")
        );

        let second_page_params = ArticleListParams {
            cursor: first_page.page.next_cursor,
            limit: 2,
            ..article_filter_params()
        };
        let second_page =
            list_articles(&fixture.config, Some(&fixture.db_name), &second_page_params)
                .expect("second page should query");
        assert_eq!(article_ids(&second_page), [1001, 1002]);

        let invalid_cursor = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                cursor: Some("not-a-cursor".to_string()),
                ..article_filter_params()
            },
        )
        .expect_err("invalid cursor should fail");
        assert!(matches!(
            invalid_cursor,
            IndexRepositoryError::InvalidCursor
        ));

        let unsupported_field = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                sort: Some("title:asc".to_string()),
                ..article_filter_params()
            },
        )
        .expect_err("unsupported article sort field should fail");
        assert!(matches!(
            unsupported_field,
            IndexRepositoryError::UnsupportedSortField(field) if field == "title"
        ));

        let empty_sort = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                sort: Some(String::new()),
                ..article_filter_params()
            },
        )
        .expect_err("empty article sort should fail");
        assert!(matches!(
            empty_sort,
            IndexRepositoryError::UnsupportedArticleSort
        ));
    }

    #[test]
    fn article_fallback_query_uses_fts5_with_joined_filters_when_listing_is_not_ready() {
        let fixture = IndexFixture::new(false);
        let indexed_only = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                area: vec!["Medicine".to_string()],
                q: Some("indexedonly".to_string()),
                ..article_filter_params()
            },
        )
        .expect("fallback search should use FTS5");
        assert_eq!(article_ids(&indexed_only), [1002]);
        assert_eq!(indexed_only.page.total, Some(1));

        let joined = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                area: vec!["Medicine".to_string()],
                year: Some(2026),
                q: Some("genome".to_string()),
                ..article_filter_params()
            },
        )
        .expect("fallback search should join FTS5, issues, and metadata");
        assert_eq!(article_ids(&joined), [1001]);
    }

    #[test]
    fn article_access_and_fulltext_urls_cover_redirect_construction() {
        let fixture = IndexFixture::new(true);
        let user_id = UserId(1);

        let stored_access =
            get_article_access(&fixture.config, Some(&fixture.db_name), 1001, user_id)
                .expect("stored full text access should resolve");
        assert!(stored_access.fulltext.available);
        assert_eq!(
            stored_access.fulltext.provider.as_deref(),
            Some("stored_url")
        );

        assert_eq!(
            article_fulltext_redirect_url(&fixture.config, Some(&fixture.db_name), 1001, user_id)
                .expect("stored full text should redirect"),
            "https://files.example/fulltext.pdf"
        );
        assert_eq!(
            article_fulltext_redirect_url(&fixture.config, Some(&fixture.db_name), 1003, user_id)
                .expect("CNKI permalink should redirect without an active session"),
            "https://oversea.cnki.net/kcms/detail/abc?foo=bar&language=chs"
        );
        assert_eq!(
            article_fulltext_redirect_url(&fixture.config, Some(&fixture.db_name), 1005, user_id)
                .expect("DOI fallback should redirect"),
            "https://doi.org/10.1000/doi-only"
        );

        let missing_url =
            article_fulltext_redirect_url(&fixture.config, Some(&fixture.db_name), 1008, user_id)
                .expect_err("missing full text should fail");
        assert!(matches!(
            missing_url,
            IndexRepositoryError::NotFound("Full text not available")
        ));

        let cnki_access =
            get_article_access(&fixture.config, Some(&fixture.db_name), 1003, user_id)
                .expect("CNKI access should resolve");
        assert_eq!(cnki_access.detail.provider.as_deref(), Some("detail_url"));
        assert_eq!(
            cnki_access.detail.url.as_deref(),
            Some("https://oversea.cnki.net/kcms/detail/abc?foo=bar&language=chs")
        );
        assert!(!cnki_access.fulltext.available);
        assert!(cnki_access.fulltext.requires_login);
        assert_eq!(cnki_access.fulltext.provider.as_deref(), Some("zjlib_cnki"));

        match article_fulltext_target(&fixture.config, Some(&fixture.db_name), 1001, user_id)
            .expect("stored full text target should resolve")
        {
            ArticleFulltextTarget::Redirect(url) => {
                assert_eq!(url, "https://files.example/fulltext.pdf");
            }
            ArticleFulltextTarget::Pdf { .. } => panic!("stored full text should redirect"),
        }

        crate::upsert_cnki_session(
            fixture.config.auth_db_path(),
            user_id,
            &json!({"bff_user_token":"x.eyJleHAiOjQxMDI0NDQ4MDB9.y"}),
            "active",
            None,
        )
        .expect("CNKI session should be stored");
        let active_cnki =
            get_article_access(&fixture.config, Some(&fixture.db_name), 1003, user_id)
                .expect("active CNKI access should resolve");
        assert!(active_cnki.fulltext.available);
        assert!(!active_cnki.fulltext.requires_login);

        let active_redirect =
            article_fulltext_redirect_url(&fixture.config, Some(&fixture.db_name), 1003, user_id)
                .expect_err("active CNKI should not redirect to protected order URL");
        assert!(matches!(
            active_redirect,
            IndexRepositoryError::NotFound("CNKI full-text download is not migrated yet")
        ));
    }

    #[test]
    fn cnki_language_url_helpers_cover_query_variants() {
        assert_eq!(
            with_cnki_chinese_language("https://example.test/article"),
            "https://example.test/article"
        );
        assert_eq!(
            with_cnki_chinese_language("https://oversea.cnki.net/kcms/detail/abc"),
            "https://oversea.cnki.net/kcms/detail/abc?language=chs"
        );
        assert_eq!(
            with_cnki_chinese_language("https://oversea.cnki.net/kcms/detail/abc?foo=bar"),
            "https://oversea.cnki.net/kcms/detail/abc?foo=bar&language=chs"
        );
        assert_eq!(
            with_cnki_chinese_language("https://oversea.cnki.net/kcms/detail/abc?language=chs"),
            "https://oversea.cnki.net/kcms/detail/abc?language=chs"
        );
        assert!(is_cnki_protected_fulltext_url(
            "https://O.OVERSEA.CNKI.NET/barnew/download/order?id=abc"
        ));
    }

    fn article_filter_params() -> ArticleListParams {
        ArticleListParams {
            suppressed: Some(false),
            sort: Some("date:desc".to_string()),
            limit: 20,
            ..ArticleListParams::default()
        }
    }

    fn article_ids(page: &ArticlePage) -> Vec<i64> {
        page.items
            .iter()
            .map(|article| article.article_id.value())
            .collect()
    }

    fn value_counts(values: Vec<ValueCount>) -> Vec<(String, i64)> {
        values
            .into_iter()
            .map(|value| (value.value, value.count))
            .collect()
    }

    fn create_fixture_user(config: &StorageConfig) {
        let connection =
            Connection::open(config.auth_db_path()).expect("auth database should open");
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

    fn create_fixture_schema(connection: &Connection, is_listing_ready: bool) {
        let listing_status = if is_listing_ready { "ready" } else { "stale" };
        connection
            .execute_batch(&format!(
                r#"
                CREATE TABLE journals (
                    journal_id INTEGER PRIMARY KEY,
                    library_id TEXT NOT NULL,
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
    }
}
