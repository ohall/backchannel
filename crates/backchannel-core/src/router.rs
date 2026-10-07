use crate::auth::{authenticate_admin, authenticate_agent, AuthState};
use crate::config::Config;
use crate::handlers;
use axum::{
    middleware,
    routing::{any, get, patch, post},
    Router,
};
use sqlx::PgPool;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

pub fn create_router(pool: PgPool, config: Config) -> Router {
    let auth_state = AuthState {
        pool: pool.clone(),
        config: Arc::new(config),
    };

    // Public routes (no auth)
    let public_routes = Router::new().route("/healthz", get(handlers::healthz));

    // Agent-authenticated routes
    let agent_routes = Router::new()
        .route("/v1/me", get(handlers::get_me))
        .route("/v1/agents", get(handlers::list_agents))
        .route("/v1/channels", get(handlers::list_channels))
        .route("/v1/channels", post(handlers::create_channel))
        .route("/v1/dms", get(handlers::list_dms))
        .route("/v1/dms", post(handlers::create_dm))
        .route(
            "/v1/conversations/:id/messages",
            get(handlers::list_messages),
        )
        .route(
            "/v1/conversations/:id/messages",
            post(handlers::create_message),
        )
        .route("/v1/feed", get(handlers::get_feed))
        .route("/api/mcp", any(handlers::mcp_handler))
        .layer(middleware::from_fn_with_state(
            auth_state.clone(),
            authenticate_agent,
        ))
        .with_state(pool.clone());

    // Admin-authenticated routes
    let admin_routes = Router::new()
        .route("/v1/admin/agents", post(handlers::create_agent))
        .route(
            "/v1/admin/agents/:id/rotate-token",
            post(handlers::rotate_token),
        )
        .route("/v1/admin/agents/:id", patch(handlers::update_agent))
        .route("/v1/admin/messages", get(handlers::list_all_messages))
        .route("/v1/admin/export", get(handlers::export_messages))
        .layer(middleware::from_fn_with_state(
            auth_state,
            authenticate_admin,
        ))
        .with_state(pool);

    // Combine all routes
    Router::new()
        .merge(public_routes)
        .merge(agent_routes)
        .merge(admin_routes)
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        )
}
