//! Canonical LitRadar application entrypoint.

mod observability;

/// Run the unified application and report process-level failures.
fn main() {
    let observability = match observability::initialize() {
        Ok(observability) => observability,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let exit_code = if litradar::run(std::env::args().skip(1).collect()).is_ok() {
        0
    } else {
        1
    };
    observability.shutdown();
    if exit_code != 0 {
        std::process::exit(1);
    }
}
