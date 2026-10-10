//! Process-local admission budgets run before bearer verification or database work.
//! Edge-wide limits are still required across replicas. Forwarding headers are not identities.
use crate::error::AppError;
use axum::{
    body::{to_bytes, Body},
    extract::{ConnectInfo, Request, State},
    http::{header, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct Admission {
    body_limit: usize,
    budget: Arc<Mutex<Budget>>,
    active: Arc<Semaphore>,
}
struct Budget {
    start: Instant,
    requests: u32,
    peers: HashMap<std::net::IpAddr, u32>,
}
impl Admission {
    pub fn new(body_limit: usize) -> Self {
        Self {
            body_limit,
            budget: Arc::new(Mutex::new(Budget {
                start: Instant::now(),
                requests: 0,
                peers: HashMap::new(),
            })),
            active: Arc::new(Semaphore::new(32)),
        }
    }
    fn admit(&self, peer: Option<std::net::IpAddr>, now: Instant) -> Result<(), AppError> {
        let mut budget = self
            .budget
            .lock()
            .map_err(|_| AppError::ServiceUnavailable("Admission unavailable".into()))?;
        if now.duration_since(budget.start) >= Duration::from_secs(60) {
            budget.start = now;
            budget.requests = 0;
            budget.peers.clear();
        }
        if budget.requests >= 1200 {
            return Err(AppError::TooManyRequests {
                retry_after_seconds: 60,
            });
        }
        budget.requests += 1;
        if let Some(peer) = peer {
            let count = budget.peers.entry(peer).or_default();
            if *count >= 240 {
                return Err(AppError::TooManyRequests {
                    retry_after_seconds: 60,
                });
            }
            *count += 1;
        }
        Ok(())
    }
}
pub async fn guard(State(state): State<Admission>, req: Request, next: Next) -> Response {
    if req.uri().path() == "/healthz" {
        return next.run(req).await;
    }
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|p| p.0.ip());
    if let Err(error) = state.admit(peer, Instant::now()) {
        return error.into_response();
    }
    let Ok(_permit) = state.active.try_acquire() else {
        return AppError::ServiceUnavailable("Request capacity exhausted".into()).into_response();
    };
    // Reject before authentication and JSON extraction, even with chunked/no-length bodies.
    if req.uri().path() == "/api/mcp" && req.method() != Method::POST {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "POST")],
            axum::Json(
                serde_json::json!({"error": "Method not allowed. Use POST for MCP requests."}),
            ),
        )
            .into_response();
    }
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        let (parts, body) = req.into_parts();
        let bytes = to_bytes(body, state.body_limit).await.map_err(|_| {
            AppError::PayloadTooLarge("Request body exceeds configured limit".into())
        })?;
        Ok::<_, AppError>(
            next.run(Request::from_parts(parts, Body::from(bytes)))
                .await,
        )
    })
    .await;
    match result {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => error.into_response(),
        Err(_) => AppError::ServiceUnavailable("Request deadline exceeded".into()).into_response(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{middleware, routing::post, Router};
    use tower::ServiceExt;
    #[test]
    fn invalid_credentials_cannot_evade_global_budget_or_create_unbounded_peer_state() {
        let gate = Admission::new(65536);
        let now = Instant::now();
        for n in 0..1200 {
            assert!(gate
                .admit(Some(std::net::IpAddr::V4((n as u32).into())), now)
                .is_ok());
        }
        assert!(matches!(
            gate.admit(None, now),
            Err(AppError::TooManyRequests { .. })
        ));
        assert_eq!(gate.budget.lock().unwrap().peers.len(), 1200);
        assert!(gate.admit(None, now + Duration::from_secs(60)).is_ok());
    }
    #[test]
    fn direct_peer_budget_is_separate() {
        let gate = Admission::new(65536);
        let peer = "127.0.0.1".parse().unwrap();
        let now = Instant::now();
        for _ in 0..240 {
            assert!(gate.admit(Some(peer), now).is_ok());
        }
        assert!(gate.admit(Some(peer), now).is_err());
        assert!(gate.admit(Some("127.0.0.2".parse().unwrap()), now).is_ok());
    }
    #[tokio::test]
    async fn body_limit_is_inclusive_without_content_length() {
        let router = Router::new()
            .route("/body", post(|| async { StatusCode::NO_CONTENT }))
            .layer(middleware::from_fn_with_state(Admission::new(65536), guard));
        for (size, expected) in [
            (65536, StatusCode::NO_CONTENT),
            (65537, StatusCode::PAYLOAD_TOO_LARGE),
        ] {
            let req = Request::builder()
                .method("POST")
                .uri("/body")
                .body(Body::from(vec![b' '; size]))
                .unwrap();
            assert_eq!(
                router.clone().oneshot(req).await.unwrap().status(),
                expected
            );
        }
    }
}
