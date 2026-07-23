//! Source clients used by Rust index migration workflows.

pub mod cnki_domestic;
pub mod cnki_oversea;
pub mod providers;
pub mod scholarly;
pub mod zjlib;

pub use cnki_domestic::{
    domestic_journal_search_form, parse_domestic_article_detail,
    parse_domestic_issue_articles, parse_domestic_journal_detail,
    parse_domestic_journal_search_results, parse_domestic_year_issues,
    DomesticCnkiFixtureData, DomesticCnkiSourceError, DOMESTIC_KNS_BASE_URL,
    DOMESTIC_NAVI_BASE_URL,
};
pub use cnki_oversea::{
    CnkiClient, CnkiFixtureData, CnkiSourceError, CnkiTransport, FixtureCnkiTransport,
    LiveCnkiConfig, LiveCnkiTransport,
};
pub use providers::{
    built_in_provider_capabilities, cnki_oversea_access_registration, cnki_oversea_index_registration,
    scholarly_access_registration, scholarly_index_registration, CnkiArticleAccessProvider,
    CnkiIndexProvider, ScholarlyArticleAccessProvider, ScholarlyIndexProvider, CNKI_OVERSEA_PROVIDER_NAME,
    CNKI_REDIRECT_HOSTS, SCHOLARLY_PROVIDER_NAME, SCHOLARLY_REDIRECT_HOSTS,
    ZJLIB_PROVIDER_NAME,
};
pub use scholarly::{
    normalize_doi, FixtureScholarlyTransport, LiveScholarlyConfig, LiveScholarlyTransport,
    ScholarlyClient, ScholarlyFixtureData, ScholarlyRequest, ScholarlyRequestKind,
    ScholarlyTransport, ScholarlyWorksPage, SourceAttempt, SourceError,
    OPENALEX_MAX_WORKERS_PER_PROCESS, SEMANTIC_SCHOLAR_BATCH_SIZE,
};
pub use zjlib::{
    FixtureZjlibCnkiMode, FixtureZjlibCnkiTransport, LiveZjlibCnkiConfig, LiveZjlibCnkiTransport,
    ZhejiangLibraryCnkiClient, ZjlibCnkiArticleCandidate, ZjlibCnkiArticleIdentity,
    ZjlibCnkiCookie, ZjlibCnkiDownloadedPdf, ZjlibCnkiError, ZjlibCnkiQrLogin,
    ZjlibCnkiSearchResult, ZjlibCnkiTransport, DEFAULT_FULL_TEXT_MAXIMUM_BYTES,
};
