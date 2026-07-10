//! Standalone local administrator maintenance entrypoint.

use std::env;

fn main() {
    if let Err(error) = ps_cli::run_admin_command(env::args().skip(1).collect()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
