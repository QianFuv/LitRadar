//! Canonical LitRadar application entrypoint.

mod observability;
mod sqlite_cleanup;

/// Run the unified application and report process-level failures.
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let observability = match observability::initialize(&args) {
        Ok(observability) => observability,
        Err(error) => {
            eprintln!("{error}");
            sqlite_cleanup::cleanup_after_process(&args);
            std::process::exit(1);
        }
    };
    let cleanup_args = args.clone();
    let exit_code = if litradar::run(args).is_ok() { 0 } else { 1 };
    sqlite_cleanup::cleanup_after_process(&cleanup_args);
    observability.shutdown();
    if exit_code != 0 {
        std::process::exit(1);
    }
}
