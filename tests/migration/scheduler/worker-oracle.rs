//! Invoke the unchanged scheduler source plus a read-only observation wrapper.
use std::io::{self, BufRead};
fn main() {
    for line in io::stdin().lock().lines() {
        let input = serde_json::from_str(&line.unwrap()).unwrap();
        println!("{}", scheduler::observe(input))
    }
}
