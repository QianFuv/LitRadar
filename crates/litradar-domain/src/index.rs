//! Index database API models.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{ArticleId, JournalId};

/// Provider-neutral journal record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct JournalRecord {
    /// Journal identifier.
    pub journal_id: JournalId,
    /// Immutable maintained catalog identifier.
    pub catalog_id: String,
    /// Journal title.
    pub title: String,
    /// Maintained title aliases.
    pub title_aliases: Vec<String>,
    /// All maintained ISSNs.
    pub issns: Vec<String>,
    /// Print ISSN.
    pub issn: Option<String>,
    /// Electronic ISSN.
    pub eissn: Option<String>,
    /// Journal area.
    pub area: Option<String>,
    /// UTD rank.
    pub utd_rank: Option<String>,
    /// UTD rating.
    pub utd_rating: Option<String>,
    /// ABS rank.
    pub abs_rank: Option<String>,
    /// ABS rating.
    pub abs_rating: Option<String>,
    /// FMS rank.
    pub fms_rank: Option<String>,
    /// FMS rating.
    pub fms_rating: Option<String>,
    /// FMS China rank.
    pub fmscn_rank: Option<String>,
    /// FMS China rating.
    pub fmscn_rating: Option<String>,
    /// Whether the content database currently contains articles for the journal.
    pub has_articles: bool,
}

/// Issue record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct IssueRecord {
    /// Issue identifier.
    pub issue_id: i64,
    /// Journal identifier.
    pub journal_id: JournalId,
    /// Publication year.
    pub publication_year: Option<i64>,
    /// Issue title.
    pub title: Option<String>,
    /// Volume.
    pub volume: Option<String>,
    /// Number.
    pub number: Option<String>,
    /// Issue date.
    pub date: Option<String>,
}

/// Article record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ArticleRecord {
    /// Article identifier.
    pub article_id: ArticleId,
    /// Journal identifier.
    pub journal_id: JournalId,
    /// Issue identifier.
    pub issue_id: Option<i64>,
    /// Article title.
    pub title: String,
    /// Publication year.
    pub publication_year: Option<i64>,
    /// Article date.
    pub date: Option<String>,
    /// Authors text.
    pub authors: Vec<String>,
    /// Start page.
    pub start_page: Option<String>,
    /// End page.
    pub end_page: Option<String>,
    /// Abstract text.
    #[serde(rename = "abstract")]
    pub abstract_text: Option<String>,
    /// DOI.
    pub doi: Option<String>,
    /// PubMed identifier.
    pub pmid: Option<String>,
    /// In-press flag.
    pub in_press: Option<bool>,
    /// Open-access flag.
    pub open_access: Option<bool>,
    /// Retraction DOI.
    pub retraction_doi: Option<String>,
    /// Journal title.
    pub journal_title: String,
    /// Issue volume.
    pub volume: Option<String>,
    /// Issue number.
    pub number: Option<String>,
}

/// Pagination metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PageMeta {
    /// Total rows when requested.
    pub total: Option<i64>,
    /// Page size.
    pub limit: i64,
    /// Offset row count.
    pub offset: i64,
    /// Keyset cursor for the next page.
    pub next_cursor: Option<String>,
    /// Whether another page may exist.
    pub has_more: Option<bool>,
}

/// Paginated journals response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct JournalPage {
    /// Journal records.
    pub items: Vec<JournalRecord>,
    /// Pagination metadata.
    pub page: PageMeta,
}

/// Paginated issues response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct IssuePage {
    /// Issue records.
    pub items: Vec<IssueRecord>,
    /// Pagination metadata.
    pub page: PageMeta,
}

/// Paginated articles response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ArticlePage {
    /// Article records.
    pub items: Vec<ArticleRecord>,
    /// Pagination metadata.
    pub page: PageMeta,
}

/// Article access action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ArticleAccessAction {
    /// Whether the action is available.
    pub available: bool,
    /// Display label.
    pub label: String,
    /// Whether login is required.
    pub requires_login: bool,
    /// Optional message.
    pub message: Option<String>,
}

/// Article access response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ArticleAccessResponse {
    /// Detail action.
    pub detail: ArticleAccessAction,
    /// Abstract-page action.
    pub abstract_page: ArticleAccessAction,
    /// Full-text action.
    pub fulltext: ArticleAccessAction,
}

/// Label/count tuple.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ValueCount {
    /// Label value.
    pub value: String,
    /// Row count.
    pub count: i64,
}

/// Publication year summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct YearSummary {
    /// Publication year.
    pub year: i64,
    /// Issue count.
    pub issue_count: i64,
    /// Journal count.
    pub journal_count: i64,
}

/// Journal option for selection lists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct JournalOption {
    /// Journal identifier.
    pub journal_id: JournalId,
    /// Journal title.
    pub title: String,
}

/// Weekly article record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct WeeklyArticleRecord {
    /// Article identifier.
    pub article_id: ArticleId,
    /// Journal identifier.
    pub journal_id: JournalId,
    /// Issue identifier.
    pub issue_id: Option<i64>,
    /// Article title.
    pub title: String,
    /// Publication year.
    pub publication_year: Option<i64>,
    /// Article date.
    pub date: Option<String>,
    /// Authors text.
    pub authors: Vec<String>,
    /// Abstract text.
    #[serde(rename = "abstract")]
    pub abstract_text: Option<String>,
    /// DOI.
    pub doi: Option<String>,
    /// Journal title.
    pub journal_title: String,
    /// Open-access flag.
    pub open_access: Option<bool>,
    /// In-press flag.
    pub in_press: Option<bool>,
    /// Issue volume.
    pub volume: Option<String>,
    /// Issue number.
    pub number: Option<String>,
}

/// Weekly update summary for one journal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct WeeklyJournalUpdate {
    /// Journal identifier.
    pub journal_id: JournalId,
    /// Journal title.
    pub journal_title: Option<String>,
    /// New article count.
    pub new_article_count: usize,
    /// Weekly articles.
    pub articles: Vec<WeeklyArticleRecord>,
}

/// Weekly update summary for one database.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct WeeklyDatabaseUpdate {
    /// Database filename.
    pub db_name: String,
    /// Source run identifier.
    pub run_id: Option<String>,
    /// Generated timestamp.
    pub generated_at: String,
    /// New article count.
    pub new_article_count: usize,
    /// Journal groups.
    pub journals: Vec<WeeklyJournalUpdate>,
}

/// Weekly updates response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct WeeklyUpdatesResponse {
    /// Response generated timestamp.
    pub generated_at: String,
    /// Window start timestamp.
    pub window_start: String,
    /// Window end timestamp.
    pub window_end: String,
    /// Database update groups.
    pub databases: Vec<WeeklyDatabaseUpdate>,
}
