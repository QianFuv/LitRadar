//! Canonical LitRadar application entrypoint.

/// Run the unified application and report process-level failures.
#[tokio::main]
async fn main() {
    if let Err(error) = litradar::run(std::env::args().skip(1).collect()).await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
