//! Provider-neutral canonical article indexing workflow.

pub mod changes;
pub mod control;
pub mod identity;
pub mod live;
pub mod schema;
pub mod stats;
pub mod transforms;

pub use litradar_sources::LiveScholarlyConfig;
pub use live::{
    run_live_index, run_live_index_worker_from_file_path, LiveCsvIndexOutcome, LiveIndexConfig,
    LiveIndexOutcome,
};
