use anyhow::Context;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::env;
use std::str::FromStr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let database_url =
        env::var("DATABASE_URL").context("DATABASE_URL environment variable required")?;

    let schema = env::var("DATABASE_SCHEMA").unwrap_or_else(|_| "backchannel".to_string());

    tracing::info!("Connecting to database for migrations (schema: {})", schema);

    let options =
        PgConnectOptions::from_str(&database_url).context("Invalid DATABASE_URL")?;

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .after_connect(move |conn, _meta| {
            let schema = schema.clone();
            Box::pin(async move {
                // Create schema if it doesn't exist
                sqlx::query(&format!("CREATE SCHEMA IF NOT EXISTS {}", schema))
                    .execute(&mut *conn)
                    .await?;

                // Set search_path for this connection
                sqlx::query(&format!("SET search_path TO {}", schema))
                    .execute(&mut *conn)
                    .await?;

                Ok(())
            })
        })
        .connect_with(options)
        .await
        .context("Failed to connect to database")?;

    tracing::info!("Running migrations in schema: {}", schema);

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .context("Failed to run migrations")?;

    tracing::info!("Migrations completed successfully");

    Ok(())
}
