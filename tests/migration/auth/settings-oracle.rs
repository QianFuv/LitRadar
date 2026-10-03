//! Observe the original runtime-setting parser without editing the Rust application.
use litradar_storage::{RuntimeSettingKey, parse_runtime_setting};
use std::io::{BufRead, Write};

fn observe() -> std::io::Result<()> {
    let mut output = std::io::BufWriter::new(std::io::stdout());
    for line in std::io::stdin().lock().lines() {
        let mut value: serde_json::Value = serde_json::from_str(&line?).unwrap();
        let field = value["field"].as_str().unwrap();
        let input = value["input"].as_str().unwrap();
        let key = RuntimeSettingKey::from_field(field).unwrap();
        match parse_runtime_setting(key, input) {
            Ok(parsed) => value["output"] = parsed.into_text().into(),
            Err(error) => value["error"] = error.to_string().into(),
        }
        writeln!(output, "{value}")?;
    }
    output.flush()
}

fn main() -> std::io::Result<()> {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(observe)?
        .join()
        .expect("settings oracle thread must complete")
}
