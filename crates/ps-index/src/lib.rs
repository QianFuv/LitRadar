//! Rust scholarly index migration workflow.

pub mod manifest;
pub mod schema;
pub mod scholarly;
pub mod stats;
pub mod transforms;

pub use scholarly::{run_scholarly_fixture_index, ScholarlyIndexConfig, ScholarlyIndexOutcome};
