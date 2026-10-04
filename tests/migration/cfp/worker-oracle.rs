//! Observe original CFP worker behavior without replacing production control flow.
use std::io::{self, BufRead};
fn main() {
    for line in io::stdin().lock().lines() {
        let input = serde_json::from_str(&line.unwrap()).unwrap();
        println!("{}", cfp::observe(&input));
    }
}
