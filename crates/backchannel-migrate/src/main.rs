use anyhow::Context;
use sqlx::postgres::PgPoolOptions;
use std::env;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let database_url =
        env::var("DATABASE_URL").context("DATABASE_URL environment variable required")?;

    let schema = env::var("DATABASE_SCHEMA").unwrap_or_else(|_| "backchannel".to_string());

    tracing::info!("Connecting to database for migrations (schema: {})", schema);

    backchannel_core::db::validate_schema(&schema)?;
    let options =
        backchannel_core::db::connection_options(&database_url).context("Invalid DATABASE_URL")?;

    let schema_for_connect = schema.clone();
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .after_connect(move |conn, _meta| {
            let schema = schema_for_connect.clone();
            Box::pin(async move {
                // Create schema only if it doesn't exist. `CREATE SCHEMA IF NOT EXISTS`
                // checks CREATE-on-database privilege even when the schema already
                // exists, so a least-privilege migration role (USAGE+CREATE on its
                // own schema only) would fail. Check the catalog first instead.
                let exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = $1)",
                )
                .bind(&schema)
                .fetch_one(&mut *conn)
                .await?;
                if !exists {
                    sqlx::query(&format!("CREATE SCHEMA {}", schema))
                        .execute(&mut *conn)
                        .await?;
                }

                // Set search_path for this connection
                sqlx::query("SELECT set_config('search_path', $1, false)")
                    .bind(&schema)
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
