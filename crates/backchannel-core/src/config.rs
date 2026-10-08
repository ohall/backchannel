use std::env;

#[derive(Clone)]
pub struct Config {
    pub oauth: Option<crate::oauth::OAuthConfig>,
    pub database_url: String,
    pub database_schema: String,
    pub admin_token_sha256: String,
    pub viewer_token_sha256: Option<String>,
    pub default_rate_limit_per_minute: u32,
    pub admin_rate_limit_per_minute: u32,
    pub max_body_size_bytes: usize,
    pub max_message_body_size_bytes: usize,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let admin_token_sha256 = env::var("ADMIN_TOKEN_SHA256")
            .map_err(|_| "ADMIN_TOKEN_SHA256 environment variable required".to_string())?;
        let viewer_token_sha256 = match env::var("VIEWER_TOKEN_SHA256") {
            Ok(value) => Some(value.to_ascii_lowercase()),
            Err(env::VarError::NotPresent) => None,
            Err(_) => return Err("VIEWER_TOKEN_SHA256 must be valid Unicode".into()),
        };
        validate_viewer_hash(viewer_token_sha256.as_deref(), &admin_token_sha256)?;
        Ok(Config {
            oauth: match env::var("OAUTH_CONFIG") {
                Ok(value) => Some(crate::oauth::OAuthConfig::from_json(&value)?),
                Err(env::VarError::NotPresent) => None,
                Err(_) => return Err("OAUTH_CONFIG must be valid Unicode".into()),
            },
            database_url: env::var("DATABASE_URL")
                .map_err(|_| "DATABASE_URL environment variable required".to_string())?,
            database_schema: env::var("DATABASE_SCHEMA")
                .unwrap_or_else(|_| "backchannel".to_string()),
            admin_token_sha256,
            viewer_token_sha256,
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

/// Dedicated viewer credentials must never grant administrator authority.
pub fn validate_viewer_hash(viewer: Option<&str>, admin: &str) -> Result<(), String> {
    if let Some(hash) = viewer {
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("VIEWER_TOKEN_SHA256 must be a 64-character SHA-256 hex digest".into());
        }
        if hash.eq_ignore_ascii_case(admin) {
            return Err("VIEWER_TOKEN_SHA256 must differ from ADMIN_TOKEN_SHA256".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_viewer_hash;
    #[test]
    fn viewer_hash_is_optional_and_fail_closed() {
        let admin = "a".repeat(64);
        assert!(validate_viewer_hash(None, &admin).is_ok());
        assert!(validate_viewer_hash(Some(&"b".repeat(64)), &admin).is_ok());
        for invalid in [
            "".into(),
            "a".repeat(63),
            "g".repeat(64),
            "A".repeat(64),
            "a".repeat(64),
        ] {
            assert!(validate_viewer_hash(Some(&invalid), &admin).is_err());
        }
    }
}
