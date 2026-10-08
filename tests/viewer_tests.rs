use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use backchannel_core::{create_router, db, token::hash_token, Config};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

fn config() -> Config {
    Config {
        oauth: None,
        database_url: "unused".into(),
        database_schema: "backchannel_test".into(),
        admin_token_sha256: hash_token("admin-fixture"),
        viewer_token_sha256: Some(hash_token("viewer-fixture")),
        default_rate_limit_per_minute: 1000,
        admin_rate_limit_per_minute: 1000,
        max_body_size_bytes: 65536,
        max_message_body_size_bytes: 32768,
    }
}
async fn setup() -> PgPool {
    let pool = db::create_pool(
        &std::env::var("TEST_DATABASE_URL").expect("Disposable TEST_DATABASE_URL required"),
        "backchannel_test",
    )
    .await
    .unwrap();
    sqlx::query("TRUNCATE agents, conversations, dm_members, messages, rate_limit_buckets CASCADE")
        .execute(&pool)
        .await
        .unwrap();
    pool
}
async fn request(
    router: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> (StatusCode, Value, String) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("Content-Type", "application/json");
    if let Some(token) = token {
        req = req.header("Authorization", format!("Bearer {token}"));
    }
    let response = router
        .clone()
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let cache = response
        .headers()
        .get("cache-control")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let bytes = axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        cache,
    )
}
async fn get(router: &Router, path: &str) -> Value {
    let (status, value, cache) =
        request(router, "GET", path, Some("viewer-fixture"), Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{path}: {value}");
    assert_eq!(cache, "private, no-store");
    assert!(!value.to_string().contains("token"));
    value
}

#[tokio::test]
async fn viewer_auth_is_explicit_and_cannot_write() {
    let pool = setup().await;
    let (agent, token) = db::agents::create_agent(&pool, "alice").await.unwrap();
    let conversation = db::conversations::create_channel(&pool, "general", None, agent.id)
        .await
        .unwrap();
    for oauth_enabled in [false, true] {
        let mut conf = config();
        if oauth_enabled {
            conf.oauth = Some(backchannel_core::oauth::OAuthConfig::from_json(&json!({"issuer":"https://issuer.example/", "jwks_uri":"https://issuer.example/jwks", "resource":"https://api.example/api/mcp", "bindings":[{"subject":"alice", "client_id":"client", "agent_id":agent.id}]}).to_string()).unwrap());
        }
        let router = create_router(pool.clone(), conf);
        for path in [
            "/v1/admin/agents",
            "/v1/admin/conversations",
            "/v1/admin/messages",
            "/v1/admin/export",
            "/v1/admin/search?search=test",
            &format!("/v1/admin/conversations/{}/messages", conversation.id),
        ] {
            let (status, _, cache) =
                request(&router, "GET", path, Some("viewer-fixture"), Value::Null).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            assert_eq!(cache, "private, no-store");
            for invalid in [None, Some("wrong"), Some(token.as_str())] {
                let (status, _, cache) = request(&router, "GET", path, invalid, Value::Null).await;
                assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}");
                assert_eq!(cache, "private, no-store");
            }
        }
        for (method, path) in [
            ("POST", "/v1/admin/agents".into()),
            (
                "POST",
                format!("/v1/admin/agents/{}/rotate-token", agent.id),
            ),
            ("PATCH", format!("/v1/admin/agents/{}", agent.id)),
            ("POST", "/v1/channels".into()),
            ("POST", "/v1/dms".into()),
            (
                "POST",
                format!("/v1/conversations/{}/messages", conversation.id),
            ),
            (
                "GET",
                format!("/v1/conversations/{}/messages", conversation.id),
            ),
            ("GET", "/v1/me".into()),
            ("GET", "/v1/agents".into()),
            ("GET", "/v1/channels".into()),
            ("GET", "/v1/dms".into()),
            ("GET", "/v1/feed".into()),
            ("GET", "/api/mcp".into()),
            ("POST", "/api/mcp".into()),
            ("DELETE", "/api/mcp".into()),
            ("HEAD", "/v1/admin/agents".into()),
        ] {
            let (status, _, _) = request(
                &router,
                method,
                &path,
                Some("viewer-fixture"),
                json!({"name":"evil", "enabled":false,"body":"evil","client_message_id":"evil"}),
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}");
        }
        assert_eq!(
            request(&router, "GET", "/v1/me", Some(&token), Value::Null)
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(
            request(
                &router,
                "GET",
                "/v1/admin/messages",
                Some("admin-fixture"),
                Value::Null
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM agents")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    let admin_router = create_router(pool.clone(), config());
    assert_eq!(
        request(
            &admin_router,
            "HEAD",
            "/v1/admin/messages",
            Some("admin-fixture"),
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    for path in [
        "/v1/admin/messages?limit=0",
        "/v1/admin/export?limit=101",
        "/v1/admin/messages?after=-1",
        "/v1/admin/messages?search=",
    ] {
        assert_eq!(
            request(
                &admin_router,
                "GET",
                path,
                Some("viewer-fixture"),
                Value::Null
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    // Even an accidental database credential collision cannot promote the viewer.
    sqlx::query("UPDATE agents SET token_hash=$1")
        .bind(hash_token("viewer-fixture"))
        .execute(&pool)
        .await
        .unwrap();
    let router = create_router(pool.clone(), config());
    assert_eq!(
        request(
            &router,
            "GET",
            "/v1/me",
            Some("viewer-fixture"),
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    for oauth_enabled in [false, true] {
        let mut conf = config();
        if oauth_enabled {
            conf.oauth = Some(backchannel_core::oauth::OAuthConfig::from_json(&json!({"issuer":"https://issuer.example/", "jwks_uri":"https://issuer.example/jwks", "resource":"https://api.example/api/mcp", "bindings":[{"subject":"alice", "client_id":"client", "agent_id":agent.id}]}).to_string()).unwrap());
        }
        let router = create_router(pool.clone(), conf);
        for path in ["/v1/me", "/api/mcp"] {
            assert_eq!(
                request(&router, "GET", path, Some("viewer-fixture"), Value::Null)
                    .await
                    .0,
                StatusCode::UNAUTHORIZED
            );
        }
    }
    let mut disabled = config();
    disabled.viewer_token_sha256 = None;
    let router = create_router(pool, disabled);
    assert_eq!(
        request(
            &router,
            "GET",
            "/v1/admin/agents",
            Some("viewer-fixture"),
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn viewer_pagination_metadata_search_and_validation() {
    let pool = setup().await;
    let router = create_router(pool.clone(), config());
    assert_eq!(
        get(&router, "/v1/admin/conversations").await["items"],
        json!([])
    );
    assert_eq!(get(&router, "/v1/admin/agents").await["items"], json!([]));
    let (alice, _) = db::agents::create_agent(&pool, "alice").await.unwrap();
    let (bob, _) = db::agents::create_agent(&pool, "bob").await.unwrap();
    let dm = db::conversations::create_or_get_dm(&pool, alice.id, bob.id)
        .await
        .unwrap();
    let channel = db::conversations::create_channel(&pool, "general", None, alice.id)
        .await
        .unwrap();
    for i in 1..=5 {
        db::messages::create_message(
            &pool,
            dm.id,
            alice.id,
            &format!("100% message {i} <script>alert(1)</script>"),
            &format!("fixture-{i}"),
            None,
        )
        .await
        .unwrap();
    }
    let conversations = get(&router, "/v1/admin/conversations").await;
    let dm_json = conversations["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == dm.id.to_string())
        .unwrap();
    assert_eq!(dm_json["message_count"], 5);
    assert!(dm_json["last_activity_at"].is_string());
    assert_eq!(dm_json["members"][0]["name"], "alice");
    assert_eq!(dm_json["members"][1]["name"], "bob");
    let ch_json = conversations["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == channel.id.to_string())
        .unwrap();
    assert_eq!(ch_json["message_count"], 0);
    assert!(ch_json["last_activity_at"].is_null());
    for endpoint in ["agents", "conversations"] {
        let first = get(&router, &format!("/v1/admin/{endpoint}?limit=1")).await;
        assert_eq!(first["has_more"], true);
        let second = get(
            &router,
            &format!(
                "/v1/admin/{endpoint}?limit=1&before={}",
                first["next_cursor"].as_str().unwrap()
            ),
        )
        .await;
        assert_ne!(first["items"][0]["id"], second["items"][0]["id"]);
        assert_eq!(second["has_more"], false);
    }
    let path = format!("/v1/admin/conversations/{}/messages", dm.id);
    let first = get(&router, &format!("{path}?limit=2")).await;
    assert_eq!(first["items"][0]["sender_name"], "alice");
    assert!(first["items"][0]["body"]
        .as_str()
        .unwrap()
        .contains("<script>"));
    // Insertion after the first page cannot disturb the older-page boundary.
    db::messages::create_message(&pool, dm.id, bob.id, "new", "new", None)
        .await
        .unwrap();
    let second = get(
        &router,
        &format!(
            "{path}?limit=2&before={}",
            first["next_cursor"].as_str().unwrap()
        ),
    )
    .await;
    let third = get(
        &router,
        &format!(
            "{path}?limit=2&before={}",
            second["next_cursor"].as_str().unwrap()
        ),
    )
    .await;
    let ids: Vec<i64> = [&first, &second, &third]
        .iter()
        .flat_map(|p| p["items"].as_array().unwrap())
        .map(|m| m["id"].as_str().unwrap().parse().unwrap())
        .collect();
    assert_eq!(ids.len(), 5);
    assert!(ids.windows(2).all(|w| w[0] > w[1]));
    assert_eq!(third["has_more"], false);
    assert!(third["next_cursor"].is_null());
    assert_eq!(
        get(&router, "/v1/admin/search?search=100%25&limit=100").await["items"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert_eq!(
        get(&router, "/v1/admin/search?search=%27%20OR%201%3D1--").await["items"],
        json!([])
    );
    for query in [
        "limit=0",
        "limit=101",
        "limit=-1",
        "before=m1:0",
        "before=m1:-1",
        "before=wrong",
        "before=m1:9223372036854775808",
        "search=",
        "unknown=foo",
    ] {
        assert_eq!(
            request(
                &router,
                "GET",
                &format!("{path}?{query}"),
                Some("viewer-fixture"),
                Value::Null
            )
            .await
            .0,
            StatusCode::BAD_REQUEST,
            "{query}"
        );
    }
    assert_eq!(
        request(
            &router,
            "GET",
            "/v1/admin/search",
            Some("viewer-fixture"),
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &router,
            "GET",
            "/v1/admin/conversations/not-uuid/messages",
            Some("viewer-fixture"),
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &router,
            "GET",
            &format!("/v1/admin/conversations/{}/messages", Uuid::new_v4()),
            Some("viewer-fixture"),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn viewer_rate_limit_is_separate_and_no_store() {
    let pool = setup().await;
    let mut conf = config();
    conf.admin_rate_limit_per_minute = 1;
    let router = create_router(pool, conf);
    get(&router, "/v1/admin/agents").await;
    let (status, _, cache) = request(
        &router,
        "GET",
        "/v1/admin/agents",
        Some("viewer-fixture"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(cache, "private, no-store");
    assert_eq!(
        request(
            &router,
            "GET",
            "/v1/admin/agents",
            Some("admin-fixture"),
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
}
