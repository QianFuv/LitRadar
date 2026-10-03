//! Observe original Crossref state, identity and disposable workset behavior.
use std::io::{self, BufRead};
#[allow(dead_code)]
mod providers {
    use litradar_domain::{
        normalize_bibliographic_label, normalize_bibliographic_text, normalize_contract_date,
        normalize_contract_text,
    };
    use serde::{Deserialize, Serialize};
    use serde_json::Value;
    include!("../../../output/migration/execution/sources-oracle/workset_providers.rs");
}
mod scholarly {
    pub use litradar_sources::scholarly::{CrossrefQuery, CrossrefWorksPage};
    include!("../../../output/migration/execution/sources-oracle/workset_rows.rs");
}
#[allow(dead_code)]
#[path = "../../../output/migration/execution/sources-oracle/crossref_workset.rs"]
mod workset;
fn main() {
    for line in io::stdin().lock().lines() {
        let mut request: serde_json::Value = serde_json::from_str(&line.unwrap()).unwrap();
        request["output"] = serde_json::json!(
            serde_json::to_string(&workset::observe_migration(&request)).unwrap()
        );
        println!("{}", serde_json::to_string(&request).unwrap());
    }
}
