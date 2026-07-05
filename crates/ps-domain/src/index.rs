//! Index database API models.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{ArticleId, JournalId};

/// Journal record with optional CSV metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct JournalRecord {
    /// Journal identifier.
    pub journal_id: JournalId,
    /// Source library identifier.
    pub library_id: String,
    /// Platform journal identifier.
    pub platform_journal_id: Option<String>,
    /// Journal title.
    pub title: Option<String>,
    /// Print ISSN.
    pub issn: Option<String>,
    /// Electronic ISSN.
    pub eissn: Option<String>,
    /// Scimago rank value.
    pub scimago_rank: Option<f64>,
    /// Cover image URL.
    pub cover_url: Option<String>,
    /// Availability flag.
    pub available: Option<i64>,
    /// TOC live flag.
    pub toc_data_approved_and_live: Option<i64>,
    /// Whether articles exist.
    pub has_articles: Option<i64>,
    /// Source CSV file.
    pub source_csv: Option<String>,
    /// Journal area.
    pub area: Option<String>,
    /// CSV title.
    pub csv_title: Option<String>,
    /// CSV ISSN.
    pub csv_issn: Option<String>,
    /// CSV library value.
    pub csv_library: Option<String>,
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
    /// Valid issue flag.
    pub is_valid_issue: Option<i64>,
    /// Suppressed flag.
    pub suppressed: Option<i64>,
    /// Embargoed flag.
    pub embargoed: Option<i64>,
    /// Subscription flag.
    pub within_subscription: Option<i64>,
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
    pub title: Option<String>,
    /// Article date.
    pub date: Option<String>,
    /// Authors text.
    pub authors: Option<String>,
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
    /// Permalink.
    pub permalink: Option<String>,
    /// Suppressed flag.
    pub suppressed: Option<i64>,
    /// In-press flag.
    pub in_press: Option<i64>,
    /// Open-access flag.
    pub open_access: Option<i64>,
    /// Platform identifier.
    pub platform_id: Option<String>,
    /// Retraction DOI.
    pub retraction_doi: Option<String>,
    /// Library holdings flag.
    pub within_library_holdings: Option<i64>,
    /// Content location URL.
    pub content_location: Option<String>,
    /// Full-text file URL.
    pub full_text_file: Option<String>,
    /// Journal title.
    pub journal_title: Option<String>,
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
    /// Provider identifier.
    pub provider: Option<String>,
    /// Action URL.
    pub url: Option<String>,
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
    pub title: Option<String>,
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
    pub title: Option<String>,
    /// Article date.
    pub date: Option<String>,
    /// Authors text.
    pub authors: Option<String>,
    /// Abstract text.
    #[serde(rename = "abstract")]
    pub abstract_text: Option<String>,
    /// DOI.
    pub doi: Option<String>,
    /// Platform identifier.
    pub platform_id: Option<String>,
    /// Permalink.
    pub permalink: Option<String>,
    /// Full-text file URL.
    pub full_text_file: Option<String>,
    /// Journal title.
    pub journal_title: Option<String>,
    /// Open-access flag.
    pub open_access: Option<i64>,
    /// In-press flag.
    pub in_press: Option<i64>,
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
