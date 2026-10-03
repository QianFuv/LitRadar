//! Export independent JSON syntax and MCP envelope results from the frozen Rust dependencies.
use std::io::{BufRead, Write};

fn main() -> std::io::Result<()> {
    let mut output = std::io::BufWriter::new(std::io::stdout());
    for line in std::io::stdin().lock().lines() {
        let hex = line?;
        let bytes: Vec<u8> = hex.as_bytes().chunks_exact(2).map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()
        }).collect();
        let syntax = serde_json::from_reader::<_, serde_json::Value>(bytes.as_slice())
            .err().map(|error| error.to_string());
        let envelope = serde_json::from_reader::<_, rmcp::model::ClientJsonRpcMessage>(bytes.as_slice())
            .err().map(|error| error.to_string());
        writeln!(output, "{}", serde_json::json!({"hex": hex, "syntax": syntax, "envelope": envelope}))?;
    }
    output.flush()
}
