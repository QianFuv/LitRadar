//! Observe original library behavior with a deterministic seconds clock.
use std::io::{self, BufRead};
#[allow(dead_code)]
#[path = "../../../crates/litradar-sources/src/request_deadline.rs"]
mod request_deadline;
#[allow(dead_code)]
#[path = "../../../crates/litradar-sources/src/response_body.rs"]
mod response_body;
mod provider_proxy {
    pub use litradar_sources::provider_proxy::*;
}
#[allow(dead_code)]
#[path = "../../../output/migration/execution/sources-oracle/zjlib.rs"]
mod zjlib;

fn main() {
    for line in io::stdin().lock().lines() {
        let mut request: serde_json::Value = serde_json::from_str(&line.unwrap()).unwrap();
        request["output"] =
            serde_json::json!(serde_json::to_string(&zjlib::observe_migration(&request)).unwrap());
        println!("{}", serde_json::to_string(&request).unwrap());
    }
}
