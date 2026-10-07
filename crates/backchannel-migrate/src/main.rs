use anyhow::Context;
use sqlx::postgres::PgPoolOptions;
use std::env;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let database_url =
        env::var("DATABASE_URL").context("DATABASE_URL environment variable required")?;

    tracing::info!("Connecting to database for migrations");

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .context("Failed to connect to database")?;

    tracing::info!("Running migrations");

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .context("Failed to run migrations")?;

    tracing::info!("Migrations completed successfully");

    Ok(())
}
