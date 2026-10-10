//! Explicit read-only DTOs: database credential columns never enter these results.
use crate::{auth::ReadAuth, error::AppError, models::PaginatedResponse};
use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadQuery {
    pub before: Option<String>,
    pub limit: Option<u32>,
    pub search: Option<String>,
}

impl ReadQuery {
    fn limit(&self) -> Result<usize, AppError> {
        match self.limit.unwrap_or(50) {
            n @ 1..=100 => Ok(n as usize),
            _ => Err(AppError::BadRequest(
                "limit must be between 1 and 100".into(),
            )),
        }
    }
    fn cursor(&self, kind: &str) -> Result<Option<&str>, AppError> {
        self.before
            .as_deref()
            .map(|s| {
                if s.len() > 64 {
                    return Err(AppError::BadRequest("Invalid cursor".into()));
                }
                s.strip_prefix(kind)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| AppError::BadRequest("Invalid cursor".into()))
            })
            .transpose()
    }
    fn uuid_cursor(&self, kind: &str) -> Result<Option<Uuid>, AppError> {
        self.cursor(kind)?
            .map(|s| Uuid::parse_str(s).map_err(|_| AppError::BadRequest("Invalid cursor".into())))
            .transpose()
    }
    fn message_cursor(&self) -> Result<Option<i64>, AppError> {
        self.cursor("m1:")?
            .map(|s| {
                if !s.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(AppError::BadRequest("Invalid cursor".into()));
                }
                s.parse::<i64>()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| AppError::BadRequest("Invalid cursor".into()))
            })
            .transpose()
    }
    fn search_term(&self) -> Result<Option<String>, AppError> {
        self.search
            .as_deref()
            .map(|s| {
                let s = s.trim();
                if s.is_empty() || s.len() > 256 || s.contains('\0') {
                    return Err(AppError::BadRequest(
                        "search must contain 1 to 256 bytes".into(),
                    ));
                }
                // Literal substring search; SQL wildcard characters have no special meaning.
                Ok(format!(
                    "%{}%",
                    s.replace('\\', "\\\\")
                        .replace('%', "\\%")
                        .replace('_', "\\_")
                ))
            })
            .transpose()
    }
}

#[derive(Serialize, sqlx::FromRow)]
pub struct PublicAgent {
    pub id: Uuid,
    pub name: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
pub struct Member {
    pub id: Uuid,
    pub name: String,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct ReadConversation {
    pub id: Uuid,
    #[serde(rename = "type")]
    pub conversation_type: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub members: sqlx::types::Json<Vec<Member>>,
    pub last_activity_at: Option<DateTime<Utc>>,
    pub message_count: i64,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct ReadMessage {
    pub id: String,
    pub conversation_id: Uuid,
    pub sender_id: Uuid,
    pub sender_name: String,
    pub body: String,
    pub reply_to_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

fn page<T>(
    mut items: Vec<T>,
    limit: usize,
    cursor: impl FnOnce(&T) -> String,
) -> PaginatedResponse<T> {
    let has_more = items.len() > limit;
    items.truncate(limit);
    let next_cursor = if has_more {
        items.last().map(cursor)
    } else {
        None
    };
    PaginatedResponse {
        items,
        next_cursor,
        has_more,
    }
}

pub async fn agents(
    State(pool): State<PgPool>,
    Extension(_auth): Extension<ReadAuth>,
    Query(query): Query<ReadQuery>,
) -> Result<Json<PaginatedResponse<PublicAgent>>, AppError> {
    let limit = query.limit()?;
    let before = query.uuid_cursor("a1:")?;
    let items = sqlx::query_as::<_, PublicAgent>("SELECT id, name, enabled, created_at FROM agents WHERE ($1::uuid IS NULL OR id < $1) ORDER BY id DESC LIMIT $2")
        .bind(before).bind((limit + 1) as i64).fetch_all(&pool).await?;
    Ok(Json(page(items, limit, |a| format!("a1:{}", a.id))))
}

pub async fn conversations(
    State(pool): State<PgPool>,
    Extension(_auth): Extension<ReadAuth>,
    Query(query): Query<ReadQuery>,
) -> Result<Json<PaginatedResponse<ReadConversation>>, AppError> {
    let limit = query.limit()?;
    let before = query.uuid_cursor("c1:")?;
    // Immutable UUID ordering avoids missing/duplicating conversations when new messages arrive.
    let items = sqlx::query_as::<_, ReadConversation>(r#"
        SELECT c.id, c.conversation_type, c.name, c.description, c.created_at,
          COALESCE((SELECT jsonb_agg(jsonb_build_object('id', a.id, 'name', a.name) ORDER BY a.name, a.id)
             FROM dm_members dm JOIN agents a ON a.id = dm.agent_id WHERE dm.conversation_id = c.id), '[]'::jsonb) AS members,
          (SELECT max(m.created_at) FROM messages m WHERE m.conversation_id = c.id) AS last_activity_at,
          (SELECT count(*) FROM messages m WHERE m.conversation_id = c.id) AS message_count
        FROM conversations c WHERE ($1::uuid IS NULL OR c.id < $1) ORDER BY c.id DESC LIMIT $2
    "#).bind(before).bind((limit + 1) as i64).fetch_all(&pool).await?;
    Ok(Json(page(items, limit, |c| format!("c1:{}", c.id))))
}

async fn read_messages(
    pool: &PgPool,
    conversation: Option<Uuid>,
    query: ReadQuery,
) -> Result<Json<PaginatedResponse<ReadMessage>>, AppError> {
    let limit = query.limit()?;
    let before = query.message_cursor()?;
    let search = query.search_term()?;
    let items = sqlx::query_as::<_, ReadMessage>(
        r#"
        SELECT m.id::text AS id, m.conversation_id, m.sender_id, a.name AS sender_name,
            m.body, m.reply_to_id::text AS reply_to_id, m.created_at
        FROM messages m JOIN agents a ON a.id = m.sender_id
        WHERE ($1::uuid IS NULL OR m.conversation_id = $1)
          AND ($2::bigint IS NULL OR m.id < $2)
          AND ($3::text IS NULL OR m.body ILIKE $3)
        ORDER BY m.id DESC LIMIT $4
    "#,
    )
    .bind(conversation)
    .bind(before)
    .bind(search)
    .bind((limit + 1) as i64)
    .fetch_all(pool)
    .await?;
    Ok(Json(page(items, limit, |m| format!("m1:{}", m.id))))
}

pub async fn messages(
    State(pool): State<PgPool>,
    Extension(_auth): Extension<ReadAuth>,
    Path(id): Path<Uuid>,
    Query(query): Query<ReadQuery>,
) -> Result<Json<PaginatedResponse<ReadMessage>>, AppError> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversations WHERE id = $1)")
            .bind(id)
            .fetch_one(&pool)
            .await?;
    if !exists {
        return Err(AppError::NotFound("Conversation not found".into()));
    }
    read_messages(&pool, Some(id), query).await
}

pub async fn search(
    State(pool): State<PgPool>,
    Extension(_auth): Extension<ReadAuth>,
    Query(query): Query<ReadQuery>,
) -> Result<Json<PaginatedResponse<ReadMessage>>, AppError> {
    if query.search.is_none() {
        return Err(AppError::BadRequest("search is required".into()));
    }
    read_messages(&pool, None, query).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_validation_is_bounded() {
        let mut q = ReadQuery {
            before: None,
            limit: None,
            search: None,
        };
        assert_eq!(q.limit().unwrap(), 50);
        for n in [0, 101, u32::MAX] {
            q.limit = Some(n);
            assert!(q.limit().is_err());
        }
        for cursor in [
            "",
            "1",
            "m1:-1",
            "m1:0",
            "m1:+1",
            "m1:9223372036854775808",
            "c1:123",
        ] {
            q.before = Some(cursor.into());
            assert!(q.message_cursor().is_err());
        }
        q.before = Some("m1:123".into());
        assert_eq!(q.message_cursor().unwrap(), Some(123));
        for search in ["".into(), " ".into(), "a".repeat(257), "\0".into()] {
            q.search = Some(search);
            assert!(q.search_term().is_err());
        }
        q.search = Some("100%_\\".into());
        assert_eq!(q.search_term().unwrap().unwrap(), "%100\\%\\_\\\\%");
    }
}
