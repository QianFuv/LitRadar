//! Source clients used by Rust index migration workflows.

pub mod cnki;
pub mod scholarly;

pub use cnki::{CnkiClient, CnkiFixtureData, CnkiSourceError, FixtureCnkiTransport};
pub use scholarly::{
    normalize_doi, FixtureScholarlyTransport, ScholarlyClient, ScholarlyFixtureData,
    ScholarlyRequest, ScholarlyRequestKind, SourceAttempt, SourceError,
    SEMANTIC_SCHOLAR_BATCH_SIZE,
};
