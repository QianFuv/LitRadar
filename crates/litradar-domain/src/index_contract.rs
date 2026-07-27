//! Provider-neutral indexing and live article access contracts.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use utoipa::ToSchema;

use crate::{ArticleId, UserId};

/// Current canonical provider contract version.
pub const INDEX_CONTRACT_VERSION: u32 = 2;

/// Precision retained by one canonical partial publication date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DatePrecision {
    /// Only the publication year is known.
    Year,
    /// The publication year and month are known.
    Month,
    /// The complete calendar date is known.
    Day,
}

/// Calendar-validated canonical partial publication date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalPartialDate {
    /// Canonical `YYYY`, `YYYY-MM`, or `YYYY-MM-DD` text.
    pub value: String,
    /// Precision represented by `value`.
    pub precision: DatePrecision,
}

/// Normalize a contract string to trimmed Unicode NFC form.
///
/// # Arguments
///
/// * `value` - Untrusted contract string.
///
/// # Returns
///
/// Canonical non-empty text or `None` for empty input.
pub fn normalize_contract_text(value: &str) -> Option<String> {
    let normalized = value.nfc().collect::<String>();
    let trimmed = normalized.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Normalize and calendar-validate a partial publication date.
///
/// # Arguments
///
/// * `value` - Candidate `YYYY`, `YYYY-MM`, or `YYYY-MM-DD` text.
///
/// # Returns
///
/// Canonical text and its real precision, or `None` for invalid syntax or calendar dates.
pub fn normalize_contract_date(value: &str) -> Option<CanonicalPartialDate> {
    let value = normalize_contract_text(value)?;
    let bytes = value.as_bytes();
    let precision = match bytes.len() {
        4 if bytes.iter().all(u8::is_ascii_digit) => {
            let year = value.parse::<i32>().ok()?;
            NaiveDate::from_ymd_opt(year, 1, 1)?;
            DatePrecision::Year
        }
        7 if bytes.get(4) == Some(&b'-')
            && bytes[..4].iter().all(u8::is_ascii_digit)
            && bytes[5..].iter().all(u8::is_ascii_digit) =>
        {
            let year = value[..4].parse::<i32>().ok()?;
            let month = value[5..].parse::<u32>().ok()?;
            NaiveDate::from_ymd_opt(year, month, 1)?;
            DatePrecision::Month
        }
        10 if bytes.get(4) == Some(&b'-')
            && bytes.get(7) == Some(&b'-')
            && bytes[..4].iter().all(u8::is_ascii_digit)
            && bytes[5..7].iter().all(u8::is_ascii_digit)
            && bytes[8..].iter().all(u8::is_ascii_digit) =>
        {
            NaiveDate::parse_from_str(&value, "%Y-%m-%d").ok()?;
            DatePrecision::Day
        }
        _ => return None,
    };
    Some(CanonicalPartialDate { value, precision })
}

/// Derive precision from an already canonical partial publication date.
///
/// # Arguments
///
/// * `value` - Stored partial publication date.
///
/// # Returns
///
/// The validated precision, or `None` when legacy data is not a valid canonical date.
pub fn date_precision(value: &str) -> Option<DatePrecision> {
    normalize_contract_date(value).map(|date| date.precision)
}

/// Normalize a DOI to lowercase identifier form without a URL or prefix.
///
/// # Arguments
///
/// * `value` - DOI identifier, DOI prefix form, or DOI URL.
///
/// # Returns
///
/// Canonical DOI when the input has a valid identity shape.
pub fn normalize_contract_doi(value: &str) -> Option<String> {
    let mut normalized = normalize_contract_text(value)?.to_lowercase();
    for prefix in ["https://doi.org/", "http://doi.org/", "doi:"] {
        if normalized.starts_with(prefix) {
            normalized = normalized[prefix.len()..].trim().to_string();
            break;
        }
    }
    (normalized.starts_with("10.")
        && normalized.contains('/')
        && !normalized.bytes().any(|byte| byte.is_ascii_whitespace())
        && !normalized.contains("://"))
    .then_some(normalized)
}

/// Normalize a PubMed identifier to its canonical decimal representation.
///
/// # Arguments
///
/// * `value` - PubMed identifier text.
///
/// # Returns
///
/// Canonical digits-only identifier.
pub fn normalize_contract_pmid(value: &str) -> Option<String> {
    let normalized = normalize_contract_text(value)?;
    if !normalized.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let without_zeroes = normalized.trim_start_matches('0');
    Some(if without_zeroes.is_empty() {
        "0".to_string()
    } else {
        without_zeroes.to_string()
    })
}

/// Normalize and checksum-validate an ISSN.
///
/// # Arguments
///
/// * `value` - ISSN with or without its canonical hyphen.
///
/// # Returns
///
/// Canonical `NNNN-NNNX` ISSN when valid.
pub fn normalize_contract_issn(value: &str) -> Option<String> {
    let compact = normalize_contract_text(value)?
        .chars()
        .filter(|character| *character != '-' && !character.is_whitespace())
        .flat_map(char::to_uppercase)
        .collect::<String>();
    let bytes = compact.as_bytes();
    if bytes.len() != 8
        || !bytes[..7].iter().all(u8::is_ascii_digit)
        || !(bytes[7].is_ascii_digit() || bytes[7] == b'X')
    {
        return None;
    }
    let sum = bytes[..7]
        .iter()
        .enumerate()
        .map(|(index, byte)| usize::from(byte - b'0') * (8 - index))
        .sum::<usize>();
    let check = (11 - (sum % 11)) % 11;
    let expected = match check {
        10 => b'X',
        value => b'0' + u8::try_from(value).ok()?,
    };
    (bytes[7] == expected).then(|| format!("{}-{}", &compact[..4], &compact[4..]))
}

/// Normalize text for provider-neutral bibliographic comparisons.
///
/// # Arguments
///
/// * `value` - Display text.
///
/// # Returns
///
/// Lowercase NFC text with punctuation and whitespace collapsed.
pub fn normalize_bibliographic_text(value: &str) -> String {
    let normalized = value
        .nfc()
        .flat_map(char::to_lowercase)
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>();
    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Normalize a bibliographic volume, issue, or page label.
///
/// # Arguments
///
/// * `value` - Bibliographic label.
///
/// # Returns
///
/// Canonical comparison label with decimal leading zeroes removed.
pub fn normalize_bibliographic_label(value: &str) -> String {
    let normalized = normalize_bibliographic_text(value);
    if normalized.bytes().all(|byte| byte.is_ascii_digit()) {
        let without_zeroes = normalized.trim_start_matches('0');
        if without_zeroes.is_empty() {
            "0".to_string()
        } else {
            without_zeroes.to_string()
        }
    } else {
        normalized
    }
}

/// Curated journal ranking values from the maintained catalog.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalRankings {
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
}

/// One provider-free journal entry maintained by LitRadar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalCatalogEntry {
    /// Immutable opaque catalog identifier.
    pub catalog_id: String,
    /// Historical catalog identifiers that resolve to the canonical entry.
    pub catalog_aliases: Vec<String>,
    /// Canonical journal title.
    pub title: String,
    /// Canonical print ISSN.
    pub issn: Option<String>,
    /// Canonical electronic ISSN.
    pub eissn: Option<String>,
    /// All normalized ISSNs associated with the journal.
    pub all_issns: Vec<String>,
    /// Accepted title aliases used to validate provider observations.
    pub title_aliases: Vec<String>,
    /// Curated journal area.
    pub area: Option<String>,
    /// Curated ranking values.
    pub rankings: JournalRankings,
}

/// Provider observation for the requested journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalDraft {
    /// Catalog identifier echoed from the request.
    pub catalog_id: String,
    /// Provider-observed journal title.
    pub observed_title: Option<String>,
    /// Provider-observed normalized ISSNs.
    pub observed_issns: Vec<String>,
    /// Provider-observed title aliases.
    pub observed_title_aliases: Vec<String>,
}

/// Provider-neutral issue content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueDraft {
    /// Catalog identifier echoed from the request.
    pub catalog_id: String,
    /// Publication year.
    pub publication_year: Option<i64>,
    /// Issue title.
    pub title: Option<String>,
    /// Volume label.
    pub volume: Option<String>,
    /// Issue number.
    pub number: Option<String>,
    /// Validated ISO publication date.
    pub date: Option<String>,
}

/// One ordered article author in the canonical v2 contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArticleAuthorDraft {
    /// Trimmed author display name.
    pub display_name: String,
}

/// Provider-neutral article content without durable identifiers or links.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArticleDraft {
    /// Catalog identifier echoed from the request.
    pub catalog_id: String,
    /// Article title.
    pub title: String,
    /// Publication year.
    pub publication_year: Option<i64>,
    /// Validated ISO publication date.
    pub date: Option<String>,
    /// Issue title when supplied by the provider.
    pub issue_title: Option<String>,
    /// Volume label.
    pub volume: Option<String>,
    /// Issue number.
    pub issue_number: Option<String>,
    /// Ordered canonical authors.
    pub authors: Vec<ArticleAuthorDraft>,
    /// Start page.
    pub start_page: Option<String>,
    /// End page.
    pub end_page: Option<String>,
    /// Abstract text.
    pub abstract_text: Option<String>,
    /// Normalized DOI without a URL or prefix.
    pub doi: Option<String>,
    /// Numeric PubMed identifier.
    pub pmid: Option<String>,
    /// Whether the article is open access.
    pub open_access: Option<bool>,
    /// Whether the article is in press.
    pub in_press: Option<bool>,
    /// Normalized retraction DOIs in lexical order without duplicates.
    pub retraction_dois: Vec<String>,
}

/// One provider page of canonical journal, issue, and article content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderBatch {
    /// Catalog identifier echoed from the request.
    pub catalog_id: String,
    /// Canonical journal observation.
    pub journal: JournalDraft,
    /// Canonical issues returned in this page.
    pub issues: Vec<IssueDraft>,
    /// Canonical articles returned in this page.
    pub articles: Vec<ArticleDraft>,
    /// Whether the provider has completed the requested journal scan.
    pub is_complete: bool,
    /// Opaque provider checkpoint stored only in the control database.
    pub next_checkpoint: Option<String>,
}

/// Provider-neutral article metadata used for request-time access resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleLocator {
    /// Internal article identifier.
    pub article_id: ArticleId,
    /// Immutable catalog identifier.
    pub catalog_id: String,
    /// Canonical journal title.
    pub journal_title: String,
    /// Canonical journal ISSNs.
    pub journal_issns: Vec<String>,
    /// Article title.
    pub title: String,
    /// Publication year.
    pub publication_year: Option<i64>,
    /// Validated ISO publication date.
    pub date: Option<String>,
    /// Ordered author display names.
    pub authors: Vec<String>,
    /// Volume label.
    pub volume: Option<String>,
    /// Issue number.
    pub issue_number: Option<String>,
    /// Start page.
    pub start_page: Option<String>,
    /// End page.
    pub end_page: Option<String>,
    /// Normalized DOI.
    pub doi: Option<String>,
    /// Numeric PubMed identifier.
    pub pmid: Option<String>,
}

/// Optional capability advertised by a provider registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProviderCapabilityKind {
    /// Canonical journal and article indexing.
    IndexContent,
    /// Request-time article abstract-page resolution.
    ArticleAbstract,
    /// Request-time article full-text resolution.
    ArticleFullText,
}

/// Request context exposed to online article capability providers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ArticleAccessContext {
    /// Authenticated LitRadar user when present.
    pub user_id: Option<UserId>,
}

/// Ephemeral redirect returned by a live provider capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleRedirect {
    /// Upstream destination used only for the current response.
    pub location: String,
}

/// Ephemeral full-text document returned by a live provider capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleFullTextDocument {
    /// Validated response content type.
    pub content_type: String,
    /// Optional safe download filename.
    pub filename: Option<String>,
    /// Bounded document bytes.
    pub bytes: Vec<u8>,
}

/// Successful request-time full-text resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArticleFullTextResolution {
    /// Redirect the caller to an ephemeral upstream destination.
    Redirect(ArticleRedirect),
    /// Stream bounded document bytes to the caller.
    Document(ArticleFullTextDocument),
}

#[cfg(test)]
mod tests {
    use super::{
        date_precision, normalize_contract_date, ArticleDraft, DatePrecision, JournalCatalogEntry,
        JournalRankings, INDEX_CONTRACT_VERSION,
    };

    #[test]
    fn canonical_partial_dates_preserve_precision_and_validate_the_calendar() {
        for (input, canonical, precision) in [
            (" 2026 ", "2026", DatePrecision::Year),
            ("2026-02", "2026-02", DatePrecision::Month),
            ("2024-02-29", "2024-02-29", DatePrecision::Day),
        ] {
            let date = normalize_contract_date(input).expect("date should be valid");
            assert_eq!(date.value, canonical);
            assert_eq!(date.precision, precision);
            assert_eq!(date_precision(canonical), Some(precision));
        }

        for invalid in [
            "2026-02-29",
            "2026-02-31",
            "2026-13",
            "2026-00",
            "2026-2",
            "2026-02-01T00:00:00Z",
            "not-a-date",
        ] {
            assert_eq!(normalize_contract_date(invalid), None, "accepted {invalid}");
            assert_eq!(date_precision(invalid), None, "classified {invalid}");
        }
    }

    #[test]
    fn canonical_content_serialization_has_no_provider_or_link_fields() {
        let catalog = JournalCatalogEntry {
            catalog_id: "issn-1234-5678".to_string(),
            catalog_aliases: vec!["legacy-journal".to_string()],
            title: "Canonical Journal".to_string(),
            issn: Some("1234-5678".to_string()),
            eissn: None,
            all_issns: vec!["1234-5678".to_string()],
            title_aliases: Vec::new(),
            area: Some("Systems".to_string()),
            rankings: JournalRankings::default(),
        };
        let article = ArticleDraft {
            catalog_id: catalog.catalog_id.clone(),
            title: "Canonical Article".to_string(),
            publication_year: Some(2026),
            date: Some("2026-07-18".to_string()),
            issue_title: None,
            volume: Some("1".to_string()),
            issue_number: Some("2".to_string()),
            authors: Vec::new(),
            start_page: Some("1".to_string()),
            end_page: Some("8".to_string()),
            abstract_text: Some("Abstract".to_string()),
            doi: Some("10.1000/canonical".to_string()),
            pmid: None,
            open_access: Some(true),
            in_press: Some(false),
            retraction_dois: Vec::new(),
        };

        let serialized =
            serde_json::to_string(&(catalog, article.clone())).expect("serialize contract");
        assert!(serialized.contains("\"retraction_dois\":[]"));
        assert!(!serialized.contains("\"retraction_doi\":"));
        for forbidden in [
            "provider",
            "source",
            "platform_id",
            "permalink",
            "content_location",
            "full_text_file",
            "cover_url",
            "https://",
        ] {
            assert!(!serialized.contains(forbidden), "found {forbidden}");
        }
        let mut provider_shaped = serde_json::to_value(article)
            .expect("serialize article")
            .as_object()
            .expect("article should serialize as object")
            .clone();
        provider_shaped.insert(
            "provider".to_string(),
            serde_json::Value::String("private-source".to_string()),
        );
        provider_shaped.insert(
            "permalink".to_string(),
            serde_json::Value::String("https://example.test/article".to_string()),
        );
        assert!(
            serde_json::from_value::<ArticleDraft>(serde_json::Value::Object(provider_shaped))
                .is_err()
        );
        assert_eq!(INDEX_CONTRACT_VERSION, 2);
    }
}
