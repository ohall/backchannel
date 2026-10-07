use crate::auth::AdminAuth;
use crate::db;
use crate::error::AppError;
use crate::models::{
    AdminMessagesQuery, AgentPublic, CreateAgentRequest, CreateAgentResponse, MessageResponse,
    PaginatedResponse, RotateTokenResponse, UpdateAgentRequest,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use sqlx::PgPool;
use uuid::Uuid;

/// POST /v1/admin/agents - Create agent
pub async fn create_agent(
    State(pool): State<PgPool>,
    Extension(_admin): Extension<AdminAuth>,
    Json(req): Json<CreateAgentRequest>,
) -> Result<(StatusCode, Json<CreateAgentResponse>), AppError> {
    let (agent, token) = db::agents::create_agent(&pool, &req.name).await?;

    Ok((
        StatusCode::CREATED,
        Json(CreateAgentResponse {
            agent: agent.into(),
            token,
        }),
    ))
}

/// POST /v1/admin/agents/{id}/rotate-token - Rotate agent token
pub async fn rotate_token(
    State(pool): State<PgPool>,
    Extension(_admin): Extension<AdminAuth>,
    Path(agent_id): Path<String>,
) -> Result<Json<RotateTokenResponse>, AppError> {
    let agent_id = Uuid::parse_str(&agent_id)
        .map_err(|_| AppError::BadRequest("Invalid agent ID".to_string()))?;

    let token = db::agents::rotate_agent_token(&pool, agent_id).await?;

    Ok(Json(RotateTokenResponse {
        agent_id: agent_id.to_string(),
        token,
    }))
}

/// PATCH /v1/admin/agents/{id} - Update agent
pub async fn update_agent(
    State(pool): State<PgPool>,
    Extension(_admin): Extension<AdminAuth>,
    Path(agent_id): Path<String>,
    Json(req): Json<UpdateAgentRequest>,
) -> Result<Json<AgentPublic>, AppError> {
    let agent_id = Uuid::parse_str(&agent_id)
        .map_err(|_| AppError::BadRequest("Invalid agent ID".to_string()))?;

    let agent = db::agents::update_agent_enabled(&pool, agent_id, req.enabled).await?;

    Ok(Json(agent.into()))
}

/// GET /v1/admin/messages - Review all messages
pub async fn list_all_messages(
    State(pool): State<PgPool>,
    Extension(_admin): Extension<AdminAuth>,
    axum::extract::Query(query): axum::extract::Query<AdminMessagesQuery>,
) -> Result<Json<PaginatedResponse<MessageResponse>>, AppError> {
    let after_id = query
        .after
        .as_ref()
        .map(|s| s.parse::<i64>())
        .transpose()
        .map_err(|_| AppError::BadRequest("Invalid cursor".to_string()))?
        .unwrap_or(0);

    let limit = query.limit.unwrap_or(100);

    let conversation_id = query
        .conversation_id
        .as_ref()
        .map(|s| Uuid::parse_str(s))
        .transpose()
        .map_err(|_| AppError::BadRequest("Invalid conversation_id".to_string()))?;

    let sender_id = query
        .sender_id
        .as_ref()
        .map(|s| Uuid::parse_str(s))
        .transpose()
        .map_err(|_| AppError::BadRequest("Invalid sender_id".to_string()))?;

    let result = db::messages::admin_list_messages(
        &pool,
        after_id,
        limit,
        conversation_id,
        sender_id,
        query.since,
        query.until,
        query.search.as_deref(),
    )
    .await?;

    Ok(Json(result))
}

/// GET /v1/admin/export - Export messages as JSONL
pub async fn export_messages(
    State(pool): State<PgPool>,
    Extension(_admin): Extension<AdminAuth>,
    axum::extract::Query(query): axum::extract::Query<AdminMessagesQuery>,
) -> Result<(StatusCode, axum::response::Response), AppError> {
    let after_id = query
        .after
        .as_ref()
        .map(|s| s.parse::<i64>())
        .transpose()
        .map_err(|_| AppError::BadRequest("Invalid cursor".to_string()))?
        .unwrap_or(0);

    let limit = query.limit.unwrap_or(100);

    let conversation_id = query
        .conversation_id
        .as_ref()
        .map(|s| Uuid::parse_str(s))
        .transpose()
        .map_err(|_| AppError::BadRequest("Invalid conversation_id".to_string()))?;

    let sender_id = query
        .sender_id
        .as_ref()
        .map(|s| Uuid::parse_str(s))
        .transpose()
        .map_err(|_| AppError::BadRequest("Invalid sender_id".to_string()))?;

    let result = db::messages::admin_list_messages(
        &pool,
        after_id,
        limit,
        conversation_id,
        sender_id,
        query.since,
        query.until,
        query.search.as_deref(),
    )
    .await?;

    // Convert to JSONL
    let mut jsonl = String::new();
    for item in &result.items {
        jsonl.push_str(
            &serde_json::to_string(item)
                .map_err(|e| AppError::Internal(format!("Serialization error: {}", e)))?,
        );
        jsonl.push('\n');
    }

    let response = axum::response::Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/x-ndjson")
        .header("X-Next-Cursor", result.next_cursor.as_deref().unwrap_or(""))
        .header("X-Has-More", result.has_more.to_string())
        .body(axum::body::Body::from(jsonl))
        .map_err(|e| AppError::Internal(format!("Response build error: {}", e)))?;

    Ok((StatusCode::OK, response))
}
