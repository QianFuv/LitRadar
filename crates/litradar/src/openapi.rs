//! OpenAPI document command for the unified application.

use std::error::Error;
use std::fs;

/// Emit or write the generated OpenAPI document.
///
/// # Arguments
///
/// * `args` - OpenAPI command arguments without the subcommand name.
///
/// # Returns
///
/// Result indicating whether document generation and output succeeded.
pub(crate) fn run(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    if args
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        println!("{}", usage());
        return Ok(());
    }
    let json = litradar_api::generated_openapi_json()?;
    match args.as_slice() {
        [] => print!("{json}"),
        [flag, output_path] if flag == "--output" => fs::write(output_path, json)?,
        _ => return Err(usage().into()),
    }
    Ok(())
}

fn usage() -> &'static str {
    "Usage: litradar openapi [--output PATH]"
}
