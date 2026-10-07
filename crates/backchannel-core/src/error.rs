use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use std::fmt;

#[derive(Debug)]
pub enum AppError {
    Unauthorized(String),
    NotFound(String),
    Conflict(String),
    BadRequest(String),
    PayloadTooLarge(String),
    TooManyRequests { retry_after_seconds: u64 },
    ServiceUnavailable(String),
    Internal(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Unauthorized(msg) => write!(f, "Unauthorized: {}", msg),
            AppError::NotFound(msg) => write!(f, "Not found: {}", msg),
            AppError::Conflict(msg) => write!(f, "Conflict: {}", msg),
            AppError::BadRequest(msg) => write!(f, "Bad request: {}", msg),
            AppError::PayloadTooLarge(msg) => write!(f, "Payload too large: {}", msg),
            AppError::TooManyRequests {
                retry_after_seconds,
            } => {
                write!(
                    f,
                    "Too many requests, retry after {} seconds",
                    retry_after_seconds
                )
            }
            AppError::ServiceUnavailable(msg) => write!(f, "Service unavailable: {}", msg),
            AppError::Internal(msg) => write!(f, "Internal error: {}", msg),
        }
    }
}

impl std::error::Error for AppError {}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: ErrorDetail,
    pub request_id: String,
}

#[derive(Serialize)]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let request_id = uuid::Uuid::new_v4().to_string();

        let (status, code, message) = match &self {
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, "unauthorized", msg.as_str()),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, "not_found", msg.as_str()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, "conflict", msg.as_str()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "bad_request", msg.as_str()),
            AppError::PayloadTooLarge(msg) => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                msg.as_str(),
            ),
            AppError::TooManyRequests { .. } => (
                StatusCode::TOO_MANY_REQUESTS,
                "too_many_requests",
                "Rate limit exceeded",
            ),
            AppError::ServiceUnavailable(msg) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "service_unavailable",
                msg.as_str(),
            ),
            AppError::Internal(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                msg.as_str(),
            ),
        };

        tracing::error!(
            error = %self,
            request_id = %request_id,
            status_code = %status.as_u16(),
            "Request failed"
        );

        let error_response = ErrorResponse {
            error: ErrorDetail {
                code: code.to_string(),
                message: message.to_string(),
            },
            request_id,
        };

        let mut response = (status, Json(error_response)).into_response();

        // Add WWW-Authenticate header for 401 responses
        if matches!(self, AppError::Unauthorized(_)) {
            response.headers_mut().insert(
                axum::http::header::WWW_AUTHENTICATE,
                "Bearer".parse().unwrap(),
            );
        }

        if let AppError::TooManyRequests {
            retry_after_seconds,
        } = self
        {
            response.headers_mut().insert(
                "Retry-After",
                retry_after_seconds.to_string().parse().unwrap(),
            );
        }

        response
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        match err {
            sqlx::Error::RowNotFound => AppError::NotFound("Resource not found".to_string()),
            sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed => {
                AppError::ServiceUnavailable("Database unavailable".to_string())
            }
            _ => {
                tracing::error!(error = %err, "Database error");
                AppError::Internal("Database error".to_string())
            }
        }
    }
}
