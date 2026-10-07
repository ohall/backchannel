use crate::error::AppError;
use crate::models::{Agent, AgentPublic, PaginatedResponse};
use crate::token;
use sqlx::PgPool;
use uuid::Uuid;

/// Create a new agent with generated token
pub async fn create_agent(pool: &PgPool, name: &str) -> Result<(Agent, String), AppError> {
    // Validate name
    if !is_valid_agent_name(name) {
        return Err(AppError::BadRequest(
            "Agent name must be lowercase, 1-64 characters, letters/digits/hyphens only"
                .to_string(),
        ));
    }

    let token = token::generate_token()
        .map_err(|e| AppError::Internal(format!("Failed to generate token: {}", e)))?;

    let token_hash = token::hash_token(&token);
    let id = Uuid::new_v4();

    let agent = sqlx::query_as::<_, Agent>(
        r#"
        INSERT INTO agents (id, name, token_hash, enabled, created_at)
        VALUES ($1, $2, $3, true, NOW())
        RETURNING id, name, token_hash, enabled, created_at
        "#,
    )
    .bind(id)
    .bind(name)
    .bind(&token_hash)
    .fetch_one(pool)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref db_err) = e {
            if db_err.constraint() == Some("agents_name_key") {
                return AppError::Conflict("Agent name already exists".to_string());
            }
        }
        e.into()
    })?;

    Ok((agent, token))
}

/// Rotate agent token
pub async fn rotate_agent_token(pool: &PgPool, agent_id: Uuid) -> Result<String, AppError> {
    let token = token::generate_token()
        .map_err(|e| AppError::Internal(format!("Failed to generate token: {}", e)))?;

    let token_hash = token::hash_token(&token);

    let result = sqlx::query("UPDATE agents SET token_hash = $1 WHERE id = $2")
        .bind(&token_hash)
        .bind(agent_id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Agent not found".to_string()));
    }

    Ok(token)
}

/// Update agent enabled status
pub async fn update_agent_enabled(
    pool: &PgPool,
    agent_id: Uuid,
    enabled: bool,
) -> Result<Agent, AppError> {
    let agent = sqlx::query_as::<_, Agent>(
        r#"
        UPDATE agents
        SET enabled = $1
        WHERE id = $2
        RETURNING id, name, token_hash, enabled, created_at
        "#,
    )
    .bind(enabled)
    .bind(agent_id)
    .fetch_one(pool)
    .await?;

    Ok(agent)
}

/// List enabled agents with pagination
pub async fn list_agents(
    pool: &PgPool,
    after_name: Option<&str>,
    limit: u32,
) -> Result<PaginatedResponse<AgentPublic>, AppError> {
    let limit = limit.min(500);
    let fetch_limit = (limit + 1) as i64;

    let agents = if let Some(after) = after_name {
        sqlx::query_as::<_, Agent>(
            r#"
            SELECT id, name, token_hash, enabled, created_at
            FROM agents
            WHERE enabled = true AND name > $1
            ORDER BY name
            LIMIT $2
            "#,
        )
        .bind(after)
        .bind(fetch_limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, Agent>(
            r#"
            SELECT id, name, token_hash, enabled, created_at
            FROM agents
            WHERE enabled = true
            ORDER BY name
            LIMIT $1
            "#,
        )
        .bind(fetch_limit)
        .fetch_all(pool)
        .await?
    };

    let has_more = agents.len() > limit as usize;
    let items: Vec<AgentPublic> = agents
        .into_iter()
        .take(limit as usize)
        .map(|a| a.into())
        .collect();

    let next_cursor = if has_more && !items.is_empty() {
        Some(items.last().unwrap().name.clone())
    } else {
        None
    };

    Ok(PaginatedResponse {
        items,
        next_cursor,
        has_more,
    })
}

/// Get agent by ID
pub async fn get_agent_by_id(pool: &PgPool, agent_id: Uuid) -> Result<Agent, AppError> {
    let agent = sqlx::query_as::<_, Agent>(
        "SELECT id, name, token_hash, enabled, created_at FROM agents WHERE id = $1",
    )
    .bind(agent_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Agent not found".to_string()))?;

    Ok(agent)
}

/// Validate agent name format
fn is_valid_agent_name(name: &str) -> bool {
    let len = name.len();
    if !(1..=64).contains(&len) {
        return false;
    }

    name.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_agent_names() {
        assert!(is_valid_agent_name("alice"));
        assert!(is_valid_agent_name("bob-123"));
        assert!(is_valid_agent_name("agent-1"));
        assert!(is_valid_agent_name("a"));
    }

    #[test]
    fn test_invalid_agent_names() {
        assert!(!is_valid_agent_name(""));
        assert!(!is_valid_agent_name("Alice")); // uppercase
        assert!(!is_valid_agent_name("alice_bob")); // underscore
        assert!(!is_valid_agent_name("alice bob")); // space
        assert!(!is_valid_agent_name(&"a".repeat(65))); // too long
    }
}
