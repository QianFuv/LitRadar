//! Observe private index contracts in an unchanged provider-module copy with a fixed wall clock.
pub use litradar_sources::*;
use std::io::{self, BufRead};
pub mod scholarly {
    pub use litradar_sources::scholarly::*;
    include!("../../../output/migration/execution/sources-oracle/workset_rows.rs");
}
#[allow(dead_code)]
#[path = "../../../output/migration/execution/sources-oracle/crossref_workset.rs"]
mod crossref_workset;
#[allow(dead_code)]
#[path = "../../../output/migration/execution/sources-oracle/index_providers.rs"]
pub mod providers;
fn main() {
    for line in io::stdin().lock().lines() {
        let mut request: serde_json::Value = serde_json::from_str(&line.unwrap()).unwrap();
        request["output"] = providers::observe_index_migration(&request);
        println!("{request}");
    }
}
