//! Standalone scheduler binary entrypoint.

use std::env;

fn main() {
    if let Err(error) = ps_cli::run_scheduler_command(env::args().skip(1).collect()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
