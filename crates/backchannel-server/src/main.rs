use anyhow::Context;
use backchannel_core::{create_router, db, Config};
use std::net::SocketAddr;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "backchannel_server=debug,backchannel_core=debug,tower_http=debug".into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config =
        Config::from_env().map_err(|e| anyhow::anyhow!("Failed to load configuration: {}", e))?;

    tracing::info!("Connecting to database (schema: {})", config.database_schema);
    let pool = db::create_pool(&config.database_url, &config.database_schema)
        .await
        .context("Failed to create database pool")?;

    let app = create_router(pool, config);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    tracing::info!("Starting server on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("Failed to bind to address")?;

    axum::serve(listener, app).await.context("Server error")?;

    Ok(())
}
