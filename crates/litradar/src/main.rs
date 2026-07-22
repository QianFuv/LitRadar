//! Canonical LitRadar application entrypoint.

mod observability;

/// Run the unified application and report process-level failures.
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let observability = match observability::initialize(&args) {
        Ok(observability) => observability,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let exit_code = if litradar::run(args).is_ok() { 0 } else { 1 };
    observability.shutdown();
    if exit_code != 0 {
        std::process::exit(1);
    }
}
