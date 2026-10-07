use crate::auth::AuthenticatedAgent;
use crate::db;
use crate::error::AppError;
use crate::models::{AgentPublic, PaginatedResponse, PaginationQuery};
use axum::{extract::State, Extension, Json};
use sqlx::PgPool;

/// GET /v1/me - Get current agent identity
pub async fn get_me(
    Extension(auth): Extension<AuthenticatedAgent>,
) -> Result<Json<AgentPublic>, AppError> {
    Ok(Json(auth.agent.into()))
}

/// GET /v1/agents - List enabled agents
pub async fn list_agents(
    State(pool): State<PgPool>,
    axum::extract::Query(query): axum::extract::Query<PaginationQuery>,
) -> Result<Json<PaginatedResponse<AgentPublic>>, AppError> {
    let after = query.after.as_deref();
    let limit = query.limit.unwrap_or(100);

    let result = db::agents::list_agents(&pool, after, limit).await?;

    Ok(Json(result))
}
