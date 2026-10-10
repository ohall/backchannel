//! Regression coverage for the October 2026 audit. No production traffic.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use backchannel_core::{create_router, db, token::hash_token, Config};
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

fn config() -> Config {
    Config {
        oauth: None,
        database_url: "unused".into(),
        database_schema: "backchannel_test".into(),
        admin_token_sha256: hash_token("admin-fixture"),
        viewer_token_sha256: None,
        default_rate_limit_per_minute: 1000,
        admin_rate_limit_per_minute: 1000,
        max_body_size_bytes: 65536,
        max_message_body_size_bytes: 32768,
    }
}
#[tokio::test]
async fn oversized_bodies_and_unsupported_mcp_methods_never_reach_auth_database() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://test@127.0.0.1:1/test")
        .unwrap();
    let router = create_router(pool, config());
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/channels")
                .header("content-type", "application/json")
                .body(Body::from(vec![b' '; 65537]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    for (size, expected) in [
        (65536, StatusCode::UNAUTHORIZED),
        (65537, StatusCode::PAYLOAD_TOO_LARGE),
    ] {
        let chunks = vec![
            Ok::<_, std::io::Error>(vec![b' '; 32768]),
            Ok(vec![b' '; size - 32768]),
        ];
        let body = Body::from_stream(futures::stream::iter(chunks));
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/channels")
                    .header("content-type", "application/json")
                    .body(body)
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    for method in ["GET", "HEAD", "PUT", "PATCH", "DELETE", "OPTIONS"] {
        let response = router.clone().oneshot(Request::builder().method(method).uri("/api/mcp")
            .header("content-type", "application/json").body(Body::from(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_channel","arguments":{"name":"must-not-exist"}}}"#)).unwrap()).await.unwrap();
        if method == "OPTIONS" {
            // Nonmutating CORS preflight is handled without the MCP dispatcher.
            assert_eq!(response.status(), StatusCode::OK);
        } else {
            assert_eq!(
                response.status(),
                StatusCode::METHOD_NOT_ALLOWED,
                "{method}"
            );
            assert_eq!(response.headers()["allow"], "POST");
        }
    }
}
#[tokio::test]
async fn escaped_feed_pages_preserve_every_message_under_byte_budget() {
    let url = std::env::var("TEST_DATABASE_URL").expect("Disposable TEST_DATABASE_URL required");
    let pool = db::create_pool(&url, "backchannel_test").await.unwrap();
    sqlx::query("TRUNCATE agents, conversations, dm_members, messages, rate_limit_buckets CASCADE")
        .execute(&pool)
        .await
        .unwrap();
    let (agent, _) = db::agents::create_agent(&pool, "audit-fixture")
        .await
        .unwrap();
    let channel = db::conversations::create_channel(&pool, "audit-fixture", None, agent.id)
        .await
        .unwrap();
    let mut expected = Vec::new();
    for n in 0..8 {
        let message = db::messages::create_message(
            &pool,
            channel.id,
            agent.id,
            &"\u{0001}".repeat(32768),
            &format!("large-{n}"),
            None,
        )
        .await
        .unwrap();
        expected.push(message.id.to_string());
    }
    expected.push(
        db::messages::create_message(
            &pool,
            channel.id,
            agent.id,
            "legitimate message after flood",
            "legitimate",
            None,
        )
        .await
        .unwrap()
        .id
        .to_string(),
    );
    let mut received = Vec::new();
    let mut cursor = 0;
    loop {
        let page = db::messages::list_feed(&pool, agent.id, cursor, 500)
            .await
            .unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len() <= 256 * 1024);
        received.extend(page.items.iter().map(|m| m.id.clone()));
        if !page.has_more {
            break;
        }
        assert!(!page.items.is_empty());
        cursor = page.next_cursor.unwrap().parse().unwrap();
    }
    assert_eq!(received, expected);
}
