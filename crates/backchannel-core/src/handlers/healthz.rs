use axum::Json;
use serde_json::{json, Value};

/// Health check endpoint (no authentication required)
pub async fn healthz() -> Json<Value> {
    Json(json!({
        "status": "ok"
    }))
}
