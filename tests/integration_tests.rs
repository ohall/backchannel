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

#[tokio::test]
async fn test_11_pg_dump_restore_isolation() {
    let pool = setup_test_db().await;

    let (_agent_id, token) = create_test_agent(&pool, "alice").await;

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

    // Create test data
    let (status, body) = make_request(
        &router,
        "POST",
        "/v1/channels",
        Some(&token),
        Some(json!({"name": "general", "description": "Test channel"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let conv_id = body["id"].as_str().unwrap().to_string();

    let (status, _) = make_request(
        &router,
        "POST",
        &format!("/v1/conversations/{}/messages", conv_id),
        Some(&token),
        Some(json!({
            "client_message_id": "test-msg-1",
            "body": "Test message content"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Dump the schema
    let database_url = std::env::var("TEST_DATABASE_URL").unwrap();
    let schema_name = "backchannel_test";

    let dump_output = std::process::Command::new("pg_dump")
        .arg(database_url.clone())
        .arg("--schema")
        .arg(schema_name)
        .arg("--schema-only")
        .arg("--no-owner")
        .arg("--no-acl")
        .output()
        .expect("Failed to run pg_dump");

    assert!(
        dump_output.status.success(),
        "pg_dump failed: {}",
        String::from_utf8_lossy(&dump_output.stderr)
    );

    let dump_sql = String::from_utf8(dump_output.stdout).unwrap();

    // Verify dump contains expected schema elements
    assert!(
        dump_sql.contains("CREATE TABLE"),
        "Dump should contain CREATE TABLE statements"
    );
    assert!(
        dump_sql.contains("agents"),
        "Dump should contain agents table"
    );
    assert!(
        dump_sql.contains("conversations"),
        "Dump should contain conversations table"
    );
    assert!(
        dump_sql.contains("messages"),
        "Dump should contain messages table"
    );
    assert!(
        dump_sql.contains("rate_limit_buckets"),
        "Dump should contain rate_limit_buckets table"
    );

    // Create a new isolated schema and restore the structure
    let restore_schema = "backchannel_restore_test";
    sqlx::query(&format!("DROP SCHEMA IF EXISTS {} CASCADE", restore_schema))
        .execute(&pool)
        .await
        .unwrap();

    // Prepare modified dump that creates and uses the new schema
    let modified_dump = dump_sql
        .lines()
        .filter(|line| !line.starts_with("\\"))
        .collect::<Vec<_>>()
        .join("\n")
        .replace(
            &format!("CREATE SCHEMA {};", schema_name),
            &format!("CREATE SCHEMA {};", restore_schema),
        )
        .replace(
            &format!("{}.", schema_name),
            &format!("{}.", restore_schema),
        )
        .replace(
            &format!("SET search_path = {}", schema_name),
            &format!("SET search_path = {}", restore_schema),
        );

    use std::io::Write;
    let mut restore_cmd = std::process::Command::new("psql")
        .arg(&database_url)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to spawn psql restore");

    restore_cmd
        .stdin
        .as_mut()
        .unwrap()
        .write_all(modified_dump.as_bytes())
        .expect("Failed to write to psql stdin");

    let restore_output = restore_cmd
        .wait_with_output()
        .expect("Failed to wait for psql");

    if !restore_output.status.success() {
        eprintln!(
            "Restore stderr: {}",
            String::from_utf8_lossy(&restore_output.stderr)
        );
        panic!("Restore failed");
    }

    // Verify schema exists
    let schema_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.schemata WHERE schema_name = $1",
    )
    .bind(restore_schema)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(schema_count, 1, "Restored schema not found");

    // Verify tables exist in restored schema
    let table_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pg_tables WHERE schemaname = $1")
            .bind(restore_schema)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert!(
        table_count >= 5,
        "Expected at least 5 tables in restored schema, got {}",
        table_count
    );

    // Verify specific tables exist
    for table in &[
        "agents",
        "conversations",
        "messages",
        "dm_members",
        "rate_limit_buckets",
    ] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_tables WHERE schemaname = $1 AND tablename = $2)",
        )
        .bind(restore_schema)
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert!(exists, "Table {} not found in restored schema", table);
    }

    // Clean up
    sqlx::query(&format!("DROP SCHEMA {} CASCADE", restore_schema))
        .execute(&pool)
        .await
        .unwrap();
}

// MCP endpoint tests

#[tokio::test]
async fn test_mcp_initialize() {
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

    let (_agent_id, token) = create_test_agent(&pool, "mcp-tester").await;

    // Test initialize
    let (status, body) = make_request(
        &router,
        "POST",
        "/api/mcp",
        Some(&token),
        Some(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "clientInfo": {
                    "name": "test-client",
                    "version": "1.0.0"
                }
            }
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 1);
    assert_eq!(body["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(body["result"]["serverInfo"]["name"], "backchannel");
    assert!(body["result"]["capabilities"]["tools"].is_object());
}

#[tokio::test]
async fn test_mcp_tools_list() {
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

    let (_agent_id, token) = create_test_agent(&pool, "mcp-tester").await;

    // Test tools/list
    let (status, body) = make_request(
        &router,
        "POST",
        "/api/mcp",
        Some(&token),
        Some(json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 2);
    assert!(body["result"]["tools"].is_array());

    let tools = body["result"]["tools"].as_array().unwrap();
    let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();

    assert!(tool_names.contains(&"whoami"));
    assert!(tool_names.contains(&"list_channels"));
    assert!(tool_names.contains(&"create_channel"));
    assert!(tool_names.contains(&"post_message"));
    assert!(tool_names.contains(&"reply"));
    assert!(tool_names.contains(&"read_messages"));
    assert!(tool_names.contains(&"open_dm"));
    assert!(tool_names.contains(&"list_dms"));
    assert!(tool_names.contains(&"feed"));
}

#[tokio::test]
async fn test_mcp_whoami() {
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

    // Test whoami tool
    let (status, body) = make_request(
        &router,
        "POST",
        "/api/mcp",
        Some(&token),
        Some(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "whoami",
                "arguments": {}
            }
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 3);
    assert!(body["result"]["content"].is_array());

    let content = &body["result"]["content"][0];
    assert_eq!(content["type"], "text");

    let text = content["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["name"], "alice");
    assert_eq!(parsed["enabled"], true);
}

#[tokio::test]
async fn test_mcp_post_message_and_read() {
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

    let (_agent_id, token) = create_test_agent(&pool, "bob").await;

    // Create a channel via REST
    let (status, channel_body) = make_request(
        &router,
        "POST",
        "/v1/channels",
        Some(&token),
        Some(json!({"name": "mcp-test", "description": "MCP test channel"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let conversation_id = channel_body["id"].as_str().unwrap();

    // Post message via MCP
    let (status, body) = make_request(
        &router,
        "POST",
        "/api/mcp",
        Some(&token),
        Some(json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "post_message",
                "arguments": {
                    "conversation_id": conversation_id,
                    "body": "Hello from MCP!",
                    "client_message_id": "mcp-msg-1"
                }
            }
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["jsonrpc"], "2.0");
    let content = &body["result"]["content"][0];
    let text = content["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["body"], "Hello from MCP!");

    // Read messages via MCP
    let (status, body) = make_request(
        &router,
        "POST",
        "/api/mcp",
        Some(&token),
        Some(json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "read_messages",
                "arguments": {
                    "conversation_id": conversation_id,
                    "after": "0",
                    "limit": 10
                }
            }
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let content = &body["result"]["content"][0];
    let text = content["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["items"][0]["body"], "Hello from MCP!");
}

#[tokio::test]
async fn test_mcp_auth_failure() {
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

    // No token
    let (status, _body) = make_request(
        &router,
        "POST",
        "/api/mcp",
        None,
        Some(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Invalid token
    let (status, _body) = make_request(
        &router,
        "POST",
        "/api/mcp",
        Some("invalid-token"),
        Some(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_mcp_dm_isolation() {
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

    let (_alice_id, alice_token) = create_test_agent(&pool, "alice").await;
    let (bob_id, bob_token) = create_test_agent(&pool, "bob").await;
    let (_charlie_id, charlie_token) = create_test_agent(&pool, "charlie").await;

    // Alice opens DM with Bob
    let (status, body) = make_request(
        &router,
        "POST",
        "/api/mcp",
        Some(&alice_token),
        Some(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "open_dm",
                "arguments": {
                    "recipient_agent_id": bob_id
                }
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let content = &body["result"]["content"][0];
    let text = content["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    let dm_id = parsed["id"].as_str().unwrap();

    // Alice posts message
    let (status, _) = make_request(
        &router,
        "POST",
        "/api/mcp",
        Some(&alice_token),
        Some(json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "post_message",
                "arguments": {
                    "conversation_id": dm_id,
                    "body": "Secret message",
                    "client_message_id": "alice-dm-1"
                }
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Bob can read
    let (status, body) = make_request(
        &router,
        "POST",
        "/api/mcp",
        Some(&bob_token),
        Some(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "read_messages",
                "arguments": {
                    "conversation_id": dm_id
                }
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let content = &body["result"]["content"][0];
    let text = content["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["items"][0]["body"], "Secret message");

    // Charlie cannot read (should get error)
    let (status, body) = make_request(
        &router,
        "POST",
        "/api/mcp",
        Some(&charlie_token),
        Some(json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "read_messages",
                "arguments": {
                    "conversation_id": dm_id
                }
            }
        })),
    )
    .await;
    // Tool errors now return 200 with isError: true
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["isError"], true);
    // DM isolation returns "Conversation not found" to avoid leaking DM existence
    assert!(body["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Conversation not found"));
}

// New MCP compatibility tests

#[tokio::test]
async fn test_mcp_notifications() {
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

    let (_agent_id, token) = create_test_agent(&pool, "notif-tester").await;

    // Test notification (no id field) - should get 202 Accepted
    let (status, _body) = make_request(
        &router,
        "POST",
        "/api/mcp",
        Some(&token),
        Some(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        })),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED);
}

#[tokio::test]
async fn test_mcp_protocol_version_negotiation() {
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

    let (_agent_id, token) = create_test_agent(&pool, "version-tester").await;

    // Test supported version 2025-03-26
    let (status, body) = make_request(
        &router,
        "POST",
        "/api/mcp",
        Some(&token),
        Some(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "clientInfo": {
                    "name": "test-client",
                    "version": "1.0.0"
                }
            }
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["protocolVersion"], "2025-03-26");

    // Test supported version 2025-11-25
    let (status, body) = make_request(
        &router,
        "POST",
        "/api/mcp",
        Some(&token),
        Some(json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "clientInfo": {
                    "name": "test-client",
                    "version": "1.0.0"
                }
            }
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["protocolVersion"], "2025-11-25");

    // Test unsupported version - should negotiate to latest
    let (status, body) = make_request(
        &router,
        "POST",
        "/api/mcp",
        Some(&token),
        Some(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "initialize",
            "params": {
                "protocolVersion": "2099-01-01",
                "clientInfo": {
                    "name": "test-client",
                    "version": "1.0.0"
                }
            }
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["protocolVersion"], "2025-11-25");
}

#[tokio::test]
async fn test_mcp_tool_error_vs_protocol_error() {
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

    let (_agent_id, token) = create_test_agent(&pool, "error-tester").await;

    // Test tool not found - should be isError result, not JSON-RPC error
    let (status, body) = make_request(
        &router,
        "POST",
        "/api/mcp",
        Some(&token),
        Some(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "nonexistent_tool",
                "arguments": {}
            }
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["result"].is_object());
    assert_eq!(body["result"]["isError"], true);
    assert!(body["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Unknown tool"));

    // Test tool validation error - should be isError result
    let (status, body) = make_request(
        &router,
        "POST",
        "/api/mcp",
        Some(&token),
        Some(json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "create_channel",
                "arguments": {}  // Missing required 'name' field
            }
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["result"].is_object());
    assert_eq!(body["result"]["isError"], true);

    // Test protocol error (unknown method) - should be JSON-RPC error
    let (status, body) = make_request(
        &router,
        "POST",
        "/api/mcp",
        Some(&token),
        Some(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "unknown/method"
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["error"].is_object());
    assert_eq!(body["error"]["code"], -32601);
}

#[tokio::test]
async fn test_mcp_get_and_ping() {
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

    let (_agent_id, token) = create_test_agent(&pool, "ping-tester").await;

    // Test GET request - should return 405
    let (status, body) = make_request(&router, "GET", "/api/mcp", Some(&token), None).await;

    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("Method not allowed"));

    // Test ping
    let (status, body) = make_request(
        &router,
        "POST",
        "/api/mcp",
        Some(&token),
        Some(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "ping"
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["result"].is_object());
}

#[tokio::test]
async fn test_mcp_content_type_header() {
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

    let (_agent_id, token) = create_test_agent(&pool, "header-tester").await;

    // Make a request and verify Content-Type header
    let req = Request::builder()
        .method("POST")
        .uri("/api/mcp")
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "ping"
            }))
            .unwrap(),
        ))
        .unwrap();

    let response = router.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok());
    assert_eq!(content_type, Some("application/json"));
}
