//! Verify current Go envelopes with the unchanged original Rust secret codec.
use litradar_storage::SecretCodec;
use std::io::{BufRead, Write};

/// Authenticate every synthetic observation, including deliberately invalid contexts and keys.
fn main() -> std::io::Result<()> {
    let mut count = 0;
    for line in std::io::stdin().lock().lines() {
        let value: serde_json::Value = serde_json::from_str(&line?).unwrap();
        let codec = SecretCodec::from_key([value["key"].as_u64().unwrap() as u8; 32]);
        let result = codec.decrypt(
            value["stored"].as_str().unwrap(),
            value["context"].as_str().unwrap(),
        );
        if value["reject"].as_bool().unwrap() {
            assert!(result.is_err(), "invalid envelope authenticated");
        } else {
            assert_eq!(result.unwrap(), value["plaintext"].as_str().unwrap());
        }
        count += 1;
    }
    assert!(count >= 28, "interoperability corpus is incomplete");
    writeln!(std::io::stdout(), "{{\"verified\":{count}}}")
}
