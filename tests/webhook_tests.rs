mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serial_test::serial;
use tower::ServiceExt;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use hex;

const APP_ID: &str = "test-app";
const TEST_WEBHOOK_SECRET: &str = "whsec_test_secret";

/// Set up test environment with webhook secret
fn setup_test_env() {
    // Override both possible webhook secret env vars to ensure tests use the test secret
    std::env::set_var("TILLED_WEBHOOK_SECRET", TEST_WEBHOOK_SECRET);
    std::env::set_var("TILLED_WEBHOOK_SECRET_TRASHTECH", TEST_WEBHOOK_SECRET);
}

/// Clean up all test webhooks before running tests
async fn cleanup_all_test_webhooks(pool: &sqlx::PgPool) {
    sqlx::query("DELETE FROM ar_webhooks WHERE app_id = $1")
        .bind(APP_ID)
        .execute(pool)
        .await
        .ok();
}

/// Generate HMAC signature for webhook payload (Tilled format).
fn generate_webhook_signature(payload: &str, timestamp: i64, secret: &str) -> String {
    let signed_payload = format!("{}.{}", timestamp, payload);
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(signed_payload.as_bytes());
    let result = mac.finalize();
    hex::encode(result.into_bytes())
}

/// TEST 1: Receive valid webhook with correct signature
#[tokio::test]
#[serial]
async fn test_receive_webhook_valid_signature() {
    setup_test_env();
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let event_id = format!("evt_{}", uuid::Uuid::new_v4());
    let timestamp = chrono::Utc::now().timestamp();
    let payload = serde_json::json!({
        "id": event_id,
        "type": "payment_intent.succeeded",
        "data": {
            "object": {
                "id": "pi_test123",
                "amount": 5000,
                "currency": "usd"
            }
        },
        "created_at": timestamp,
        "livemode": false
    });

    let payload_str = serde_json::to_string(&payload).unwrap();
    let signature = generate_webhook_signature(&payload_str, timestamp, TEST_WEBHOOK_SECRET);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/webhooks/tilled")
                .header("content-type", "application/json")
                .header("tilled-signature", format!("t={},v1={}", timestamp, signature))
                .body(Body::from(payload_str))
                .unwrap(),
        )
        .await
        .unwrap();

    // Assert 200 status (webhook accepted)
    assert_eq!(response.status(), StatusCode::OK);

    // Verify webhook was stored in database
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_webhooks WHERE event_id = $1"
    )
    .bind(&event_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1, "Webhook should be stored");

    // Cleanup
    sqlx::query("DELETE FROM ar_webhooks WHERE event_id = $1")
        .bind(&event_id)
        .execute(&pool)
        .await
        .ok();

    common::teardown_pool(pool).await;
}

/// TEST 2: Reject webhook with invalid signature
#[tokio::test]
#[serial]
async fn test_receive_webhook_invalid_signature() {
    setup_test_env();
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let event_id = format!("evt_{}", uuid::Uuid::new_v4());
    let timestamp = chrono::Utc::now().timestamp();
    let payload = serde_json::json!({
        "id": event_id,
        "type": "payment_intent.succeeded",
        "data": {"object": {"id": "pi_test123"}},
        "created_at": timestamp,
        "livemode": false
    });

    let payload_str = serde_json::to_string(&payload).unwrap();
    let invalid_signature = "invalid_signature_here";

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/webhooks/tilled")
                .header("content-type", "application/json")
                .header("tilled-signature", format!("t={},v1={}", timestamp, invalid_signature))
                .body(Body::from(payload_str))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should reject with 401 or 400
    assert!(
        response.status() == StatusCode::UNAUTHORIZED || response.status() == StatusCode::BAD_REQUEST,
        "Should reject webhook with invalid signature"
    );

    // Verify webhook was NOT stored
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_webhooks WHERE event_id = $1"
    )
    .bind(&event_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0, "Invalid webhook should not be stored");

    common::teardown_pool(pool).await;
}

/// TEST 3: Handle duplicate event_id (idempotency)
#[tokio::test]
#[serial]
async fn test_receive_webhook_duplicate_event_id() {
    setup_test_env();
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let event_id = format!("evt_{}", uuid::Uuid::new_v4());

    // Seed existing webhook with same event_id
    let webhook_id = common::seed_webhook(&pool, APP_ID, &event_id, "payment_intent.succeeded", "processed").await;

    let timestamp = chrono::Utc::now().timestamp();
    let payload = serde_json::json!({
        "id": event_id,
        "type": "payment_intent.succeeded",
        "data": {"object": {"id": "pi_test123"}},
        "created_at": timestamp,
        "livemode": false
    });

    let payload_str = serde_json::to_string(&payload).unwrap();
    let signature = generate_webhook_signature(&payload_str, timestamp, TEST_WEBHOOK_SECRET);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/webhooks/tilled")
                .header("content-type", "application/json")
                .header("tilled-signature", format!("t={},v1={}", timestamp, signature))
                .body(Body::from(payload_str))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return 200 (idempotent)
    assert_eq!(response.status(), StatusCode::OK);

    // Verify only 1 webhook exists
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_webhooks WHERE event_id = $1"
    )
    .bind(&event_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1, "Should not create duplicate webhook");

    common::cleanup_webhooks(&pool, &[webhook_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 4: List webhooks by event type
#[tokio::test]
#[serial]
async fn test_list_webhooks_by_event_type() {
    setup_test_env();
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Clean up any leftover test data
    cleanup_all_test_webhooks(&pool).await;

    // Seed webhooks with different event types
    let event_id_1 = format!("evt_{}", uuid::Uuid::new_v4());
    let event_id_2 = format!("evt_{}", uuid::Uuid::new_v4());
    let event_id_3 = format!("evt_{}", uuid::Uuid::new_v4());
    let webhook1_id = common::seed_webhook(&pool, APP_ID, &event_id_1, "payment_intent.succeeded", "processed").await;
    let webhook2_id = common::seed_webhook(&pool, APP_ID, &event_id_2, "payment_intent.failed", "processed").await;
    let webhook3_id = common::seed_webhook(&pool, APP_ID, &event_id_3, "payment_intent.succeeded", "received").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ar/webhooks?event_type=payment_intent.succeeded")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert!(json.is_array(), "Response should be array");
    assert_eq!(json.as_array().unwrap().len(), 2, "Should have 2 webhooks with event type");

    common::cleanup_webhooks(&pool, &[webhook1_id, webhook2_id, webhook3_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 5: List webhooks by status
#[tokio::test]
#[serial]
async fn test_list_webhooks_by_status() {
    setup_test_env();
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Clean up any leftover test data
    cleanup_all_test_webhooks(&pool).await;

    let event_id_1 = format!("evt_{}", uuid::Uuid::new_v4());
    let event_id_2 = format!("evt_{}", uuid::Uuid::new_v4());
    let event_id_3 = format!("evt_{}", uuid::Uuid::new_v4());
    let webhook1_id = common::seed_webhook(&pool, APP_ID, &event_id_1, "payment_intent.succeeded", "processed").await;
    let webhook2_id = common::seed_webhook(&pool, APP_ID, &event_id_2, "payment_intent.succeeded", "failed").await;
    let webhook3_id = common::seed_webhook(&pool, APP_ID, &event_id_3, "payment_intent.succeeded", "received").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ar/webhooks?status=failed")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert!(json.is_array(), "Response should be array");
    assert_eq!(json.as_array().unwrap().len(), 1, "Should have 1 failed webhook");
    assert_eq!(json[0]["id"], webhook2_id);

    common::cleanup_webhooks(&pool, &[webhook1_id, webhook2_id, webhook3_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 6: Get webhook by ID
#[tokio::test]
#[serial]
async fn test_get_webhook_success() {
    setup_test_env();
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let event_id = format!("evt_{}", uuid::Uuid::new_v4());
    let webhook_id = common::seed_webhook(&pool, APP_ID, &event_id, "payment_intent.succeeded", "processed").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/webhooks/{}", webhook_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert_eq!(json["id"], webhook_id);
    assert_eq!(json["event_id"], event_id.as_str());
    assert_eq!(json["event_type"], "payment_intent.succeeded");
    assert_eq!(json["status"], "processed");

    common::cleanup_webhooks(&pool, &[webhook_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 7: Replay failed webhook
#[tokio::test]
#[serial]
async fn test_replay_webhook_failed() {
    setup_test_env();
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let event_id = format!("evt_{}", uuid::Uuid::new_v4());
    let webhook_id = common::seed_webhook(&pool, APP_ID, &event_id, "payment_intent.succeeded", "failed").await;

    let body = serde_json::json!({
        "force": false
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/ar/webhooks/{}/replay", webhook_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should accept replay request
    let status = response.status();
    if status != StatusCode::OK {
        let body = common::body_json(response).await;
        eprintln!("Replay failed with {}: {:?}", status, body);
    }
    assert_eq!(status, StatusCode::OK);

    common::cleanup_webhooks(&pool, &[webhook_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 8: Replay processed webhook without force flag
#[tokio::test]
#[serial]
async fn test_replay_webhook_already_processed() {
    setup_test_env();
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let event_id = format!("evt_{}", uuid::Uuid::new_v4());
    let webhook_id = common::seed_webhook(&pool, APP_ID, &event_id, "payment_intent.succeeded", "processed").await;

    let body = serde_json::json!({
        "force": false
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/ar/webhooks/{}/replay", webhook_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should reject or warn (not allowed to replay processed webhook)
    assert!(
        response.status() == StatusCode::BAD_REQUEST || response.status() == StatusCode::CONFLICT,
        "Should not replay already processed webhook without force"
    );

    common::cleanup_webhooks(&pool, &[webhook_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 9: Replay processed webhook with force flag
#[tokio::test]
#[serial]
async fn test_replay_webhook_with_force() {
    setup_test_env();
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let event_id = format!("evt_{}", uuid::Uuid::new_v4());
    let webhook_id = common::seed_webhook(&pool, APP_ID, &event_id, "payment_intent.succeeded", "processed").await;

    let body = serde_json::json!({
        "force": true
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/ar/webhooks/{}/replay", webhook_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should accept replay with force
    assert_eq!(response.status(), StatusCode::OK);

    common::cleanup_webhooks(&pool, &[webhook_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 10: Process out-of-order webhook events
#[tokio::test]
#[serial]
async fn test_receive_webhooks_out_of_order() {
    setup_test_env();
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let event_id_1 = format!("evt_{}", uuid::Uuid::new_v4());
    let event_id_2 = format!("evt_{}", uuid::Uuid::new_v4());

    // Receive event 2 before event 1 (out of order)
    let timestamp = chrono::Utc::now().timestamp();

    // Event 2 (arrives first)
    let payload2 = serde_json::json!({
        "id": event_id_2,
        "type": "charge.succeeded",
        "data": {"object": {"id": "ch_test", "sequence": 2}},
        "created_at": timestamp + 10,
        "livemode": false
    });
    let payload2_str = serde_json::to_string(&payload2).unwrap();
    let signature2 = generate_webhook_signature(&payload2_str, timestamp + 10, TEST_WEBHOOK_SECRET);

    let response2 = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/webhooks/tilled")
                .header("content-type", "application/json")
                .header("tilled-signature", format!("t={},v1={}", timestamp + 10, signature2))
                .body(Body::from(payload2_str))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response2.status(), StatusCode::OK);

    // Event 1 (arrives second, out of order)
    let payload1 = serde_json::json!({
        "id": event_id_1,
        "type": "charge.created",
        "data": {"object": {"id": "ch_test", "sequence": 1}},
        "created_at": timestamp,
        "livemode": false
    });
    let payload1_str = serde_json::to_string(&payload1).unwrap();
    let signature1 = generate_webhook_signature(&payload1_str, timestamp, TEST_WEBHOOK_SECRET);

    let response1 = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/webhooks/tilled")
                .header("content-type", "application/json")
                .header("tilled-signature", format!("t={},v1={}", timestamp, signature1))
                .body(Body::from(payload1_str))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response1.status(), StatusCode::OK);

    // Verify both webhooks were stored
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_webhooks WHERE event_id IN ($1, $2)"
    )
    .bind(&event_id_1)
    .bind(&event_id_2)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 2, "Both webhooks should be stored");

    // Cleanup
    sqlx::query("DELETE FROM ar_webhooks WHERE event_id IN ($1, $2)")
        .bind(&event_id_1)
        .bind(&event_id_2)
        .execute(&pool)
        .await
        .ok();

    common::teardown_pool(pool).await;
}
