use crate::error::AppError;
use crate::models::{Conversation, ConversationResponse, PaginatedResponse};
use sqlx::PgPool;
use uuid::Uuid;

/// Create a public channel
pub async fn create_channel(
    pool: &PgPool,
    name: &str,
    description: Option<&str>,
    creator_id: Uuid,
) -> Result<Conversation, AppError> {
    // Validate name
    if !is_valid_channel_name(name) {
        return Err(AppError::BadRequest(
            "Channel name must be lowercase, 2-64 characters, letters/digits/hyphens only"
                .to_string(),
        ));
    }

    // Validate description
    if let Some(desc) = description {
        if desc.len() > 512 {
            return Err(AppError::BadRequest(
                "Description must be at most 512 characters".to_string(),
            ));
        }
    }

    let id = Uuid::new_v4();

    let conversation = sqlx::query_as::<_, Conversation>(
        r#"
        INSERT INTO conversations (id, conversation_type, name, description, creator_id, created_at)
        VALUES ($1, 'public', $2, $3, $4, NOW())
        RETURNING id, conversation_type, name, description, creator_id, dm_canonical_key, created_at
        "#,
    )
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(creator_id)
    .fetch_one(pool)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref db_err) = e {
            if db_err.constraint() == Some("conversations_name_key") {
                return AppError::Conflict("Channel name already exists".to_string());
            }
        }
        e.into()
    })?;

    Ok(conversation)
}

/// Create or get DM conversation
pub async fn create_or_get_dm(
    pool: &PgPool,
    agent1_id: Uuid,
    agent2_id: Uuid,
) -> Result<Conversation, AppError> {
    if agent1_id == agent2_id {
        return Err(AppError::BadRequest(
            "Cannot create DM with self".to_string(),
        ));
    }

    // Create canonical key (sorted IDs)
    let canonical_key = if agent1_id < agent2_id {
        format!("{}:{}", agent1_id, agent2_id)
    } else {
        format!("{}:{}", agent2_id, agent1_id)
    };

    // Check if both agents are enabled
    let (agent1_enabled, agent2_enabled): (bool, bool) = sqlx::query_as(
        "SELECT 
            (SELECT enabled FROM agents WHERE id = $1) as agent1_enabled,
            (SELECT enabled FROM agents WHERE id = $2) as agent2_enabled",
    )
    .bind(agent1_id)
    .bind(agent2_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("One or both agents not found".to_string()))?;

    if !agent1_enabled || !agent2_enabled {
        return Err(AppError::BadRequest(
            "Cannot create DM with disabled agent".to_string(),
        ));
    }

    // Try to find existing DM
    if let Some(existing) = sqlx::query_as::<_, Conversation>(
        r#"
        SELECT id, conversation_type, name, description, creator_id, dm_canonical_key, created_at
        FROM conversations
        WHERE dm_canonical_key = $1
        "#,
    )
    .bind(&canonical_key)
    .fetch_optional(pool)
    .await?
    {
        return Ok(existing);
    }

    // Create new DM with transaction to ensure atomic insert of conversation and members
    let mut tx = pool.begin().await?;

    let conv_id = Uuid::new_v4();
    let conversation = sqlx::query_as::<_, Conversation>(
        r#"
        INSERT INTO conversations (id, conversation_type, creator_id, dm_canonical_key, created_at)
        VALUES ($1, 'dm', $2, $3, NOW())
        RETURNING id, conversation_type, name, description, creator_id, dm_canonical_key, created_at
        "#,
    )
    .bind(conv_id)
    .bind(agent1_id)
    .bind(&canonical_key)
    .fetch_one(&mut *tx)
    .await;

    match conversation {
        Ok(conv) => {
            // Insert both members
            sqlx::query("INSERT INTO dm_members (conversation_id, agent_id) VALUES ($1, $2), ($1, $3)")
                .bind(conv_id)
                .bind(agent1_id)
                .bind(agent2_id)
                .execute(&mut *tx)
                .await?;

            tx.commit().await?;
            Ok(conv)
        }
        Err(sqlx::Error::Database(ref db_err))
            if db_err.constraint() == Some("conversations_dm_canonical_key_key") =>
        {
            // Concurrent request created it - fetch and return
            drop(tx);
            sqlx::query_as::<_, Conversation>(
                r#"
                SELECT id, conversation_type, name, description, creator_id, dm_canonical_key, created_at
                FROM conversations
                WHERE dm_canonical_key = $1
                "#,
            )
            .bind(&canonical_key)
            .fetch_one(pool)
            .await
            .map_err(Into::into)
        }
        Err(e) => Err(e.into()),
    }
}

/// List public channels with pagination
pub async fn list_channels(
    pool: &PgPool,
    after_name: Option<&str>,
    limit: u32,
) -> Result<PaginatedResponse<ConversationResponse>, AppError> {
    let limit = limit.min(500);
    let fetch_limit = (limit + 1) as i64;

    let conversations = if let Some(after) = after_name {
        sqlx::query_as::<_, Conversation>(
            r#"
            SELECT id, conversation_type, name, description, creator_id, dm_canonical_key, created_at
            FROM conversations
            WHERE conversation_type = 'public' AND name > $1
            ORDER BY name
            LIMIT $2
            "#
        )
        .bind(after)
        .bind(fetch_limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, Conversation>(
            r#"
            SELECT id, conversation_type, name, description, creator_id, dm_canonical_key, created_at
            FROM conversations
            WHERE conversation_type = 'public'
            ORDER BY name
            LIMIT $1
            "#
        )
        .bind(fetch_limit)
        .fetch_all(pool)
        .await?
    };

    let has_more = conversations.len() > limit as usize;
    let items: Vec<ConversationResponse> = conversations
        .into_iter()
        .take(limit as usize)
        .map(|c| c.into())
        .collect();

    let next_cursor = if has_more && !items.is_empty() {
        items.last().unwrap().name.clone()
    } else {
        None
    };

    Ok(PaginatedResponse {
        items,
        next_cursor,
        has_more,
    })
}

/// List DMs for an agent with pagination
pub async fn list_dms(
    pool: &PgPool,
    agent_id: Uuid,
    after_id: Option<Uuid>,
    limit: u32,
) -> Result<PaginatedResponse<ConversationResponse>, AppError> {
    let limit = limit.min(500);
    let fetch_limit = (limit + 1) as i64;

    let conversations = if let Some(after) = after_id {
        sqlx::query_as::<_, Conversation>(
            r#"
            SELECT c.id, c.conversation_type, c.name, c.description, c.creator_id, c.dm_canonical_key, c.created_at
            FROM conversations c
            INNER JOIN dm_members m ON c.id = m.conversation_id
            WHERE m.agent_id = $1 AND c.id > $2
            ORDER BY c.id
            LIMIT $3
            "#
        )
        .bind(agent_id)
        .bind(after)
        .bind(fetch_limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, Conversation>(
            r#"
            SELECT c.id, c.conversation_type, c.name, c.description, c.creator_id, c.dm_canonical_key, c.created_at
            FROM conversations c
            INNER JOIN dm_members m ON c.id = m.conversation_id
            WHERE m.agent_id = $1
            ORDER BY c.id
            LIMIT $2
            "#
        )
        .bind(agent_id)
        .bind(fetch_limit)
        .fetch_all(pool)
        .await?
    };

    let has_more = conversations.len() > limit as usize;
    let items: Vec<ConversationResponse> = conversations
        .into_iter()
        .take(limit as usize)
        .map(|c| c.into())
        .collect();

    let next_cursor = if has_more && !items.is_empty() {
        Some(items.last().unwrap().id.clone())
    } else {
        None
    };

    Ok(PaginatedResponse {
        items,
        next_cursor,
        has_more,
    })
}

/// Check if agent has access to conversation
pub async fn check_conversation_access(
    pool: &PgPool,
    conversation_id: Uuid,
    agent_id: Uuid,
) -> Result<Conversation, AppError> {
    let conversation = sqlx::query_as::<_, Conversation>(
        "SELECT id, conversation_type, name, description, creator_id, dm_canonical_key, created_at 
         FROM conversations WHERE id = $1",
    )
    .bind(conversation_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Conversation not found".to_string()))?;

    // Public channels are accessible to all authenticated agents
    if conversation.conversation_type == "public" {
        return Ok(conversation);
    }

    // For DMs, check membership
    if conversation.conversation_type == "dm" {
        let is_member = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM dm_members WHERE conversation_id = $1 AND agent_id = $2)",
        )
        .bind(conversation_id)
        .bind(agent_id)
        .fetch_one(pool)
        .await?;

        if !is_member {
            return Err(AppError::NotFound("Conversation not found".to_string()));
        }

        return Ok(conversation);
    }

    Err(AppError::NotFound("Conversation not found".to_string()))
}

/// Validate channel name format
fn is_valid_channel_name(name: &str) -> bool {
    let len = name.len();
    if !(2..=64).contains(&len) {
        return false;
    }

    name.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_channel_names() {
        assert!(is_valid_channel_name("general"));
        assert!(is_valid_channel_name("dev-team"));
        assert!(is_valid_channel_name("channel-123"));
    }

    #[test]
    fn test_invalid_channel_names() {
        assert!(!is_valid_channel_name("a")); // too short
        assert!(!is_valid_channel_name("General")); // uppercase
        assert!(!is_valid_channel_name("dev_team")); // underscore
        assert!(!is_valid_channel_name(&"a".repeat(65))); // too long
    }
}
