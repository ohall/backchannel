use axum::body::Body;
use axum::http::{Request, StatusCode};
use backchannel_core::{create_router, db, Config};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;

// Test helpers

async fn setup_test_db() -> PgPool {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL must be set for integration tests");

    let schema =
        std::env::var("TEST_DATABASE_SCHEMA").unwrap_or_else(|_| "backchannel_test".to_string());

    let pool = db::create_pool(&database_url, &schema)
        .await
        .expect("Failed to create test database pool");

    // Clean database
    sqlx::query("TRUNCATE agents, conversations, dm_members, messages, rate_limit_buckets CASCADE")
        .execute(&pool)
        .await
        .expect("Failed to clean test database");

    pool
}

async fn create_test_agent(pool: &PgPool, name: &str) -> (String, String) {
    let (agent, token) = backchannel_core::db::agents::create_agent(pool, name)
        .await
        .expect("Failed to create test agent");
    (agent.id.to_string(), token)
}

async fn make_request(
    router: &axum::Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(path);

    if let Some(tok) = token {
        req = req.header("Authorization", format!("Bearer {}", tok));
    }

    if let Some(b) = body {
        req = req.header("Content-Type", "application/json");
        let req = req
            .body(Body::from(serde_json::to_vec(&b).unwrap()))
            .unwrap();
        let response = router.clone().oneshot(req).await.unwrap();
        let status = response.status();
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_json: Value = serde_json::from_slice(&body_bytes).unwrap_or(json!({}));
        (status, body_json)
    } else {
        let req = req.body(Body::empty()).unwrap();
        let response = router.clone().oneshot(req).await.unwrap();
        let status = response.status();
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_json: Value = serde_json::from_slice(&body_bytes).unwrap_or(json!({}));
        (status, body_json)
    }
}

#[tokio::test]
async fn test_1_authentication() {
    let pool = setup_test_db().await;
    let config = Config {
        database_schema: "backchannel_test".to_string(),
        database_url: "unused".to_string(),
        admin_token_sha256: backchannel_core::token::hash_token("admin_token"),
        default_rate_limit_per_minute: 120,
        admin_rate_limit_per_minute: 300,
        max_body_size_bytes: 64 * 1024,
        max_message_body_size_bytes: 32 * 1024,
    };
    let router = create_router(pool.clone(), config);

    // Create two agents
    let (_agent1_id, token1) = create_test_agent(&pool, "alice").await;
    let (_agent2_id, token2) = create_test_agent(&pool, "bob").await;

    // Valid token
    let (status, body) = make_request(&router, "GET", "/v1/me", Some(&token1), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "alice");

    // Another valid token
    let (status, body) = make_request(&router, "GET", "/v1/me", Some(&token2), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "bob");

    // Invalid token
    let (status, _) = make_request(&router, "GET", "/v1/me", Some("invalid"), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Disabled agent
    sqlx::query("UPDATE agents SET enabled = false WHERE name = 'alice'")
        .execute(&pool)
        .await
        .unwrap();
    let (status, _) = make_request(&router, "GET", "/v1/me", Some(&token1), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Rotated token
    sqlx::query("UPDATE agents SET enabled = true WHERE name = 'alice'")
        .execute(&pool)
        .await
        .unwrap();
    let new_token = backchannel_core::db::agents::rotate_agent_token(
        &pool,
        uuid::Uuid::parse_str(&_agent1_id).unwrap(),
    )
    .await
    .unwrap();

    let (status, _) = make_request(&router, "GET", "/v1/me", Some(&token1), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = make_request(&router, "GET", "/v1/me", Some(&new_token), None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn test_2_public_channel_operations() {
    let pool = setup_test_db().await;
    let config = Config {
        database_schema: "backchannel_test".to_string(),
        database_url: "unused".to_string(),
        admin_token_sha256: backchannel_core::token::hash_token("admin_token"),
        default_rate_limit_per_minute: 120,
        admin_rate_limit_per_minute: 300,
        max_body_size_bytes: 64 * 1024,
        max_message_body_size_bytes: 32 * 1024,
    };
    let router = create_router(pool.clone(), config);

    let (_agent1_id, token1) = create_test_agent(&pool, "alice").await;
    let (_agent2_id, token2) = create_test_agent(&pool, "bob").await;

    // Create channel
    let (status, body) = make_request(
        &router,
        "POST",
        "/v1/channels",
        Some(&token1),
        Some(json!({"name": "general", "description": "General chat"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["name"], "general");
    let conv_id = body["id"].as_str().unwrap();

    // Both agents can discover it
    let (status, body) = make_request(&router, "GET", "/v1/channels", Some(&token1), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 1);

    let (status, body) = make_request(&router, "GET", "/v1/channels", Some(&token2), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 1);

    // Both agents can post
    let (status, body) = make_request(
        &router,
        "POST",
        &format!("/v1/conversations/{}/messages", conv_id),
        Some(&token1),
        Some(json!({"body": "Hello from Alice", "client_message_id": "alice-1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let msg1_id = body["id"].as_str().unwrap();

    let (status, _) = make_request(
        &router,
        "POST",
        &format!("/v1/conversations/{}/messages", conv_id),
        Some(&token2),
        Some(
            json!({"body": "Hello from Bob", "client_message_id": "bob-1", "reply_to_id": msg1_id}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Both agents can read
    let (status, body) = make_request(
        &router,
        "GET",
        &format!("/v1/conversations/{}/messages", conv_id),
        Some(&token1),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 2);

    let (status, body) = make_request(
        &router,
        "GET",
        &format!("/v1/conversations/{}/messages", conv_id),
        Some(&token2),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_3_dm_privacy() {
    let pool = setup_test_db().await;
    let config = Config {
        database_schema: "backchannel_test".to_string(),
        database_url: "unused".to_string(),
        admin_token_sha256: backchannel_core::token::hash_token("admin_token"),
        default_rate_limit_per_minute: 120,
        admin_rate_limit_per_minute: 300,
        max_body_size_bytes: 64 * 1024,
        max_message_body_size_bytes: 32 * 1024,
    };
    let router = create_router(pool.clone(), config);

    let (_agent1_id, token1) = create_test_agent(&pool, "alice").await;
    let (agent2_id, token2) = create_test_agent(&pool, "bob").await;
    let (_agent3_id, token3) = create_test_agent(&pool, "charlie").await;

    // Create DM between alice and bob
    let (status, body) = make_request(
        &router,
        "POST",
        "/v1/dms",
        Some(&token1),
        Some(json!({"recipient_agent_id": agent2_id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let dm_id = body["id"].as_str().unwrap();

    // Post message in DM
    let (status, _) = make_request(
        &router,
        "POST",
        &format!("/v1/conversations/{}/messages", dm_id),
        Some(&token1),
        Some(json!({"body": "Private message", "client_message_id": "alice-dm-1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Alice can read
    let (status, body) = make_request(
        &router,
        "GET",
        &format!("/v1/conversations/{}/messages", dm_id),
        Some(&token1),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 1);

    // Bob can read
    let (status, body) = make_request(
        &router,
        "GET",
        &format!("/v1/conversations/{}/messages", dm_id),
        Some(&token2),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 1);

    // Charlie cannot read (404, not 403, to avoid enumeration)
    let (status, _) = make_request(
        &router,
        "GET",
        &format!("/v1/conversations/{}/messages", dm_id),
        Some(&token3),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Charlie cannot see DM in list
    let (status, body) = make_request(&router, "GET", "/v1/dms", Some(&token3), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 0);

    // Charlie cannot see DM messages in feed
    let (status, body) = make_request(&router, "GET", "/v1/feed", Some(&token3), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 0);

    // Non-admin cannot access admin endpoints
    let (status, _) = make_request(&router, "GET", "/v1/admin/messages", Some(&token1), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_4_concurrent_dm_creation() {
    let pool = setup_test_db().await;

    let (agent1_id, _token1) = create_test_agent(&pool, "alice").await;
    let (agent2_id, _token2) = create_test_agent(&pool, "bob").await;
    let agent1_uuid = uuid::Uuid::parse_str(&agent1_id).unwrap();
    let agent2_uuid = uuid::Uuid::parse_str(&agent2_id).unwrap();

    // Simulate concurrent DM creation
    let mut handles = vec![];
    for _ in 0..5 {
        let pool_clone = pool.clone();
        let a1 = agent1_uuid;
        let a2 = agent2_uuid;
        let handle = tokio::spawn(async move {
            backchannel_core::db::conversations::create_or_get_dm(&pool_clone, a1, a2).await
        });
        handles.push(handle);
    }

    let results: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap().unwrap())
        .collect();

    // All should return the same conversation ID
    let first_id = results[0].id;
    for conv in &results {
        assert_eq!(conv.id, first_id);
    }

    // Verify exactly 2 members
    let member_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM dm_members WHERE conversation_id = $1")
            .bind(first_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(member_count, 2);
}

#[tokio::test]
async fn test_5_message_idempotency() {
    let pool = setup_test_db().await;
    let config = Config {
        database_schema: "backchannel_test".to_string(),
        database_url: "unused".to_string(),
        admin_token_sha256: backchannel_core::token::hash_token("admin_token"),
        default_rate_limit_per_minute: 120,
        admin_rate_limit_per_minute: 300,
        max_body_size_bytes: 64 * 1024,
        max_message_body_size_bytes: 32 * 1024,
    };
    let router = create_router(pool.clone(), config);

    let (_agent1_id, token1) = create_test_agent(&pool, "alice").await;

    // Create channel
    let (_, body) = make_request(
        &router,
        "POST",
        "/v1/channels",
        Some(&token1),
        Some(json!({"name": "general"})),
    )
    .await;
    let conv_id = body["id"].as_str().unwrap();

    // Post message
    let (status, body1) = make_request(
        &router,
        "POST",
        &format!("/v1/conversations/{}/messages", conv_id),
        Some(&token1),
        Some(json!({"body": "Test message", "client_message_id": "test-1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let msg_id1 = body1["id"].as_str().unwrap();

    // Repeat with same client_message_id and identical content
    let (status, body2) = make_request(
        &router,
        "POST",
        &format!("/v1/conversations/{}/messages", conv_id),
        Some(&token1),
        Some(json!({"body": "Test message", "client_message_id": "test-1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let msg_id2 = body2["id"].as_str().unwrap();

    // Should return same message
    assert_eq!(msg_id1, msg_id2);

    // Verify only one message in database
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE conversation_id = $1")
        .bind(uuid::Uuid::parse_str(conv_id).unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);

    // Reuse client_message_id with different content - should conflict
    let (status, _) = make_request(
        &router,
        "POST",
        &format!("/v1/conversations/{}/messages", conv_id),
        Some(&token1),
        Some(json!({"body": "Different message", "client_message_id": "test-1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn test_6_pagination_ordering() {
    let pool = setup_test_db().await;
    let config = Config {
        database_schema: "backchannel_test".to_string(),
        database_url: "unused".to_string(),
        admin_token_sha256: backchannel_core::token::hash_token("admin_token"),
        default_rate_limit_per_minute: 120,
        admin_rate_limit_per_minute: 300,
        max_body_size_bytes: 64 * 1024,
        max_message_body_size_bytes: 32 * 1024,
    };
    let router = create_router(pool.clone(), config);

    let (_agent1_id, token1) = create_test_agent(&pool, "alice").await;

    // Create channel
    let (_, body) = make_request(
        &router,
        "POST",
        "/v1/channels",
        Some(&token1),
        Some(json!({"name": "general"})),
    )
    .await;
    let conv_id = body["id"].as_str().unwrap();

    // Post 10 messages
    for i in 0..10 {
        make_request(
            &router,
            "POST",
            &format!("/v1/conversations/{}/messages", conv_id),
            Some(&token1),
            Some(json!({"body": format!("Message {}", i), "client_message_id": format!("msg-{}", i)}))
        ).await;
    }

    // Fetch with limit=5
    let (status, body) = make_request(
        &router,
        "GET",
        &format!("/v1/conversations/{}/messages?limit=5", conv_id),
        Some(&token1),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let page1 = body["items"].as_array().unwrap();
    assert_eq!(page1.len(), 5);
    assert_eq!(body["has_more"], true);

    let cursor = body["next_cursor"].as_str().unwrap();

    // Fetch next page
    let (status, body) = make_request(
        &router,
        "GET",
        &format!(
            "/v1/conversations/{}/messages?after={}&limit=5",
            conv_id, cursor
        ),
        Some(&token1),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let page2 = body["items"].as_array().unwrap();
    assert_eq!(page2.len(), 5);
    assert_eq!(body["has_more"], false);

    // Verify ordering (ascending IDs)
    let ids1: Vec<i64> = page1
        .iter()
        .map(|m| m["id"].as_str().unwrap().parse().unwrap())
        .collect();
    let ids2: Vec<i64> = page2
        .iter()
        .map(|m| m["id"].as_str().unwrap().parse().unwrap())
        .collect();

    for i in 1..ids1.len() {
        assert!(ids1[i] > ids1[i - 1]);
    }
    for i in 1..ids2.len() {
        assert!(ids2[i] > ids2[i - 1]);
    }
    assert!(ids2[0] > ids1[ids1.len() - 1]);
}

#[tokio::test]
async fn test_7_validation_and_limits() {
    let pool = setup_test_db().await;
    let config = Config {
        database_schema: "backchannel_test".to_string(),
        database_url: "unused".to_string(),
        admin_token_sha256: backchannel_core::token::hash_token("admin_token"),
        default_rate_limit_per_minute: 120,
        admin_rate_limit_per_minute: 300,
        max_body_size_bytes: 64 * 1024,
        max_message_body_size_bytes: 32 * 1024,
    };
    let router = create_router(pool.clone(), config);

    let (_agent1_id, token1) = create_test_agent(&pool, "alice").await;

    // Create channel
    let (_, body) = make_request(
        &router,
        "POST",
        "/v1/channels",
        Some(&token1),
        Some(json!({"name": "general"})),
    )
    .await;
    let conv_id = body["id"].as_str().unwrap();

    // Cross-conversation reply (create another channel first)
    let (_, body2) = make_request(
        &router,
        "POST",
        "/v1/channels",
        Some(&token1),
        Some(json!({"name": "other"})),
    )
    .await;
    let other_conv_id = body2["id"].as_str().unwrap();

    let (_, msg_body) = make_request(
        &router,
        "POST",
        &format!("/v1/conversations/{}/messages", conv_id),
        Some(&token1),
        Some(json!({"body": "Message 1", "client_message_id": "msg-1"})),
    )
    .await;
    let msg_id = msg_body["id"].as_str().unwrap();

    let (status, _) = make_request(
        &router,
        "POST",
        &format!("/v1/conversations/{}/messages", other_conv_id),
        Some(&token1),
        Some(json!({"body": "Reply", "client_message_id": "msg-2", "reply_to_id": msg_id})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Oversized body (> 32 KiB)
    let large_body = "x".repeat(33 * 1024);
    let (status, _) = make_request(
        &router,
        "POST",
        &format!("/v1/conversations/{}/messages", conv_id),
        Some(&token1),
        Some(json!({"body": large_body, "client_message_id": "msg-3"})),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);

    // Blank body
    let (status, _) = make_request(
        &router,
        "POST",
        &format!("/v1/conversations/{}/messages", conv_id),
        Some(&token1),
        Some(json!({"body": "   ", "client_message_id": "msg-4"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Invalid cursor
    let (status, _) = make_request(
        &router,
        "GET",
        &format!("/v1/conversations/{}/messages?after=invalid", conv_id),
        Some(&token1),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_8_admin_operations() {
    let pool = setup_test_db().await;
    let admin_token = "admin_secret";
    let config = Config {
        database_schema: "backchannel_test".to_string(),
        database_url: "unused".to_string(),
        admin_token_sha256: backchannel_core::token::hash_token(admin_token),
        default_rate_limit_per_minute: 120,
        admin_rate_limit_per_minute: 300,
        max_body_size_bytes: 64 * 1024,
        max_message_body_size_bytes: 32 * 1024,
    };
    let router = create_router(pool.clone(), config);

    let (_agent1_id, token1) = create_test_agent(&pool, "alice").await;
    let (agent2_id, _token2) = create_test_agent(&pool, "bob").await;

    // Create DM and post message
    let (_, body) = make_request(
        &router,
        "POST",
        "/v1/dms",
        Some(&token1),
        Some(json!({"recipient_agent_id": agent2_id})),
    )
    .await;
    let dm_id = body["id"].as_str().unwrap();

    make_request(
        &router,
        "POST",
        &format!("/v1/conversations/{}/messages", dm_id),
        Some(&token1),
        Some(json!({"body": "Private DM", "client_message_id": "dm-1"})),
    )
    .await;

    // Create public channel and post
    let (_, body) = make_request(
        &router,
        "POST",
        "/v1/channels",
        Some(&token1),
        Some(json!({"name": "general"})),
    )
    .await;
    let channel_id = body["id"].as_str().unwrap();

    make_request(
        &router,
        "POST",
        &format!("/v1/conversations/{}/messages", channel_id),
        Some(&token1),
        Some(json!({"body": "Public message", "client_message_id": "pub-1"})),
    )
    .await;

    // Admin can see all messages
    let (status, body) = make_request(
        &router,
        "GET",
        "/v1/admin/messages",
        Some(admin_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 2);

    // Admin can filter by conversation
    let (status, body) = make_request(
        &router,
        "GET",
        &format!("/v1/admin/messages?conversation_id={}", dm_id),
        Some(admin_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["items"][0]["body"], "Private DM");

    // Admin can export
    let (status, _) =
        make_request(&router, "GET", "/v1/admin/export", Some(admin_token), None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn test_10_database_unavailability() {
    // This test verifies graceful degradation when database is unavailable
    // In a real scenario, you'd stop the database or use a wrong connection string
    // For this test, we'll just verify that database errors don't leak sensitive info

    let pool = setup_test_db().await;
    let config = Config {
        database_schema: "backchannel_test".to_string(),
        database_url: "unused".to_string(),
        admin_token_sha256: backchannel_core::token::hash_token("admin_token"),
        default_rate_limit_per_minute: 120,
        admin_rate_limit_per_minute: 300,
        max_body_size_bytes: 64 * 1024,
        max_message_body_size_bytes: 32 * 1024,
    };
    let router = create_router(pool.clone(), config);

    let (_agent_id, token) = create_test_agent(&pool, "alice").await;

    // Close all connections to simulate unavailability
    pool.close().await;

    // Request should fail gracefully with 503
    let (status, body) = make_request(&router, "GET", "/v1/channels", Some(&token), None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unavailable"));
    // Should not contain SQL errors or connection strings
    assert!(!body["error"]["message"]
        .as_str()
        .unwrap()
        .to_lowercase()
        .contains("sql"));
    assert!(!body["error"]["message"]
        .as_str()
        .unwrap()
        .to_lowercase()
        .contains("postgres"));
}
