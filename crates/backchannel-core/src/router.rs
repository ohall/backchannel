use crate::auth::{
    authenticate_admin, authenticate_agent, authenticate_mcp, authenticate_read, read_no_store,
    AuthState,
};
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
    let oauth = config
        .oauth
        .clone()
        .map(crate::oauth::OAuthVerifier::new)
        .transpose()
        .expect("Invalid OAuth configuration");
    create_router_with_verifier(pool, config, oauth)
}

pub(crate) fn create_router_with_verifier(
    pool: PgPool,
    config: Config,
    oauth: Option<crate::oauth::OAuthVerifier>,
) -> Router {
    crate::config::validate_viewer_hash(
        config.viewer_token_sha256.as_deref(),
        &config.admin_token_sha256,
    )
    .expect("Invalid viewer credential configuration");
    let admission = crate::admission::Admission::new(config.max_body_size_bytes);
    let auth_state = AuthState {
        oauth: oauth.clone(),
        pool: pool.clone(),
        config: Arc::new(config),
    };

    // Public routes (no auth)
    let mut public_routes = Router::new().route("/healthz", get(handlers::healthz));
    if let Some(verifier) = oauth.clone() {
        public_routes = public_routes.merge(
            Router::new()
                .route(
                    "/.well-known/oauth-protected-resource/api/mcp",
                    get(crate::oauth::protected_resource_metadata),
                )
                .route(
                    "/.well-known/oauth-protected-resource",
                    get(crate::oauth::protected_resource_metadata),
                )
                .with_state(verifier),
        );
    }
    let mcp_routes = Router::new()
        .route("/api/mcp", any(handlers::mcp_handler))
        .route_layer(middleware::from_fn_with_state(
            auth_state.clone(),
            authenticate_mcp,
        ))
        .layer(axum::Extension(oauth.is_some()))
        .with_state(pool.clone());

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
        .route_layer(middleware::from_fn_with_state(
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
        .route_layer(middleware::from_fn_with_state(
            auth_state.clone(),
            authenticate_admin,
        ))
        .with_state(pool.clone());

    // No AdminAuth is ever granted here. New GETs require explicit registration.
    let read_routes = Router::new()
        .route("/v1/admin/messages", get(handlers::list_all_messages))
        .route("/v1/admin/export", get(handlers::export_messages))
        .route(
            "/v1/admin/conversations",
            get(handlers::viewer::conversations),
        )
        .route("/v1/admin/agents", get(handlers::viewer::agents))
        .route(
            "/v1/admin/conversations/:id/messages",
            get(handlers::viewer::messages),
        )
        .route("/v1/admin/search", get(handlers::viewer::search))
        .route_layer(middleware::from_fn_with_state(
            auth_state,
            authenticate_read,
        ))
        .layer(middleware::from_fn(read_no_store))
        .with_state(pool);

    // Combine all routes
    Router::new()
        .merge(public_routes)
        .merge(agent_routes)
        .merge(mcp_routes)
        .merge(admin_routes)
        .merge(read_routes)
        .layer(middleware::from_fn_with_state(
            admission,
            crate::admission::guard,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        )
}
