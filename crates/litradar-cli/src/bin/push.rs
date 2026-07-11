//! Standalone tracking push binary entrypoint.

use std::env;

fn main() {
    if let Err(error) = litradar_cli::run_push_command(env::args().skip(1).collect()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
