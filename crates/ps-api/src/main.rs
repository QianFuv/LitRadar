//! Binary entrypoint for the Rust API server.

use std::error::Error;

use ps_api::build_router;
use ps_api::config::ApiConfig;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = ApiConfig::from_env()?;
    let bind_address = config.bind_address();
    let listener = TcpListener::bind(&bind_address).await?;
    println!("ps-api listening on {}", listener.local_addr()?);

    axum::serve(listener, build_router(config))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        eprintln!("failed to install Ctrl+C handler: {error}");
    }
}
