use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Agent model
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Agent {
    pub id: Uuid,
    pub name: String,
    #[serde(skip_serializing)]
    pub token_hash: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct AgentPublic {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

impl From<Agent> for AgentPublic {
    fn from(agent: Agent) -> Self {
        AgentPublic {
            id: agent.id.to_string(),
            name: agent.name,
            enabled: agent.enabled,
            created_at: agent.created_at,
        }
    }
}

// Conversation model
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Conversation {
    pub id: Uuid,
    pub conversation_type: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub creator_id: Uuid,
    pub dm_canonical_key: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ConversationResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub conversation_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub creator_id: String,
    pub created_at: DateTime<Utc>,
}

impl From<Conversation> for ConversationResponse {
    fn from(conv: Conversation) -> Self {
        ConversationResponse {
            id: conv.id.to_string(),
            conversation_type: conv.conversation_type,
            name: conv.name,
            description: conv.description,
            creator_id: conv.creator_id.to_string(),
            created_at: conv.created_at,
        }
    }
}

// Message model
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Message {
    pub id: i64,
    pub conversation_id: Uuid,
    pub sender_id: Uuid,
    pub body: String,
    pub reply_to_id: Option<i64>,
    pub client_message_id: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub id: String,
    pub conversation_id: String,
    pub sender_id: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to_id: Option<String>,
    pub client_message_id: String,
    pub created_at: DateTime<Utc>,
}

impl From<Message> for MessageResponse {
    fn from(msg: Message) -> Self {
        MessageResponse {
            id: msg.id.to_string(),
            conversation_id: msg.conversation_id.to_string(),
            sender_id: msg.sender_id.to_string(),
            body: msg.body,
            reply_to_id: msg.reply_to_id.map(|id| id.to_string()),
            client_message_id: msg.client_message_id,
            created_at: msg.created_at,
        }
    }
}

// Request/Response types
#[derive(Debug, Deserialize)]
pub struct CreateChannelRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDmRequest {
    pub recipient_agent_id: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateMessageRequest {
    pub body: String,
    pub client_message_id: String,
    #[serde(default)]
    pub reply_to_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PaginationQuery {
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct PaginatedResponse<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

// Admin request types
#[derive(Debug, Deserialize)]
pub struct CreateAgentRequest {
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct CreateAgentResponse {
    pub agent: AgentPublic,
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct RotateTokenResponse {
    pub agent_id: String,
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAgentRequest {
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct AdminMessagesQuery {
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub sender_id: Option<String>,
    #[serde(default)]
    pub since: Option<DateTime<Utc>>,
    #[serde(default)]
    pub until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub search: Option<String>,
}
