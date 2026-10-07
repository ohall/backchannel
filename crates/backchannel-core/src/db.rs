pub mod agents;
pub mod conversations;
pub mod messages;

use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use sqlx::PgPool;
use std::str::FromStr;

/// Create a database connection pool with bounded connections
/// Configured for Supabase with session pooler (port 5432) to support prepared statements
pub async fn create_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
    let mut options = PgConnectOptions::from_str(database_url)?;

    // Ensure SSL is required with certificate verification for Supabase
    // Always set to Require mode for production use
    options = options.ssl_mode(PgSslMode::Require);

    PgPoolOptions::new()
        .max_connections(10)
        .min_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect_with(options)
        .await
}

/// Advisory lock key for message ordering (arbitrary constant)
const MESSAGE_ORDERING_LOCK_KEY: i64 = 1234567890;

/// Acquire advisory lock for message creation to ensure ordering
pub async fn acquire_message_lock(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(MESSAGE_ORDERING_LOCK_KEY)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
