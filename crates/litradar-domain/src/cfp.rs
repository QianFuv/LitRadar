//! Original-language calls for papers, normalized dates and conservative availability.

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

mod dates;
pub use dates::parse_cfp_dates;

/// Version of the original-text normalization contract.
pub const CFP_PARSER_VERSION: u32 = 1;

/// Submission purpose supported by the original announcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CfpKind {
    /// Themed journal issue or section.
    SpecialIssue,
    /// Regular submissions or annual topic guidance.
    General,
    /// Guest editor or issue proposal.
    Proposal,
    /// Journal call connected with an event.
    ConferenceLinked,
}

/// Purpose of a dated milestone, independent of whether it is required.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CfpDateStage {
    /// Initial manuscript.
    Paper,
    /// Initial abstract prerequisite.
    Abstract,
    /// Initial proposal prerequisite.
    Proposal,
    /// Start of a submission window.
    Opens,
    /// Revision or final accepted manuscript.
    Revision,
    /// Editorial decision or notification.
    Decision,
    /// Publication milestone.
    Publication,
    /// Conference or related event.
    Event,
    /// Event registration or payment.
    Registration,
}

impl CfpDateStage {
    /// Whether this stage can admit a new contribution.
    pub fn is_submission(self) -> bool {
        matches!(self, Self::Paper | Self::Abstract | Self::Proposal)
    }
}

/// Availability evaluated at a specified instant, without translating source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CfpState {
    /// A known admission deadline is still open.
    Open,
    /// The known submission window has not started.
    Upcoming,
    /// The initial gate has closed or the publisher explicitly closed the call.
    Closed,
    /// Archived source without current admission evidence.
    Historical,
    /// Only invited contributions may enter.
    InvitationOnly,
    /// No submission deadline was extracted.
    Undated,
    /// An unresolved timeline or timezone boundary prevents a reliable answer.
    Uncertain,
}

impl CfpState {
    /// Whether the notice belongs to the archived/closed filter.
    pub fn is_archived(self) -> bool {
        matches!(self, Self::Closed | Self::Historical)
    }
}

/// Literal, reviewed source fields before normalization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CfpSource {
    /// Immutable catalog aliases of this journal only.
    pub catalog_ids: Vec<String>,
    /// Publisher journal title.
    pub journal_title: String,
    /// Original announcement title.
    pub title: String,
    /// Literal topic/scope excerpt.
    #[serde(default)]
    pub scope: String,
    /// Literal submission instructions.
    #[serde(default)]
    pub requirements: String,
    /// Original call-kind label used for classification.
    pub type_text: String,
    /// Labeled timeline suitable for strict date extraction.
    pub date_text: String,
    /// Public original announcement URL.
    pub source_url: String,
    /// Date on which the original was actually checked.
    pub checked_on: String,
    /// Source-specific initial submission stage.
    pub entry_stage: Option<CfpDateStage>,
    /// Explicit source timezone, never inferred for international sources.
    pub time_zone: Option<String>,
    /// Original explicit closure or invitation statement.
    pub status_text: Option<String>,
    /// Whether the source was verified as archival.
    #[serde(default)]
    pub is_historical: bool,
    /// Original timeline that could not be normalized safely.
    #[serde(default)]
    pub raw_date_text: String,
}

/// A valid calendar date and its original clause.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CfpDate {
    /// ISO calendar date, with no invented time of day.
    pub date: String,
    /// Purpose of this date.
    pub stage: CfpDateStage,
    /// Untranslated source clause.
    pub original_text: String,
    /// Whether the source says strictly before this date.
    pub is_exclusive: bool,
    /// Whether this milestone is optional.
    pub is_optional: bool,
}

/// Normalized notice persisted by the backend; availability is evaluated on read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CfpNotice {
    /// Stable legacy-compatible URL/title identity within a journal.
    pub id: String,
    /// Original announcement title.
    pub title: String,
    /// Original topic excerpt, empty if unavailable.
    pub scope: String,
    /// Original instructions, empty if unavailable.
    pub requirements: String,
    /// Classified kind of call.
    pub kind: CfpKind,
    /// Original public announcement URL.
    pub source_url: String,
    /// Last verified original-source date.
    pub checked_on: String,
    /// Validated dates, including non-submission milestones.
    pub dates: Vec<CfpDate>,
    /// Default initial submission stage.
    pub entry_stage: CfpDateStage,
    /// Explicit annual topic year, when applicable.
    pub topic_year: Option<i32>,
    /// Explicit IANA timezone, if supplied by the source.
    pub time_zone: Option<String>,
    /// Explicit closed or invitation-only source constraint.
    pub source_status: Option<CfpState>,
    /// Whether this announcement is an archival record.
    pub is_historical: bool,
    /// Unresolved original timeline; never interpreted as perpetual availability.
    pub raw_date_text: String,
}

/// A publisher's verified, scoped statement that no matching calls are listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CfpEmptyJournal {
    /// All maintained aliases of the journal.
    pub catalog_ids: Vec<String>,
    /// Original journal title.
    pub journal_title: String,
    /// Actual verification date.
    pub checked_on: String,
    /// Original page containing the statement.
    pub source_url: String,
    /// Literal scoped statement, not a general claim about ordinary submissions.
    pub source_statement: String,
    /// Empty array retained for importing the reviewed legacy payload.
    #[serde(default)]
    pub notices: Vec<CfpNotice>,
}

/// Versioned offline import payload with optional count assertions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CfpSeed {
    /// Backend input format version.
    pub format_version: u32,
    /// Literal source fragments.
    pub sources: Vec<CfpSource>,
    /// Explicit verified-empty statements.
    pub empty_journals: Vec<CfpEmptyJournal>,
    /// Expected distinct journal count, if supplied by the export.
    pub expected_journals: Option<usize>,
    /// Expected distinct notice count, if supplied by the export.
    pub expected_notices: Option<usize>,
}

/// Coverage of a journal's reviewed original-source data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CfpCoverage {
    /// At least one reviewed notice or explicit empty-source statement is stored.
    Adapted,
    /// No reliable source snapshot has been adapted yet.
    Unadapted,
}

/// Source refresh freshness, independent of a notice's submission availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CfpRefreshStatus {
    /// Reviewed offline snapshot; no live refresh has succeeded.
    Snapshot,
    /// Latest acquisition succeeded.
    Success,
    /// Latest attempt failed; last-good data is retained.
    Failed,
    /// An unexpired source refresh is in progress.
    Refreshing,
    /// Data exists but automatic source discovery is not adapted.
    Unsupported,
    /// No source data is available for this catalog member.
    Unadapted,
}

/// Lightweight journal metadata and backend-calculated CFP summary.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CfpJournalSummary {
    /// Canonical maintained catalog ID used by the notice endpoint.
    pub catalog_id: String,
    /// Maintained historical aliases in the selected catalog.
    pub catalog_aliases: Vec<String>,
    /// All maintained print and electronic ISSNs for display and journal search.
    pub all_issns: Vec<String>,
    /// Maintained journal title aliases for search.
    pub title_aliases: Vec<String>,
    /// Journal title from the selected metadata database.
    pub title: String,
    /// Maintained subject area, if available.
    pub area: Option<String>,
    /// Reviewed-source coverage.
    pub coverage: CfpCoverage,
    /// Last verified original-source date, if adapted.
    pub checked_on: Option<String>,
    /// Original source URL associated with a scoped publisher statement.
    pub source_url: Option<String>,
    /// Untranslated scoped statement that no matching calls are listed.
    pub source_statement: Option<String>,
    /// Total stored notices, including closed and historical calls.
    pub notice_count: usize,
    /// Notices remaining after the default closed/historical filter.
    pub current_count: usize,
    /// Backend-computed counts for each availability state.
    pub state_counts: std::collections::BTreeMap<CfpState, usize>,
    /// Whether a backend discovery adapter can attempt automatic refresh.
    pub can_refresh: bool,
    /// Latest acquisition status, including interrupted lease detection.
    pub refresh_status: CfpRefreshStatus,
    /// Latest acquisition attempt Unix timestamp.
    pub last_attempt: Option<i64>,
    /// Last successful live acquisition Unix timestamp.
    pub last_success: Option<i64>,
    /// Sanitized latest failure reason.
    pub last_error: Option<String>,
}

/// Aggregate counts for the selected database before the journal-name search filter.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CfpCatalogSummary {
    /// Complete maintained journal count, including journals without indexed articles.
    pub journals: usize,
    /// Journals with a reviewed snapshot or scoped empty statement.
    pub adapted_journals: usize,
    /// All stored notices associated with this database.
    pub notices: usize,
    /// Notices excluding closed and historical calls.
    pub current_notices: usize,
}

/// Complete lightweight CFP journal catalog for one database.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CfpCatalogResponse {
    /// Canonical selected database filename.
    pub database: String,
    /// UTC Unix instant used to evaluate every state in this response.
    pub evaluated_at: i64,
    /// Whole-database summary.
    pub summary: CfpCatalogSummary,
    /// Lightweight journals matching the optional search, without notice bodies.
    pub items: Vec<CfpJournalSummary>,
}

/// One original notice with its authoritative backend-evaluated state and initial gate.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CfpNoticeView {
    /// Original normalized notice fields.
    #[serde(flatten)]
    pub notice: CfpNotice,
    /// Availability evaluated at the page's fixed instant.
    pub state: CfpState,
    /// Earliest mandatory initial submission deadline.
    pub entry_deadline: Option<CfpDate>,
}

/// A revision-consistent page of original notices for one maintained catalog member.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CfpNoticePage {
    /// Metadata, coverage and source freshness for the journal.
    pub journal: CfpJournalSummary,
    /// Fixed UTC Unix evaluation instant shared across cursor pages.
    pub evaluated_at: i64,
    /// Original notice records in stable source order.
    pub items: Vec<CfpNoticeView>,
    /// Standard API pagination metadata, with a scope-bound next cursor.
    pub page: crate::PageMeta,
}

/// Normalize whitespace while preserving paragraph boundaries and original language.
pub fn clean_cfp_text(value: &str) -> String {
    dates::regex(r"[\t \u{00a0}]+")
        .replace_all(&value.replace("\r\n", "\n").replace('\r', "\n"), " ")
        .trim()
        .to_string()
}

/// Validate a public source URL syntax without fetching it.
pub fn is_cfp_source_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

/// Strictly normalize one source, preserving literal fields and admission semantics.
pub fn parse_cfp_source(source: &CfpSource) -> Option<CfpNotice> {
    use dates::matches;
    let kind = if matches(r"proposal|提案|专刊建议", &source.type_text) {
        CfpKind::Proposal
    } else if matches(r"conference|workshop|会议|研讨会|工作坊", &source.type_text) {
        CfpKind::ConferenceLinked
    } else if matches(r"special|专[题刊栏辑]|特[刊辑]", &source.type_text) {
        CfpKind::SpecialIssue
    } else if matches(r"general|regular|常规|年度|重点选题", &source.type_text) {
        CfpKind::General
    } else {
        return None;
    };
    let entry_stage = source.entry_stage.unwrap_or(if kind == CfpKind::Proposal {
        CfpDateStage::Proposal
    } else {
        CfpDateStage::Paper
    });
    let status = source.status_text.as_deref().unwrap_or_default();
    let source_status = if matches(r"\bclosed\b|已截[稿止]|征稿结束|已结束", status) {
        Some(CfpState::Closed)
    } else if matches(
        r"invit(?:ation|e|ed)[\s-]*only|invitation (?:is )?required|invitation basis|invited (?:submissions|papers) only|only (?:invited|by invitation)|仅限受邀|仅接受邀请",
        &format!("{status}\n{}", source.title),
    ) {
        Some(CfpState::InvitationOnly)
    } else {
        None
    };
    if !entry_stage.is_submission()
        || !is_cfp_source_url(&source.source_url)
        || source.catalog_ids.is_empty()
        || source.catalog_ids.iter().any(|id| id.trim().is_empty())
        || source.title.trim().is_empty()
        || matches(
            r"^(just a moment|access denied|404|暂未适配)",
            source.title.trim(),
        )
        || source
            .time_zone
            .as_ref()
            .is_some_and(|zone| zone.parse::<chrono_tz::Tz>().is_err())
        || !dates::regex(r"^\d{4}-\d{2}-\d{2}$").is_match(&source.checked_on)
        || NaiveDate::parse_from_str(&source.checked_on, "%Y-%m-%d").is_err()
    {
        return None;
    }
    let dates = match parse_cfp_dates(&source.date_text, entry_stage) {
        Some(dates) => dates,
        None if source.is_historical || source_status == Some(CfpState::Closed) => Vec::new(),
        None => return None,
    };
    let topic_year = (kind == CfpKind::General && matches(r"年度|年.*选题", &source.title))
        .then(|| {
            dates::regex(r"20\d{2}")
                .find(&source.title)
                .and_then(|value| value.as_str().parse().ok())
        })
        .flatten();
    let title = clean_cfp_text(&source.title);
    Some(CfpNotice {
        id: format!("{}#{title}", source.source_url),
        title,
        scope: clean_cfp_text(&source.scope),
        requirements: clean_cfp_text(&source.requirements),
        kind,
        source_url: source.source_url.clone(),
        checked_on: source.checked_on.clone(),
        dates,
        entry_stage,
        topic_year,
        time_zone: source.time_zone.clone(),
        source_status,
        is_historical: source.is_historical,
        raw_date_text: clean_cfp_text(&source.raw_date_text),
    })
}

impl CfpNotice {
    /// Return the earliest required abstract/proposal gate before the full-paper gate.
    pub fn entry_deadline(&self) -> Option<&CfpDate> {
        self.dates
            .iter()
            .filter(|date| {
                !date.is_optional
                    && matches!(date.stage, CfpDateStage::Abstract | CfpDateStage::Proposal)
            })
            .min_by(|first, second| {
                first
                    .date
                    .cmp(&second.date)
                    .then(second.is_exclusive.cmp(&first.is_exclusive))
            })
            .or_else(|| {
                self.dates
                    .iter()
                    .find(|date| date.stage == self.entry_stage && !date.is_optional)
            })
    }

    /// Evaluate source constraints and initial gates at a fixed UTC instant.
    pub fn state(&self, now: DateTime<Utc>) -> CfpState {
        if self.source_status == Some(CfpState::Closed) {
            return CfpState::Closed;
        }
        let uncertain = || {
            if self.is_historical {
                CfpState::Historical
            } else {
                self.source_status.unwrap_or(CfpState::Uncertain)
            }
        };
        if !self.raw_date_text.is_empty() {
            return uncertain();
        }
        let today = self
            .time_zone
            .as_ref()
            .and_then(|zone| zone.parse::<chrono_tz::Tz>().ok())
            .map_or(now.date_naive(), |zone| {
                now.with_timezone(&zone).date_naive()
            });
        let deadline = self.entry_deadline();
        let start = self
            .dates
            .iter()
            .find(|date| date.stage == CfpDateStage::Opens);
        let is_closed = |date: NaiveDate, deadline: &CfpDate| {
            let date = date.to_string();
            if deadline.is_exclusive {
                date >= deadline.date
            } else {
                date > deadline.date
            }
        };
        if self.time_zone.is_none() {
            let earliest = (now + Duration::hours(14)).date_naive();
            let latest = (now - Duration::hours(12)).date_naive();
            if deadline.is_some_and(|deadline| {
                is_closed(earliest, deadline) != is_closed(latest, deadline)
            }) {
                return uncertain();
            }
            if start.is_some_and(|start| {
                (earliest.to_string() < start.date) != (latest.to_string() < start.date)
            }) {
                return CfpState::Uncertain;
            }
        }
        if deadline.is_some_and(|deadline| is_closed(today, deadline))
            || self.topic_year.is_some_and(|year| today.year() > year)
        {
            CfpState::Closed
        } else if self.is_historical {
            CfpState::Historical
        } else if self.source_status == Some(CfpState::InvitationOnly) {
            CfpState::InvitationOnly
        } else if start.is_some_and(|start| today.to_string() < start.date) {
            CfpState::Upcoming
        } else if deadline.is_some() {
            CfpState::Open
        } else {
            CfpState::Undated
        }
    }
}
