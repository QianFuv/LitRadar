//! Legacy API binary entrypoint.

use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    ps_api::serve_from_env().await
}
