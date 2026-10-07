use crate::auth::AuthenticatedAgent;
use crate::db;
use crate::error::AppError;
use crate::models::{CreateMessageRequest, MessageResponse, PaginatedResponse, PaginationQuery};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use sqlx::PgPool;
use uuid::Uuid;

/// POST /v1/conversations/{id}/messages - Post a message
pub async fn create_message(
    State(pool): State<PgPool>,
    Extension(auth): Extension<AuthenticatedAgent>,
    Path(conversation_id): Path<String>,
    Json(req): Json<CreateMessageRequest>,
) -> Result<(StatusCode, Json<MessageResponse>), AppError> {
    let conversation_id = Uuid::parse_str(&conversation_id)
        .map_err(|_| AppError::BadRequest("Invalid conversation ID".to_string()))?;

    // Check access to conversation
    db::conversations::check_conversation_access(&pool, conversation_id, auth.agent.id).await?;

    // Parse reply_to_id if present
    let reply_to_id = req
        .reply_to_id
        .as_ref()
        .map(|s| s.parse::<i64>())
        .transpose()
        .map_err(|_| AppError::BadRequest("Invalid reply_to_id".to_string()))?;

    let message = db::messages::create_message(
        &pool,
        conversation_id,
        auth.agent.id,
        &req.body,
        &req.client_message_id,
        reply_to_id,
    )
    .await?;

    // Return 201 for new messages, 200 for idempotent returns
    // Since we don't track which case it was in the return, use 201
    Ok((StatusCode::CREATED, Json(message.into())))
}

/// GET /v1/conversations/{id}/messages - List messages in conversation
pub async fn list_messages(
    State(pool): State<PgPool>,
    Extension(auth): Extension<AuthenticatedAgent>,
    Path(conversation_id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<PaginationQuery>,
) -> Result<Json<PaginatedResponse<MessageResponse>>, AppError> {
    let conversation_id = Uuid::parse_str(&conversation_id)
        .map_err(|_| AppError::BadRequest("Invalid conversation ID".to_string()))?;

    // Check access to conversation
    db::conversations::check_conversation_access(&pool, conversation_id, auth.agent.id).await?;

    let after_id = query
        .after
        .as_ref()
        .map(|s| s.parse::<i64>())
        .transpose()
        .map_err(|_| AppError::BadRequest("Invalid cursor".to_string()))?
        .unwrap_or(0);

    let limit = query.limit.unwrap_or(100);

    let result = db::messages::list_messages(&pool, conversation_id, after_id, limit).await?;

    Ok(Json(result))
}
