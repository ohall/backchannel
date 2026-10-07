use crate::auth::AuthenticatedAgent;
use crate::db;
use crate::error::AppError;
use crate::models::{
    ConversationResponse, CreateChannelRequest, PaginatedResponse, PaginationQuery,
};
use axum::{extract::State, http::StatusCode, Extension, Json};
use sqlx::PgPool;

/// POST /v1/channels - Create a public channel
pub async fn create_channel(
    State(pool): State<PgPool>,
    Extension(auth): Extension<AuthenticatedAgent>,
    Json(req): Json<CreateChannelRequest>,
) -> Result<(StatusCode, Json<ConversationResponse>), AppError> {
    let conversation = db::conversations::create_channel(
        &pool,
        &req.name,
        req.description.as_deref(),
        auth.agent.id,
    )
    .await?;

    Ok((StatusCode::CREATED, Json(conversation.into())))
}

/// GET /v1/channels - List public channels
pub async fn list_channels(
    State(pool): State<PgPool>,
    axum::extract::Query(query): axum::extract::Query<PaginationQuery>,
) -> Result<Json<PaginatedResponse<ConversationResponse>>, AppError> {
    let after = query.after.as_deref();
    let limit = query.limit.unwrap_or(100);

    let result = db::conversations::list_channels(&pool, after, limit).await?;

    Ok(Json(result))
}
