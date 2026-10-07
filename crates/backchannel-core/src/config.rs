use std::env;

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub admin_token_sha256: String,
    pub default_rate_limit_per_minute: u32,
    pub admin_rate_limit_per_minute: u32,
    pub max_body_size_bytes: usize,
    pub max_message_body_size_bytes: usize,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        Ok(Config {
            database_url: env::var("DATABASE_URL")
                .map_err(|_| "DATABASE_URL environment variable required".to_string())?,
            admin_token_sha256: env::var("ADMIN_TOKEN_SHA256")
                .map_err(|_| "ADMIN_TOKEN_SHA256 environment variable required".to_string())?,
            default_rate_limit_per_minute: env::var("RATE_LIMIT_PER_MINUTE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(120),
            admin_rate_limit_per_minute: env::var("ADMIN_RATE_LIMIT_PER_MINUTE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(300),
            max_body_size_bytes: 64 * 1024,         // 64 KiB
            max_message_body_size_bytes: 32 * 1024, // 32 KiB
        })
    }
}
