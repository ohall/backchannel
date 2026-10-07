use crate::auth::AuthenticatedAgent;
use crate::db;
use crate::error::AppError;
use crate::models::{MessageResponse, PaginatedResponse, PaginationQuery};
use axum::{extract::State, Extension, Json};
use sqlx::PgPool;

/// GET /v1/feed - Get public messages and agent's DMs
pub async fn get_feed(
    State(pool): State<PgPool>,
    Extension(auth): Extension<AuthenticatedAgent>,
    axum::extract::Query(query): axum::extract::Query<PaginationQuery>,
) -> Result<Json<PaginatedResponse<MessageResponse>>, AppError> {
    let after_id = query
        .after
        .as_ref()
        .map(|s| s.parse::<i64>())
        .transpose()
        .map_err(|_| AppError::BadRequest("Invalid cursor".to_string()))?
        .unwrap_or(0);

    let limit = query.limit.unwrap_or(100);

    let result = db::messages::list_feed(&pool, auth.agent.id, after_id, limit).await?;

    Ok(Json(result))
}
