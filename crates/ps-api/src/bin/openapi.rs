//! Deterministic OpenAPI document emitter for frontend contract generation.

use std::env;
use std::error::Error;
use std::fs;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let json = ps_api::generated_openapi_json()?;
    match args.as_slice() {
        [] => print!("{json}"),
        [flag, output_path] if flag == "--output" => fs::write(output_path, json)?,
        _ => return Err("usage: openapi [--output PATH]".into()),
    }
    Ok(())
}
