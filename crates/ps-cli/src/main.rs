//! Rust backend command entrypoints.

use std::env;

fn main() {
    if let Err(error) = ps_cli::run_ps_cli(env::args().skip(1).collect()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
