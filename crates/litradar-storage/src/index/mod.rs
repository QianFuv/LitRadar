//! Typed repositories for index database read routes.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use litradar_domain::{
    ArticleAccessAction, ArticleAccessResponse, ArticleCandidateInfo, ArticleId, ArticlePage,
    ArticleRecord, IssuePage, IssueRecord, JournalId, JournalOption, JournalPage, JournalRecord,
    PageMeta, UserId, ValueCount, WeeklyArticleRecord, WeeklyDatabaseUpdate, WeeklyJournalUpdate,
    WeeklyUpdatesResponse, YearSummary,
};
use rusqlite::types::Value as SqlValue;
use rusqlite::{params_from_iter, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::cnki::{get_cnki_session_data, get_cnki_session_status, CnkiRepositoryError};
use crate::{
    open_sqlite_connection, try_load_extension, DatabaseResolutionError, SecretCodec, StorageConfig,
};

const MAX_LIMIT: i64 = 200;
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

mod articles;
mod fulltext;
mod metadata;
mod shared;
mod weekly;

pub use articles::{
    collect_inpress_article_counts, collect_issue_article_counts, fetch_candidates_for_article_ids,
    fetch_candidates_for_inpress_keys, fetch_candidates_for_issue_keys, get_article, list_articles,
    ArticleListParams,
};
pub use fulltext::{
    article_fulltext_redirect_url, article_fulltext_target, get_article_access,
    ArticleFulltextTarget, CnkiFulltextTarget,
};
pub use metadata::{
    get_issue, get_journal, list_areas, list_index_database_names, list_issues,
    list_journal_options, list_journals, list_sources, list_years, IssueListParams,
    JournalListParams,
};
pub use weekly::get_weekly_updates;

#[cfg(test)]
mod test_support;
