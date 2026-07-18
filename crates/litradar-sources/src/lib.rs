//! Source clients used by Rust index migration workflows.

pub mod cnki;
pub mod providers;
pub mod scholarly;
pub mod zjlib_cnki;

pub use cnki::{
    CnkiClient, CnkiFixtureData, CnkiSourceError, CnkiTransport, FixtureCnkiTransport,
    LiveCnkiConfig, LiveCnkiTransport,
};
pub use providers::{
    cnki_access_registration, cnki_index_registration, scholarly_access_registration,
    scholarly_index_registration, CnkiArticleAccessProvider, CnkiIndexProvider,
    ScholarlyArticleAccessProvider, ScholarlyIndexProvider, CNKI_PROVIDER_NAME,
    CNKI_REDIRECT_HOSTS, SCHOLARLY_PROVIDER_NAME, SCHOLARLY_REDIRECT_HOSTS,
};
pub use scholarly::{
    normalize_doi, FixtureScholarlyTransport, LiveScholarlyConfig, LiveScholarlyTransport,
    ScholarlyClient, ScholarlyFixtureData, ScholarlyRequest, ScholarlyRequestKind,
    ScholarlyTransport, ScholarlyWorksPage, SourceAttempt, SourceError,
    OPENALEX_MAX_WORKERS_PER_PROCESS, SEMANTIC_SCHOLAR_BATCH_SIZE,
};
pub use zjlib_cnki::{
    FixtureZjlibCnkiMode, FixtureZjlibCnkiTransport, LiveZjlibCnkiConfig, LiveZjlibCnkiTransport,
    ZhejiangLibraryCnkiClient, ZjlibCnkiArticleCandidate, ZjlibCnkiArticleIdentity,
    ZjlibCnkiCookie, ZjlibCnkiDownloadedPdf, ZjlibCnkiError, ZjlibCnkiQrLogin,
    ZjlibCnkiSearchResult, ZjlibCnkiTransport, DEFAULT_FULL_TEXT_MAXIMUM_BYTES,
};
