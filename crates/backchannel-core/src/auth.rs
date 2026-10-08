use crate::config::Config;
use crate::error::AppError;
use crate::models::Agent;
use crate::token::verify_token_hash;
use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct AuthState {
    pub pool: PgPool,
    pub config: Arc<Config>,
    pub oauth: Option<crate::oauth::OAuthVerifier>,
}

#[derive(Clone, Debug)]
pub struct AuthenticatedAgent {
    pub agent: Agent,
}

#[derive(Clone, Debug)]
pub struct AdminAuth;

/// Extract bearer token from Authorization header
fn extract_bearer_token(headers: &axum::http::HeaderMap) -> Result<String, AppError> {
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("Missing Authorization header".to_string()))?;

    if !auth_header.starts_with("Bearer ") {
        return Err(AppError::Unauthorized(
            "Invalid Authorization header format".to_string(),
        ));
    }

    Ok(auth_header[7..].to_string())
}

/// Authenticate agent token
pub async fn authenticate_agent(
    State(state): State<AuthState>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let token = extract_bearer_token(req.headers())?;

    // Redact token from logs
    tracing::debug!("Authenticating agent request");

    let token_hash = crate::token::hash_token(&token);

    let agent = sqlx::query_as::<_, Agent>(
        "SELECT id, name, token_hash, enabled, created_at FROM agents WHERE token_hash = $1",
    )
    .bind(&token_hash)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::Unauthorized("Invalid token".to_string()))?;

    if !agent.enabled {
        return Err(AppError::Unauthorized("Agent is disabled".to_string()));
    }

    // Apply rate limiting
    let rate_limit = state.config.default_rate_limit_per_minute;
    check_rate_limit(&state.pool, &agent.id.to_string(), rate_limit).await?;

    req.extensions_mut().insert(AuthenticatedAgent { agent });

    Ok(next.run(req).await)
}

/// MCP additionally accepts OAuth access tokens. REST/admin remain legacy-only.
pub async fn authenticate_mcp(
    State(state): State<AuthState>,
    mut req: Request,
    next: Next,
) -> Response {
    use crate::oauth::{challenge, OAuthFailure};
    use axum::{http::StatusCode, response::IntoResponse};
    let Some(verifier) = &state.oauth else {
        return match authenticate_agent(State(state), req, next).await {
            Ok(response) => response,
            Err(error) => error.into_response(),
        };
    };
    if !req
        .headers()
        .contains_key(axum::http::header::AUTHORIZATION)
    {
        return crate::oauth::missing_credentials_challenge(&verifier.config);
    }
    let token = match extract_bearer_token(req.headers()) {
        Ok(token) => token,
        Err(_) => return challenge(&verifier.config, StatusCode::UNAUTHORIZED),
    };
    // Existing agent tokens never contain dots. JWTs must not be interpreted as
    // legacy credentials, nor cause a database lookup before signature checks.
    let agent_result = if token.contains('.') {
        let agent_id = match verifier.verify(&token).await {
            Ok(id) => id,
            Err(OAuthFailure::InvalidToken) => {
                return challenge(&verifier.config, StatusCode::UNAUTHORIZED)
            }
            Err(OAuthFailure::InsufficientScope) => {
                return challenge(&verifier.config, StatusCode::FORBIDDEN)
            }
            Err(OAuthFailure::Unavailable) => {
                return AppError::ServiceUnavailable("OAuth signing keys unavailable".into())
                    .into_response()
            }
        };
        sqlx::query_as::<_, Agent>(
            "SELECT id, name, token_hash, enabled, created_at FROM agents WHERE id = $1",
        )
        .bind(agent_id)
        .fetch_optional(&state.pool)
        .await
    } else {
        sqlx::query_as::<_, Agent>(
            "SELECT id, name, token_hash, enabled, created_at FROM agents WHERE token_hash = $1",
        )
        .bind(crate::token::hash_token(&token))
        .fetch_optional(&state.pool)
        .await
    };
    let agent = match agent_result {
        Ok(Some(agent)) if agent.enabled => agent,
        Ok(_) => return challenge(&verifier.config, StatusCode::UNAUTHORIZED),
        Err(error) => return AppError::from(error).into_response(),
    };
    if let Err(error) = check_rate_limit(
        &state.pool,
        &agent.id.to_string(),
        state.config.default_rate_limit_per_minute,
    )
    .await
    {
        return error.into_response();
    }
    req.extensions_mut().insert(AuthenticatedAgent { agent });
    next.run(req).await
}

/// Authenticate admin token
pub async fn authenticate_admin(
    State(state): State<AuthState>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let token = extract_bearer_token(req.headers())?;

    tracing::debug!("Authenticating admin request");

    let provided_hash = crate::token::hash_token(&token);

    if !verify_token_hash(&provided_hash, &state.config.admin_token_sha256) {
        return Err(AppError::Unauthorized("Invalid admin token".to_string()));
    }

    // Apply admin rate limiting
    let rate_limit = state.config.admin_rate_limit_per_minute;
    check_rate_limit(&state.pool, "admin", rate_limit).await?;

    req.extensions_mut().insert(AdminAuth);

    Ok(next.run(req).await)
}

/// Check rate limit using Postgres atomic buckets
async fn check_rate_limit(pool: &PgPool, identity: &str, limit: u32) -> Result<(), AppError> {
    let current_minute = chrono::Utc::now().format("%Y-%m-%d %H:%M").to_string();

    // Atomic increment and check
    let result = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO rate_limit_buckets (identity, minute_bucket, counter)
        VALUES ($1, $2, 1)
        ON CONFLICT (identity, minute_bucket)
        DO UPDATE SET counter = rate_limit_buckets.counter + 1
        RETURNING counter
        "#,
    )
    .bind(identity)
    .bind(&current_minute)
    .fetch_one(pool)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "Rate limit check failed");
        AppError::ServiceUnavailable("Rate limit check unavailable".to_string())
    })?;

    if result > limit as i64 {
        return Err(AppError::TooManyRequests {
            retry_after_seconds: 60,
        });
    }

    Ok(())
}
