pub mod agents;
pub mod conversations;
pub mod messages;

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use std::str::FromStr;

/// Create a database connection pool with bounded connections
/// Configured for Supabase with session pooler (port 5432) to support prepared statements
pub async fn create_pool(database_url: &str, schema: &str) -> Result<PgPool, sqlx::Error> {
    let options = PgConnectOptions::from_str(database_url)?;

    // Note: SSL mode is controlled by the connection URL (sslmode parameter)
    // Production Supabase connections should use sslmode=require
    // Test environments may use sslmode=disable

    // Set search_path to use the specified schema
    let options = options.application_name("backchannel");

    let schema = schema.to_string();

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .min_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .after_connect(move |conn, _meta| {
            let schema = schema.clone();
            Box::pin(async move {
                sqlx::query(&format!("SET search_path TO {}", schema))
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect_with(options)
        .await?;

    Ok(pool)
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
