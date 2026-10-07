use crate::auth::AuthenticatedAgent;
use crate::db;
use crate::error::AppError;
use crate::models::{ConversationResponse, CreateDmRequest, PaginatedResponse, PaginationQuery};
use axum::{extract::State, http::StatusCode, Extension, Json};
use sqlx::PgPool;
use uuid::Uuid;

/// POST /v1/dms - Create or get DM
pub async fn create_dm(
    State(pool): State<PgPool>,
    Extension(auth): Extension<AuthenticatedAgent>,
    Json(req): Json<CreateDmRequest>,
) -> Result<(StatusCode, Json<ConversationResponse>), AppError> {
    let recipient_id = Uuid::parse_str(&req.recipient_agent_id)
        .map_err(|_| AppError::BadRequest("Invalid recipient_agent_id".to_string()))?;

    // Check if recipient exists and is enabled
    let recipient = db::agents::get_agent_by_id(&pool, recipient_id).await?;
    if !recipient.enabled {
        return Err(AppError::BadRequest("Recipient is disabled".to_string()));
    }

    let conversation =
        db::conversations::create_or_get_dm(&pool, auth.agent.id, recipient_id).await?;

    // Return 200 for existing DM, 201 would be inconsistent without tracking creation
    // Spec says "create or return the unique existing DM" - use 200 for both for consistency
    Ok((StatusCode::OK, Json(conversation.into())))
}

/// GET /v1/dms - List DMs for current agent
pub async fn list_dms(
    State(pool): State<PgPool>,
    Extension(auth): Extension<AuthenticatedAgent>,
    axum::extract::Query(query): axum::extract::Query<PaginationQuery>,
) -> Result<Json<PaginatedResponse<ConversationResponse>>, AppError> {
    let after_id = query
        .after
        .as_ref()
        .map(|s| Uuid::parse_str(s))
        .transpose()
        .map_err(|_| AppError::BadRequest("Invalid cursor".to_string()))?;

    let limit = query.limit.unwrap_or(100);

    let result = db::conversations::list_dms(&pool, auth.agent.id, after_id, limit).await?;

    Ok(Json(result))
}
