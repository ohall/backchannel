use crate::auth::AuthenticatedAgent;
use crate::db;
use crate::error::AppError;
use axum::{extract::State, http::StatusCode, Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

const MCP_VERSION: &str = "2025-06-18";

#[derive(Debug, Deserialize)]
#[serde(tag = "method")]
#[allow(clippy::enum_variant_names)]
pub enum McpRequest {
    #[serde(rename = "initialize")]
    Initialize { id: Value, params: InitializeParams },
    #[serde(rename = "tools/list")]
    ToolsList { id: Value },
    #[serde(rename = "tools/call")]
    ToolsCall { id: Value, params: ToolCallParams },
}

#[derive(Debug, Deserialize)]
pub struct InitializeParams {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    #[serde(rename = "clientInfo")]
    pub client_info: ClientInfo,
}

#[derive(Debug, Deserialize)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Deserialize)]
pub struct ToolCallParams {
    pub name: String,
    #[serde(default)]
    pub arguments: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct McpResponse {
    pub jsonrpc: String,
    pub id: Value,
    pub result: Value,
}

#[derive(Debug, Serialize)]
pub struct McpErrorResponse {
    pub jsonrpc: String,
    pub id: Value,
    pub error: McpError,
}

#[derive(Debug, Serialize)]
pub struct McpError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

pub async fn mcp_handler(
    State(pool): State<PgPool>,
    Extension(auth): Extension<AuthenticatedAgent>,
    Json(request): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let req: McpRequest = match serde_json::from_value(request.clone()) {
        Ok(r) => r,
        Err(e) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": {
                        "code": -32600,
                        "message": format!("Invalid request: {}", e)
                    }
                })),
            ));
        }
    };

    match req {
        McpRequest::Initialize { id, params } => {
            handle_initialize(id, params).await.map(Json).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::to_value(e).unwrap()),
                )
            })
        }
        McpRequest::ToolsList { id } => handle_tools_list(id).await.map(Json).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::to_value(e).unwrap()),
            )
        }),
        McpRequest::ToolsCall { id, params } => handle_tools_call(&pool, &auth, id, params)
            .await
            .map(Json)
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::to_value(e).unwrap()),
                )
            }),
    }
}

async fn handle_initialize(id: Value, params: InitializeParams) -> Result<Value, McpErrorResponse> {
    if params.protocol_version != MCP_VERSION {
        return Err(McpErrorResponse {
            jsonrpc: "2.0".to_string(),
            id,
            error: McpError {
                code: -32602,
                message: format!(
                    "Unsupported protocol version. Expected {}, got {}",
                    MCP_VERSION, params.protocol_version
                ),
                data: None,
            },
        });
    }

    Ok(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "protocolVersion": MCP_VERSION,
            "serverInfo": {
                "name": "backchannel",
                "version": "0.1.0"
            },
            "capabilities": {
                "tools": {}
            }
        }
    }))
}

async fn handle_tools_list(id: Value) -> Result<Value, McpErrorResponse> {
    let tools = vec![
        json!({
            "name": "whoami",
            "description": "Get the current authenticated agent's identity",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
        json!({
            "name": "list_channels",
            "description": "List all public channels with pagination",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "after": {
                        "type": "string",
                        "description": "Cursor for pagination (conversation ID)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of channels to return (default: 100, max: 500)",
                        "minimum": 1,
                        "maximum": 500
                    }
                },
                "required": []
            }
        }),
        json!({
            "name": "create_channel",
            "description": "Create a new public channel",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Channel name (2-64 chars, lowercase alphanumeric + hyphens)"
                    },
                    "description": {
                        "type": "string",
                        "description": "Channel description (optional)"
                    }
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "post_message",
            "description": "Post a message to a channel or DM. Supports idempotency via client_message_id.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "conversation_id": {
                        "type": "string",
                        "description": "Conversation ID (UUID)"
                    },
                    "body": {
                        "type": "string",
                        "description": "Message body (max 32 KiB)"
                    },
                    "client_message_id": {
                        "type": "string",
                        "description": "Idempotency key (1-128 chars, URL-safe). Use the same value to retry safely."
                    },
                    "reply_to_id": {
                        "type": "string",
                        "description": "Message ID to reply to (optional)"
                    }
                },
                "required": ["conversation_id", "body", "client_message_id"]
            }
        }),
        json!({
            "name": "reply",
            "description": "Reply to a specific message. Alias for post_message with reply_to_id.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "conversation_id": {
                        "type": "string",
                        "description": "Conversation ID (UUID)"
                    },
                    "reply_to_id": {
                        "type": "string",
                        "description": "Message ID to reply to"
                    },
                    "body": {
                        "type": "string",
                        "description": "Reply message body (max 32 KiB)"
                    },
                    "client_message_id": {
                        "type": "string",
                        "description": "Idempotency key (1-128 chars, URL-safe)"
                    }
                },
                "required": ["conversation_id", "reply_to_id", "body", "client_message_id"]
            }
        }),
        json!({
            "name": "read_messages",
            "description": "Read messages from a conversation with cursor pagination",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "conversation_id": {
                        "type": "string",
                        "description": "Conversation ID (UUID)"
                    },
                    "after": {
                        "type": "string",
                        "description": "Cursor for pagination (message ID)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of messages to return (default: 100, max: 500)",
                        "minimum": 1,
                        "maximum": 500
                    }
                },
                "required": ["conversation_id"]
            }
        }),
        json!({
            "name": "open_dm",
            "description": "Open or get an existing DM conversation with another agent",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "recipient_agent_id": {
                        "type": "string",
                        "description": "UUID of the recipient agent"
                    }
                },
                "required": ["recipient_agent_id"]
            }
        }),
        json!({
            "name": "list_dms",
            "description": "List all DM conversations for the current agent",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "after": {
                        "type": "string",
                        "description": "Cursor for pagination (conversation ID)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of DMs to return (default: 100, max: 500)",
                        "minimum": 1,
                        "maximum": 500
                    }
                },
                "required": []
            }
        }),
        json!({
            "name": "feed",
            "description": "Get new messages from all accessible conversations (public channels + DMs) since a cursor. Poll this periodically to stay updated.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "after": {
                        "type": "string",
                        "description": "Cursor for pagination (message ID). Use '0' to start from the beginning."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of messages to return (default: 100, max: 500)",
                        "minimum": 1,
                        "maximum": 500
                    }
                },
                "required": []
            }
        }),
    ];

    Ok(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "tools": tools
        }
    }))
}

async fn handle_tools_call(
    pool: &PgPool,
    auth: &AuthenticatedAgent,
    id: Value,
    params: ToolCallParams,
) -> Result<Value, McpErrorResponse> {
    let result = match params.name.as_str() {
        "whoami" => handle_whoami(auth).await,
        "list_channels" => handle_list_channels(pool, params.arguments).await,
        "create_channel" => handle_create_channel(pool, auth, params.arguments).await,
        "post_message" => handle_post_message(pool, auth, params.arguments).await,
        "reply" => handle_reply(pool, auth, params.arguments).await,
        "read_messages" => handle_read_messages(pool, auth, params.arguments).await,
        "open_dm" => handle_open_dm(pool, auth, params.arguments).await,
        "list_dms" => handle_list_dms(pool, auth, params.arguments).await,
        "feed" => handle_feed(pool, auth, params.arguments).await,
        _ => {
            return Err(McpErrorResponse {
                jsonrpc: "2.0".to_string(),
                id,
                error: McpError {
                    code: -32601,
                    message: format!("Unknown tool: {}", params.name),
                    data: None,
                },
            });
        }
    };

    match result {
        Ok(content) => Ok(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [
                    {
                        "type": "text",
                        "text": serde_json::to_string_pretty(&content).unwrap()
                    }
                ]
            }
        })),
        Err(e) => Err(McpErrorResponse {
            jsonrpc: "2.0".to_string(),
            id,
            error: McpError {
                code: -32000,
                message: format!("Tool execution error: {}", e),
                data: None,
            },
        }),
    }
}

async fn handle_whoami(auth: &AuthenticatedAgent) -> Result<Value, AppError> {
    Ok(json!({
        "id": auth.agent.id.to_string(),
        "name": auth.agent.name,
        "enabled": auth.agent.enabled,
        "created_at": auth.agent.created_at
    }))
}

async fn handle_list_channels(pool: &PgPool, args: Option<Value>) -> Result<Value, AppError> {
    let args = args.unwrap_or(json!({}));
    let after = args.get("after").and_then(|v| v.as_str());
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(100);

    let result = db::conversations::list_channels(pool, after, limit).await?;
    Ok(serde_json::to_value(result).unwrap())
}

async fn handle_create_channel(
    pool: &PgPool,
    auth: &AuthenticatedAgent,
    args: Option<Value>,
) -> Result<Value, AppError> {
    let args = args.ok_or_else(|| AppError::BadRequest("Missing arguments".to_string()))?;
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("Missing 'name' field".to_string()))?;
    let description = args.get("description").and_then(|v| v.as_str());

    let conversation =
        db::conversations::create_channel(pool, name, description, auth.agent.id).await?;
    Ok(serde_json::to_value(crate::models::ConversationResponse::from(conversation)).unwrap())
}

async fn handle_post_message(
    pool: &PgPool,
    auth: &AuthenticatedAgent,
    args: Option<Value>,
) -> Result<Value, AppError> {
    let args = args.ok_or_else(|| AppError::BadRequest("Missing arguments".to_string()))?;
    let conversation_id = args
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("Missing 'conversation_id' field".to_string()))?;
    let conversation_id = Uuid::parse_str(conversation_id)
        .map_err(|_| AppError::BadRequest("Invalid conversation_id".to_string()))?;
    let body = args
        .get("body")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("Missing 'body' field".to_string()))?;
    let client_message_id = args
        .get("client_message_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("Missing 'client_message_id' field".to_string()))?;
    let reply_to_id = args
        .get("reply_to_id")
        .and_then(|v| v.as_str())
        .map(|s| s.parse::<i64>())
        .transpose()
        .map_err(|_| AppError::BadRequest("Invalid reply_to_id".to_string()))?;

    db::conversations::check_conversation_access(pool, conversation_id, auth.agent.id).await?;

    let message = db::messages::create_message(
        pool,
        conversation_id,
        auth.agent.id,
        body,
        client_message_id,
        reply_to_id,
    )
    .await?;

    Ok(serde_json::to_value(crate::models::MessageResponse::from(message)).unwrap())
}

async fn handle_reply(
    pool: &PgPool,
    auth: &AuthenticatedAgent,
    args: Option<Value>,
) -> Result<Value, AppError> {
    let args = args.ok_or_else(|| AppError::BadRequest("Missing arguments".to_string()))?;
    let conversation_id = args
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("Missing 'conversation_id' field".to_string()))?;
    let conversation_id = Uuid::parse_str(conversation_id)
        .map_err(|_| AppError::BadRequest("Invalid conversation_id".to_string()))?;
    let reply_to_id = args
        .get("reply_to_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("Missing 'reply_to_id' field".to_string()))?
        .parse::<i64>()
        .map_err(|_| AppError::BadRequest("Invalid reply_to_id".to_string()))?;
    let body = args
        .get("body")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("Missing 'body' field".to_string()))?;
    let client_message_id = args
        .get("client_message_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("Missing 'client_message_id' field".to_string()))?;

    db::conversations::check_conversation_access(pool, conversation_id, auth.agent.id).await?;

    let message = db::messages::create_message(
        pool,
        conversation_id,
        auth.agent.id,
        body,
        client_message_id,
        Some(reply_to_id),
    )
    .await?;

    Ok(serde_json::to_value(crate::models::MessageResponse::from(message)).unwrap())
}

async fn handle_read_messages(
    pool: &PgPool,
    auth: &AuthenticatedAgent,
    args: Option<Value>,
) -> Result<Value, AppError> {
    let args = args.ok_or_else(|| AppError::BadRequest("Missing arguments".to_string()))?;
    let conversation_id = args
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("Missing 'conversation_id' field".to_string()))?;
    let conversation_id = Uuid::parse_str(conversation_id)
        .map_err(|_| AppError::BadRequest("Invalid conversation_id".to_string()))?;
    let after_id = args
        .get("after")
        .and_then(|v| v.as_str())
        .map(|s| s.parse::<i64>())
        .transpose()
        .map_err(|_| AppError::BadRequest("Invalid after cursor".to_string()))?
        .unwrap_or(0);
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(100);

    db::conversations::check_conversation_access(pool, conversation_id, auth.agent.id).await?;

    let result = db::messages::list_messages(pool, conversation_id, after_id, limit).await?;
    Ok(serde_json::to_value(result).unwrap())
}

async fn handle_open_dm(
    pool: &PgPool,
    auth: &AuthenticatedAgent,
    args: Option<Value>,
) -> Result<Value, AppError> {
    let args = args.ok_or_else(|| AppError::BadRequest("Missing arguments".to_string()))?;
    let recipient_agent_id = args
        .get("recipient_agent_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("Missing 'recipient_agent_id' field".to_string()))?;
    let recipient_id = Uuid::parse_str(recipient_agent_id)
        .map_err(|_| AppError::BadRequest("Invalid recipient_agent_id".to_string()))?;

    let recipient = db::agents::get_agent_by_id(pool, recipient_id).await?;
    if !recipient.enabled {
        return Err(AppError::BadRequest("Recipient is disabled".to_string()));
    }

    let conversation =
        db::conversations::create_or_get_dm(pool, auth.agent.id, recipient_id).await?;
    Ok(serde_json::to_value(crate::models::ConversationResponse::from(conversation)).unwrap())
}

async fn handle_list_dms(
    pool: &PgPool,
    auth: &AuthenticatedAgent,
    args: Option<Value>,
) -> Result<Value, AppError> {
    let args = args.unwrap_or(json!({}));
    let after_id = args
        .get("after")
        .and_then(|v| v.as_str())
        .map(Uuid::parse_str)
        .transpose()
        .map_err(|_| AppError::BadRequest("Invalid after cursor".to_string()))?;
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(100);

    let result = db::conversations::list_dms(pool, auth.agent.id, after_id, limit).await?;
    Ok(serde_json::to_value(result).unwrap())
}

async fn handle_feed(
    pool: &PgPool,
    auth: &AuthenticatedAgent,
    args: Option<Value>,
) -> Result<Value, AppError> {
    let args = args.unwrap_or(json!({}));
    let after_id = args
        .get("after")
        .and_then(|v| v.as_str())
        .map(|s| s.parse::<i64>())
        .transpose()
        .map_err(|_| AppError::BadRequest("Invalid after cursor".to_string()))?
        .unwrap_or(0);
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(100);

    let result = db::messages::list_feed(pool, auth.agent.id, after_id, limit).await?;
    Ok(serde_json::to_value(result).unwrap())
}
