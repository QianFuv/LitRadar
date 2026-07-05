//! Source clients used by Rust index migration workflows.

pub mod cnki;
pub mod scholarly;

pub use cnki::{
    CnkiClient, CnkiFixtureData, CnkiSourceError, CnkiTransport, FixtureCnkiTransport,
    LiveCnkiConfig, LiveCnkiTransport,
};
pub use scholarly::{
    normalize_doi, FixtureScholarlyTransport, LiveScholarlyConfig, LiveScholarlyTransport,
    ScholarlyClient, ScholarlyFixtureData, ScholarlyRequest, ScholarlyRequestKind,
    ScholarlyTransport, SourceAttempt, SourceError, SEMANTIC_SCHOLAR_BATCH_SIZE,
};
