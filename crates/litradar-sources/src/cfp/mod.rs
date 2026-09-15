//! Registered original-language CFP discovery and bounded public-source transport.

use std::fmt;
use std::sync::LazyLock;
use std::time::Instant;

use litradar_domain::cfp::{CfpEmptyJournal, CfpSource};
use reqwest::Url;
use serde::{Deserialize, Serialize};

mod html;
mod http;

pub use html::{
    cfp_original_links, extract_cfp_full_text, is_cfp_challenge, parse_cfp_page, CfpParsedPage,
};
pub use http::{decode_cfp_body, CfpHttpDocument, CfpHttpTransport};

/// Reviewed offline initial snapshot, packaged with the backend only.
pub const CFP_SEED: &[u8] = include_bytes!("../../assets/cfp-seed.json");
/// Immutable import identity for the bundled reviewed snapshot.
pub const CFP_SEED_ID: &str = "reviewed-2026-09-15-v1";
/// Maximum decoded body or helper output bytes per source page.
pub const CFP_MAX_PAGE_BYTES: usize = 4 * 1024 * 1024;
/// Maximum total capture bytes retained for one acquisition unit.
pub const CFP_MAX_CAPTURE_BYTES: usize = 24 * 1024 * 1024;
/// Maximum detail pages followed from one discovery list.
pub const CFP_MAX_DETAIL_PAGES: usize = 12;

/// Reviewed parser capability, kept separate from snapshot coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CfpAdapter {
    /// Journal collection cards with explicit submission status.
    SpringerCollections,
    /// Named notices inside a journal's Call for papers section.
    ElsevierCalls,
    /// KeAi's journal-scoped call list and detail headings.
    KeaiCalls,
    /// Reviewed data retained pending a reliable automatic discovery adapter.
    SnapshotOnly,
}

/// Allowed original-source URL boundary, with exact hostname matching.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CfpUrlRule {
    /// Exact public publisher hostname.
    pub host: String,
    /// Approved path root; matching respects path segment boundaries.
    pub path_prefix: String,
}

/// One reviewed acquisition unit and its explicit journal identity binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CfpSourceConfig {
    /// Stable storage source identity.
    pub source_key: String,
    /// Canonical journal identity followed by maintained aliases.
    pub catalog_ids: Vec<String>,
    /// Original journal name.
    pub journal_title: String,
    /// Discovery list, never a search-engine snippet URL.
    pub discovery_url: String,
    /// Backend parser family or explicit snapshot-only capability.
    pub adapter: CfpAdapter,
    /// Reviewed configuration version.
    pub config_version: u32,
    /// Allowed discovery, redirect and detail URL boundaries.
    pub allowed_urls: Vec<CfpUrlRule>,
    /// Literal publisher journal identity strings required on the discovery page.
    pub identity_texts: Vec<String>,
    /// Verified literal negative statements that may establish an empty list.
    pub empty_statements: Vec<String>,
    /// Explanation when automated refresh is not yet supported.
    pub capability_note: Option<String>,
    /// Whether a filtered discovery list must retain previously verified notices.
    pub retains_previous_notices: bool,
}

impl CfpSourceConfig {
    /// Whether a URL remains inside the reviewed public-source boundary.
    pub fn permits_url(&self, url: &Url) -> bool {
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some_and(|port| !matches!(port, 80 | 443))
        {
            return false;
        }
        self.allowed_urls.iter().any(|rule| {
            url.host_str() == Some(rule.host.as_str())
                && (rule.path_prefix == "/"
                    || url.path() == rule.path_prefix
                    || url
                        .path()
                        .strip_prefix(rule.path_prefix.trim_end_matches('/'))
                        .is_some_and(|suffix| suffix.starts_with('/')))
        })
    }

    /// Whether an implemented discovery adapter can attempt an automatic update.
    pub fn can_refresh(&self) -> bool {
        self.adapter != CfpAdapter::SnapshotOnly
    }
}

/// Immutable backend registry; no runtime dependency on audit files or frontend code.
pub fn cfp_source_registry() -> &'static [CfpSourceConfig] {
    static REGISTRY: LazyLock<Vec<CfpSourceConfig>> = LazyLock::new(|| {
        let sources: Vec<CfpSourceConfig> =
            serde_json::from_str(include_str!("../../assets/cfp-sources.json"))
                .expect("bundled CFP registry must decode");
        for source in &sources {
            assert!(
                !source.catalog_ids.is_empty()
                    && source.config_version > 0
                    && source.permits_url(
                        &Url::parse(&source.discovery_url).expect("registered discovery URL")
                    ),
                "invalid bundled CFP registration"
            );
        }
        sources
    });
    &REGISTRY
}

/// Stable acquisition failures; no captured page or credentials enter diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CfpSourceError {
    /// The registered source has no reliable automatic adapter.
    Unsupported,
    /// The operation exceeded its finite elapsed-time budget.
    Deadline,
    /// A URL, redirect or address escaped the reviewed public-source policy.
    DisallowedUrl,
    /// An HTTP request, DNS lookup or connection failed.
    Request,
    /// The server returned a non-success status.
    HttpStatus(u16),
    /// Captcha, access-denied or JavaScript challenge was returned.
    Challenge,
    /// A source page or total capture exceeded its byte/page bound.
    TooLarge,
    /// The original text encoding could not be decoded without replacement.
    Encoding,
    /// A returned document was not a supported HTML/text/PDF representation.
    ContentType,
    /// Original journal identity or reliable notice boundaries were not established.
    Unrecognized,
    /// The optional browser/PDF helper is unavailable or failed.
    Helper,
    /// Cancellation was requested by the caller.
    Cancelled,
}

impl fmt::Display for CfpSourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => {
                formatter.write_str("Automatic discovery adapter is not yet available")
            }
            Self::Deadline => formatter.write_str("Source acquisition deadline exceeded"),
            Self::DisallowedUrl => formatter
                .write_str("Source URL or address is outside the registered public boundary"),
            Self::Request => formatter.write_str("Source request failed"),
            Self::HttpStatus(status) => write!(formatter, "Source returned HTTP {status}"),
            Self::Challenge => formatter.write_str("Source returned an access challenge"),
            Self::TooLarge => formatter.write_str("Source capture exceeded its size or page bound"),
            Self::Encoding => {
                formatter.write_str("Source encoding could not be decoded faithfully")
            }
            Self::ContentType => formatter.write_str("Unsupported source content type"),
            Self::Unrecognized => {
                formatter.write_str("Journal identity or complete CFP layout could not be verified")
            }
            Self::Helper => formatter.write_str("Optional source helper is unavailable or failed"),
            Self::Cancelled => formatter.write_str("Source acquisition cancelled"),
        }
    }
}
impl std::error::Error for CfpSourceError {}

/// A bounded original document and its final transport URL.
#[derive(Debug, Clone)]
pub struct CfpDocument {
    /// Final URL after registered redirects.
    pub final_url: String,
    /// HTML or extracted original text.
    pub text: String,
    /// Representation used for capture provenance.
    pub format: String,
}

/// Source transport seam used by live HTTP/helpers and deterministic local fixtures.
pub trait CfpTransport: Sync {
    /// Fetch a registered page within the supplied source deadline.
    fn fetch(
        &self,
        config: &CfpSourceConfig,
        url: &str,
        deadline: Instant,
    ) -> Result<CfpDocument, CfpSourceError>;
}

/// Complete verified source result ready for short transactional publication.
#[derive(Debug, Clone)]
pub struct CfpAcquisition {
    /// Exact original document captures with their final URLs.
    pub documents: Vec<CfpDocument>,
    /// Literal extracted notices, all bound to the registered journal.
    pub sources: Vec<CfpSource>,
    /// Verified scoped empty statements, when no calls are listed.
    pub empty_journals: Vec<CfpEmptyJournal>,
}

/// Discover current calls from the original list and follow only its bounded detail links.
pub fn acquire_cfp_source(
    transport: &impl CfpTransport,
    config: &CfpSourceConfig,
    checked_on: &str,
    deadline: Instant,
) -> Result<CfpAcquisition, CfpSourceError> {
    if !config.can_refresh() {
        return Err(CfpSourceError::Unsupported);
    }
    let document = transport.fetch(config, &config.discovery_url, deadline)?;
    let parsed = parse_cfp_page(config, &document, checked_on, true)?;
    let mut sources = parsed.sources;
    let empty_journals = parsed.empty_journals;
    let mut documents = vec![document];
    if parsed.detail_urls.len() > CFP_MAX_DETAIL_PAGES {
        return Err(CfpSourceError::TooLarge);
    }
    for url in parsed.detail_urls {
        if Instant::now() >= deadline {
            return Err(CfpSourceError::Deadline);
        }
        let document = transport.fetch(config, &url, deadline)?;
        let parsing_document = if document.format == "pdf_text" {
            let title = parsed
                .detail_titles
                .get(&url)
                .ok_or(CfpSourceError::Unrecognized)?;
            let identity_text = document
                .text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            let expected_title = title
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            if !identity_text.contains(&expected_title) {
                return Err(CfpSourceError::Unrecognized);
            }
            CfpDocument {
                final_url: document.final_url.clone(),
                text: format!("# {title}\n{}", document.text),
                format: "linked_pdf_text".into(),
            }
        } else {
            document.clone()
        };
        let detail = parse_cfp_page(config, &parsing_document, checked_on, false)?;
        if detail.sources.is_empty() {
            return Err(CfpSourceError::Unrecognized);
        }
        if let Some(expected_title) = parsed.detail_titles.get(&url) {
            if !detail
                .sources
                .iter()
                .any(|source| source.title.eq_ignore_ascii_case(expected_title))
            {
                return Err(CfpSourceError::Unrecognized);
            }
        }
        for source in detail.sources {
            if let Some(existing) = sources
                .iter_mut()
                .find(|existing| existing.title == source.title)
            {
                *existing = source;
            } else {
                sources.push(source);
            }
        }
        documents.push(document);
        if documents
            .iter()
            .map(|document| document.text.len())
            .sum::<usize>()
            > CFP_MAX_CAPTURE_BYTES
        {
            return Err(CfpSourceError::TooLarge);
        }
    }
    if sources.is_empty() && empty_journals.is_empty() {
        return Err(CfpSourceError::Unrecognized);
    }
    if !sources.is_empty() && !empty_journals.is_empty() {
        return Err(CfpSourceError::Unrecognized);
    }
    Ok(CfpAcquisition {
        documents,
        sources,
        empty_journals,
    })
}
