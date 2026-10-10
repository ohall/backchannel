use crate::db::acquire_message_lock;
use crate::error::AppError;
use crate::models::{Message, MessageResponse, PaginatedResponse};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// Create a new message with idempotency and advisory lock for ordering
pub async fn create_message(
    pool: &PgPool,
    conversation_id: Uuid,
    sender_id: Uuid,
    body: &str,
    client_message_id: &str,
    reply_to_id: Option<i64>,
) -> Result<Message, AppError> {
    // Validate body
    if body.trim().is_empty() {
        return Err(AppError::BadRequest(
            "Message body cannot be blank".to_string(),
        ));
    }

    if body.len() > 32 * 1024 {
        return Err(AppError::PayloadTooLarge(
            "Message body exceeds 32 KiB".to_string(),
        ));
    }

    // Validate client_message_id
    if client_message_id.is_empty() || client_message_id.len() > 128 {
        return Err(AppError::BadRequest(
            "client_message_id must be 1-128 characters".to_string(),
        ));
    }

    if !client_message_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(AppError::BadRequest(
            "client_message_id must be URL-safe (alphanumeric, hyphens, underscores)".to_string(),
        ));
    }

    // Check for existing message with same client_message_id from sender
    if let Some(existing) = sqlx::query_as::<_, Message>(
        r#"
        SELECT id, conversation_id, sender_id, body, reply_to_id, client_message_id, created_at
        FROM messages
        WHERE sender_id = $1 AND client_message_id = $2
        "#,
    )
    .bind(sender_id)
    .bind(client_message_id)
    .fetch_optional(pool)
    .await?
    {
        // Check if the existing message matches semantically
        if existing.conversation_id == conversation_id
            && existing.body == body
            && existing.reply_to_id == reply_to_id
        {
            // Idempotent: return existing message
            return Ok(existing);
        } else {
            // Conflict: same client_message_id but different content
            return Err(AppError::Conflict(
                "client_message_id already used with different content".to_string(),
            ));
        }
    }

    // Validate reply_to_id if present
    if let Some(reply_id) = reply_to_id {
        let reply_exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE id = $1 AND conversation_id = $2)",
        )
        .bind(reply_id)
        .bind(conversation_id)
        .fetch_one(pool)
        .await?;

        if !reply_exists {
            return Err(AppError::BadRequest(
                "Reply target does not exist in this conversation".to_string(),
            ));
        }
    }

    // Begin transaction and acquire advisory lock for ordering
    let mut tx = pool.begin().await?;
    acquire_message_lock(&mut tx).await?;

    let message = sqlx::query_as::<_, Message>(
        r#"
        INSERT INTO messages (conversation_id, sender_id, body, reply_to_id, client_message_id, created_at)
        VALUES ($1, $2, $3, $4, $5, NOW())
        RETURNING id, conversation_id, sender_id, body, reply_to_id, client_message_id, created_at
        "#
    )
    .bind(conversation_id)
    .bind(sender_id)
    .bind(body)
    .bind(reply_to_id)
    .bind(client_message_id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(message)
}

/// List messages in a conversation with cursor pagination
pub async fn list_messages(
    pool: &PgPool,
    conversation_id: Uuid,
    after_id: i64,
    limit: u32,
) -> Result<PaginatedResponse<MessageResponse>, AppError> {
    let limit = limit.clamp(1, 500);
    let fetch_limit = (limit + 1) as i64;

    let messages = sqlx::query_as::<_, Message>(
        r#"
        WITH candidates AS (
            SELECT id, conversation_id, sender_id, body, reply_to_id, client_message_id, created_at
            FROM messages WHERE conversation_id = $1 AND id > $2 ORDER BY id LIMIT $3
        ), budgeted AS (
            SELECT candidates.*, SUM(6::bigint * octet_length(body) + 2048) OVER (ORDER BY id)
                - (6::bigint * octet_length(body) + 2048) AS prior_bytes FROM candidates
        ) SELECT * FROM budgeted WHERE prior_bytes < 262144 ORDER BY id
        "#,
    )
    .bind(conversation_id)
    .bind(after_id)
    .bind(fetch_limit)
    .fetch_all(pool)
    .await?;

    Ok(bounded_page(messages, limit))
}

/// List feed messages (public + agent's DMs) with cursor pagination
pub async fn list_feed(
    pool: &PgPool,
    agent_id: Uuid,
    after_id: i64,
    limit: u32,
) -> Result<PaginatedResponse<MessageResponse>, AppError> {
    let limit = limit.clamp(1, 500);
    let fetch_limit = (limit + 1) as i64;

    let messages = sqlx::query_as::<_, Message>(
        r#"
        WITH candidates AS (
        SELECT m.id, m.conversation_id, m.sender_id, m.body, m.reply_to_id, m.client_message_id, m.created_at
        FROM messages m
        INNER JOIN conversations c ON m.conversation_id = c.id
        WHERE m.id > $1 AND (
            c.conversation_type = 'public'
            OR EXISTS (
                SELECT 1 FROM dm_members dm
                WHERE dm.conversation_id = c.id AND dm.agent_id = $2
            )
        )
        ORDER BY m.id
        LIMIT $3
        ), budgeted AS (
            SELECT candidates.*, SUM(6::bigint * octet_length(body) + 2048) OVER (ORDER BY id)
                - (6::bigint * octet_length(body) + 2048) AS prior_bytes FROM candidates
        ) SELECT * FROM budgeted WHERE prior_bytes < 262144 ORDER BY id
        "#
    )
    .bind(after_id)
    .bind(agent_id)
    .bind(fetch_limit)
    .fetch_all(pool)
    .await?;

    Ok(bounded_page(messages, limit))
}

/// Admin message filter parameters
pub struct AdminMessageFilters {
    pub after_id: i64,
    pub limit: u32,
    pub conversation_id: Option<Uuid>,
    pub sender_id: Option<Uuid>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub search: Option<String>,
}

/// Admin: list all messages with filters
#[allow(clippy::too_many_arguments)]
pub async fn admin_list_messages(
    pool: &PgPool,
    after_id: i64,
    limit: u32,
    conversation_id: Option<Uuid>,
    sender_id: Option<Uuid>,
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
    search: Option<&str>,
) -> Result<PaginatedResponse<MessageResponse>, AppError> {
    let limit = limit.clamp(1, 500);
    let fetch_limit = (limit + 1) as i64;

    // Build dynamic query
    let mut query = String::from(
        "SELECT id, conversation_id, sender_id, body, reply_to_id, client_message_id, created_at \
         FROM messages WHERE id > $1",
    );

    let mut param_count = 1;

    if conversation_id.is_some() {
        param_count += 1;
        query.push_str(&format!(" AND conversation_id = ${}", param_count));
    }

    if sender_id.is_some() {
        param_count += 1;
        query.push_str(&format!(" AND sender_id = ${}", param_count));
    }

    if since.is_some() {
        param_count += 1;
        query.push_str(&format!(" AND created_at >= ${}", param_count));
    }

    if until.is_some() {
        param_count += 1;
        query.push_str(&format!(" AND created_at <= ${}", param_count));
    }

    if search.is_some() {
        param_count += 1;
        query.push_str(&format!(" AND body ILIKE ${}", param_count));
    }

    param_count += 1;
    query.push_str(&format!(" ORDER BY id LIMIT ${}", param_count));

    let query = format!(
        "WITH candidates AS ({query}), budgeted AS (SELECT candidates.*, \
        SUM(6::bigint * octet_length(body) + 2048) OVER (ORDER BY id) - \
        (6::bigint * octet_length(body) + 2048) AS prior_bytes FROM candidates) \
        SELECT * FROM budgeted WHERE prior_bytes < 262144 ORDER BY id"
    );
    let mut query_builder = sqlx::query_as::<_, Message>(&query).bind(after_id);

    if let Some(cid) = conversation_id {
        query_builder = query_builder.bind(cid);
    }
    if let Some(sid) = sender_id {
        query_builder = query_builder.bind(sid);
    }
    if let Some(s) = since {
        query_builder = query_builder.bind(s);
    }
    if let Some(u) = until {
        query_builder = query_builder.bind(u);
    }
    if let Some(search_term) = search {
        let pattern = format!("%{}%", search_term);
        query_builder = query_builder.bind(pattern);
    }

    query_builder = query_builder.bind(fetch_limit);

    let messages = query_builder.fetch_all(pool).await?;

    Ok(bounded_page(messages, limit))
}

/// Conservative serialized-byte accounting covers JSON escaping and metadata.
/// Never advance the cursor past the last returned message, even on byte truncation.
const PAGE_BYTES: usize = 256 * 1024;
fn bounded_page(messages: Vec<Message>, limit: u32) -> PaginatedResponse<MessageResponse> {
    let total = messages.len();
    let mut used = 1024; // pagination/MCP envelope
    let mut items: Vec<MessageResponse> = Vec::new();
    for message in messages {
        let cost = message.body.len().saturating_mul(6).saturating_add(2048);
        if items.len() >= limit as usize || used + cost > PAGE_BYTES {
            break;
        }
        used += cost;
        items.push(message.into());
    }
    let has_more = items.len() < total;
    let next_cursor = if has_more {
        items.last().map(|m| m.id.clone())
    } else {
        None
    };
    PaginatedResponse {
        items,
        next_cursor,
        has_more,
    }
}
#[cfg(test)]
mod budget_tests {
    use super::*;
    fn message(id: i64, body: &str) -> Message {
        Message {
            id,
            conversation_id: Uuid::nil(),
            sender_id: Uuid::nil(),
            body: body.into(),
            reply_to_id: None,
            client_message_id: "test".into(),
            created_at: Utc::now(),
        }
    }
    #[test]
    fn escaped_large_feed_is_bounded_and_resumable() {
        let body = "\u{0001}".repeat(32768);
        let rows = (1..=100).map(|n| message(n, &body)).collect();
        let page = bounded_page(rows, 100);
        assert!(serde_json::to_vec(&page).unwrap().len() < PAGE_BYTES);
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.next_cursor.as_deref(), Some("1"));
        assert!(page.has_more);
        let next = bounded_page(vec![message(2, "legitimate next message")], 100);
        assert_eq!(next.items[0].id, "2");
        assert!(!next.has_more);
    }
    #[test]
    fn exact_row_boundary_preserves_cursor() {
        let page = bounded_page(vec![message(1, "a"), message(2, "b"), message(3, "c")], 2);
        assert_eq!(page.next_cursor.as_deref(), Some("2"));
        assert!(page.has_more);
        assert!(bounded_page(Vec::new(), 100).next_cursor.is_none());
    }
}
