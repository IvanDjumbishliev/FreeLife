use std::net::SocketAddr;

use tracing_subscriber::EnvFilter;

const DEFAULT_ADDR: &str = "0.0.0.0:8787";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let addr: SocketAddr = std::env::var("FREELIFE_RELAY_ADDR")
        .unwrap_or_else(|_| DEFAULT_ADDR.into())
        .parse()?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "freelife relay listening");
    axum::serve(listener, freelife_relay::router(Default::default())).await?;
    Ok(())
}
