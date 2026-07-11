//! Standalone notification binary entrypoint.

use std::env;

fn main() {
    if let Err(error) = litradar_cli::run_notify_command(env::args().skip(1).collect()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
