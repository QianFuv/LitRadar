//! Rust scholarly index migration workflow.

pub mod changes;
pub mod cnki;
pub mod control;
pub mod identity;
pub mod live;
pub mod manifest;
pub mod schema;
pub mod scholarly;
pub mod stats;
pub mod transforms;

pub use cnki::{run_cnki_fixture_index, CnkiIndexConfig, CnkiIndexOutcome};
pub use litradar_sources::LiveScholarlyConfig;
pub use live::{
    run_live_index, run_live_index_worker_from_file_path, LiveCsvIndexOutcome, LiveIndexConfig,
    LiveIndexOutcome,
};
pub use scholarly::{run_scholarly_fixture_index, ScholarlyIndexConfig, ScholarlyIndexOutcome};
