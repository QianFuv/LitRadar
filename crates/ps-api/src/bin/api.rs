//! Legacy API binary entrypoint.

use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        println!("{}", ps_api::config::api_usage());
        return Ok(());
    }
    ps_api::serve(ps_api::config::ApiConfig::from_args(args)?).await
}
