pub mod agents;
pub mod conversations;
pub mod messages;

use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use sqlx::PgPool;
use std::str::FromStr;

/// Create a database connection pool with bounded connections
/// Configured for Supabase with session pooler (port 5432) to support prepared statements
pub async fn create_pool(database_url: &str, schema: &str) -> Result<PgPool, sqlx::Error> {
    let options = connection_options(database_url)?;
    validate_schema(schema)?;

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
                sqlx::query("SELECT set_config('search_path', $1, false)")
                    .bind(&schema)
                    .execute(&mut *conn)
                    .await?;
                sqlx::query("SET statement_timeout = '5s'")
                    .execute(&mut *conn)
                    .await?;
                sqlx::query("SET lock_timeout = '2s'")
                    .execute(&mut *conn)
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

/// Remote endpoints always require both CA and hostname verification. Only explicit
/// loopback URLs with sslmode=disable are accepted for isolated local tests.
pub fn connection_options(database_url: &str) -> Result<PgConnectOptions, sqlx::Error> {
    let url = url::Url::parse(database_url)
        .map_err(|_| sqlx::Error::Configuration("Invalid database URL".into()))?;
    if url
        .query_pairs()
        .any(|(key, _)| matches!(key.as_ref(), "host" | "hostaddr"))
    {
        return Err(sqlx::Error::Configuration(
            "Database endpoint overrides are not allowed".into(),
        ));
    }
    let host = url.host_str().unwrap_or_default();
    let local = matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1");
    let modes: Vec<_> = url
        .query_pairs()
        .filter(|(k, _)| k == "sslmode" || k == "ssl-mode")
        .map(|(_, v)| v.into_owned())
        .collect();
    if modes.len() != 1 || (modes[0] != "verify-full" && !(local && modes[0] == "disable")) {
        return Err(sqlx::Error::Configuration(
            "Database requires sslmode=verify-full; only loopback tests may use sslmode=disable"
                .into(),
        ));
    }
    let options = PgConnectOptions::from_str(database_url)?;
    Ok(options.ssl_mode(if local && modes[0] == "disable" {
        PgSslMode::Disable
    } else {
        PgSslMode::VerifyFull
    }))
}
pub fn validate_schema(schema: &str) -> Result<(), sqlx::Error> {
    if schema.is_empty()
        || schema.len() > 63
        || !schema
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
        || !schema.as_bytes()[0].is_ascii_alphabetic()
    {
        return Err(sqlx::Error::Configuration(
            "Invalid database schema name".into(),
        ));
    }
    Ok(())
}
#[cfg(test)]
mod security_tests {
    use super::*;
    #[test]
    fn remote_database_tls_fails_closed() {
        for mode in [
            "",
            "?sslmode=disable",
            "?sslmode=prefer",
            "?sslmode=require",
            "?sslmode=verify-ca",
            "?sslmode=verify-full&sslmode=disable",
        ] {
            assert!(connection_options(&format!("postgres://test@db.example/test{mode}")).is_err());
        }
        assert!(connection_options("postgres://test@db.example/test?sslmode=verify-full").is_ok());
        assert!(connection_options("postgres://test@127.0.0.1/test?sslmode=disable").is_ok());
        assert!(connection_options("postgres://test@127.0.0.1/test").is_err());
        assert!(connection_options(
            "postgres://test@127.0.0.1/test?sslmode=disable&host=db.example"
        )
        .is_err());
        assert!(connection_options(
            "postgres://test@127.0.0.1/test?sslmode=disable&hostaddr=198.51.100.1"
        )
        .is_err());
        assert!(connection_options(
            "postgres://test@db.example/test?sslmode=verify-full&ssl-mode=require"
        )
        .is_err());
    }
    #[test]
    fn rejects_schema_syntax() {
        assert!(validate_schema("backchannel_test").is_ok());
        for value in ["", "a;DROP TABLE agents", "a,b", "1abc"] {
            assert!(validate_schema(value).is_err());
        }
    }
}
