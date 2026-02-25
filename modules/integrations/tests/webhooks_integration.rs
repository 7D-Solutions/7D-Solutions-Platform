//! Integrated tests for webhook ingest, signature verification, idempotency,
//! and outbox routing (bd-68cs).
//!
//! Covers:
//!  1.  Ingest via internal system — happy path
//!  2.  Idempotency — duplicate key returns is_duplicate=true
//!  3.  Idempotency — distinct keys produce separate rows
//!  4.  Idempotency — no key allows duplicate inserts
//!  5.  Outbox receives webhook.received event on ingest
//!  6.  Outbox receives webhook.routed event when mapped
//!  7.  Unknown event type is NOT routed (no webhook.routed)
//!  8.  Unsupported system is rejected
//!  9.  Stripe signature verification — valid HMAC
//! 10.  Stripe signature verification — bad HMAC rejected
//! 11.  Stripe signature verification — missing header rejected
//! 12.  Stripe signature verification — expired timestamp rejected
//! 13.  GitHub signature verification — valid HMAC
//! 14.  GitHub signature verification — bad HMAC rejected
//! 15.  Routing table: stripe payment_intent.succeeded → payment.received
//! 16.  Routing table: internal passthrough
//! 17.  Processed_at is set after successful ingest
//! 18.  Tenant isolation — different app_ids cannot see each other's ingests

use hmac::{Hmac, Mac};
use integrations_rs::domain::webhooks::{
    IngestWebhookRequest, WebhookError, WebhookService,
};
use serial_test::serial;
use sha2::Sha256;
use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

// ============================================================================
// Test helpers
// ============================================================================

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://integrations_user:integrations_pass@localhost:5449/integrations_db"
            .to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to integrations test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run integrations migrations");
    pool
}

fn unique_app() -> String {
    format!("wh-test-{}", Uuid::new_v4().simple())
}

fn hmac_sha256_hex(secret: &str, message: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(message);
    let result = mac.finalize().into_bytes();
    result.iter().map(|b| format!("{:02x}", b)).collect()
}

fn internal_req(
    app_id: &str,
    idempotency_key: Option<&str>,
    event_type: Option<&str>,
) -> IngestWebhookRequest {
    IngestWebhookRequest {
        app_id: app_id.to_string(),
        system: "internal".to_string(),
        event_type: event_type.map(str::to_string),
        idempotency_key: idempotency_key.map(str::to_string),
        raw_payload: serde_json::json!({ "data": "test" }),
        headers: HashMap::new(),
    }
}

async fn cleanup(pool: &sqlx::PgPool, app_id: &str) {
    sqlx::query("DELETE FROM integrations_outbox WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_webhook_ingest WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// 1. Ingest via internal system — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_ingest_internal_happy_path() {
    let pool = setup_db().await;
    let app = unique_app();

    let svc = WebhookService::new(pool.clone());
    let req = internal_req(&app, Some("evt-hp-001"), Some("order.created"));
    let result = svc.ingest(req, b"{}").await.expect("ingest failed");

    assert!(!result.is_duplicate);
    assert!(result.ingest_id > 0);

    // Verify the row exists
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT system, event_type FROM integrations_webhook_ingest WHERE id = $1",
    )
    .bind(result.ingest_id)
    .fetch_optional(&pool)
    .await
    .expect("query failed");

    let (system, event_type) = row.expect("row should exist");
    assert_eq!(system, "internal");
    assert_eq!(event_type.as_deref(), Some("order.created"));

    cleanup(&pool, &app).await;
}

// ============================================================================
// 2. Idempotency — duplicate key returns is_duplicate=true
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_idempotency_duplicate_key() {
    let pool = setup_db().await;
    let app = unique_app();

    let svc = WebhookService::new(pool.clone());

    let r1 = internal_req(&app, Some("evt-dup-001"), None);
    let r2 = internal_req(&app, Some("evt-dup-001"), None);

    let first = svc.ingest(r1, b"{}").await.expect("first ingest");
    assert!(!first.is_duplicate);

    let second = svc.ingest(r2, b"{}").await.expect("second ingest");
    assert!(second.is_duplicate);
    assert_eq!(first.ingest_id, second.ingest_id);

    // Only one row in DB
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_webhook_ingest
         WHERE app_id = $1 AND idempotency_key = 'evt-dup-001'",
    )
    .bind(&app)
    .fetch_one(&pool)
    .await
    .expect("count query failed");
    assert_eq!(count.0, 1);

    cleanup(&pool, &app).await;
}

// ============================================================================
// 3. Idempotency — distinct keys produce separate rows
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_distinct_keys_separate_rows() {
    let pool = setup_db().await;
    let app = unique_app();

    let svc = WebhookService::new(pool.clone());

    let r1 = internal_req(&app, Some("evt-a"), None);
    let r2 = internal_req(&app, Some("evt-b"), None);

    let a = svc.ingest(r1, b"{}").await.expect("ingest a");
    let b = svc.ingest(r2, b"{}").await.expect("ingest b");

    assert!(!a.is_duplicate);
    assert!(!b.is_duplicate);
    assert_ne!(a.ingest_id, b.ingest_id);

    cleanup(&pool, &app).await;
}

// ============================================================================
// 4. Idempotency — no key allows duplicate inserts
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_no_idempotency_key_allows_duplicates() {
    let pool = setup_db().await;
    let app = unique_app();

    let svc = WebhookService::new(pool.clone());

    let r1 = internal_req(&app, None, None);
    let r2 = internal_req(&app, None, None);

    let a = svc.ingest(r1, b"{}").await.expect("ingest a");
    let b = svc.ingest(r2, b"{}").await.expect("ingest b");

    // Without idempotency key, both should succeed as new rows
    assert!(!a.is_duplicate);
    assert!(!b.is_duplicate);

    cleanup(&pool, &app).await;
}

// ============================================================================
// 5. Outbox receives webhook.received event on ingest
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_outbox_received_event() {
    let pool = setup_db().await;
    let app = unique_app();

    let svc = WebhookService::new(pool.clone());
    let req = internal_req(&app, Some("evt-outbox-recv"), Some("ping"));
    svc.ingest(req, b"{}").await.expect("ingest failed");

    let events: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM integrations_outbox
         WHERE app_id = $1 AND aggregate_type = 'webhook'
         ORDER BY created_at",
    )
    .bind(&app)
    .fetch_all(&pool)
    .await
    .expect("outbox query failed");

    let types: Vec<&str> = events.iter().map(|(t,)| t.as_str()).collect();
    assert!(
        types.contains(&"webhook.received"),
        "expected webhook.received in outbox, got: {:?}",
        types
    );

    cleanup(&pool, &app).await;
}

// ============================================================================
// 6. Outbox receives webhook.routed event when mapped
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_outbox_routed_event() {
    let pool = setup_db().await;
    let app = unique_app();

    let svc = WebhookService::new(pool.clone());
    // "internal" + event_type passes through routing as-is
    let req = internal_req(&app, Some("evt-routed-001"), Some("my.domain.event"));
    svc.ingest(req, b"{}").await.expect("ingest failed");

    let events: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM integrations_outbox
         WHERE app_id = $1 AND aggregate_type = 'webhook'
         ORDER BY created_at",
    )
    .bind(&app)
    .fetch_all(&pool)
    .await
    .expect("outbox query failed");

    let types: Vec<&str> = events.iter().map(|(t,)| t.as_str()).collect();
    assert!(types.contains(&"webhook.received"), "missing webhook.received");
    assert!(types.contains(&"webhook.routed"), "missing webhook.routed");

    cleanup(&pool, &app).await;
}

// ============================================================================
// 7. Unknown event type is NOT routed
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_unknown_event_not_routed() {
    let pool = setup_db().await;
    let app = unique_app();

    let svc = WebhookService::new(pool.clone());
    // internal + None event type → routing returns None
    let req = internal_req(&app, Some("evt-noroute-001"), None);
    svc.ingest(req, b"{}").await.expect("ingest failed");

    let events: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM integrations_outbox
         WHERE app_id = $1 AND aggregate_type = 'webhook'",
    )
    .bind(&app)
    .fetch_all(&pool)
    .await
    .expect("outbox query failed");

    let types: Vec<&str> = events.iter().map(|(t,)| t.as_str()).collect();
    assert!(types.contains(&"webhook.received"), "missing webhook.received");
    assert!(
        !types.contains(&"webhook.routed"),
        "webhook.routed should NOT be emitted for unmapped events"
    );

    cleanup(&pool, &app).await;
}

// ============================================================================
// 8. Unsupported system is rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_unsupported_system() {
    let pool = setup_db().await;
    let app = unique_app();

    let svc = WebhookService::new(pool.clone());
    let req = IngestWebhookRequest {
        app_id: app.clone(),
        system: "unknown-system".to_string(),
        event_type: None,
        idempotency_key: None,
        raw_payload: serde_json::json!({}),
        headers: HashMap::new(),
    };

    let err = svc.ingest(req, b"{}").await;
    assert!(matches!(err, Err(WebhookError::UnsupportedSystem { .. })));

    cleanup(&pool, &app).await;
}

// ============================================================================
// 9. Stripe signature verification — valid HMAC
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_stripe_valid_signature() {
    let pool = setup_db().await;
    let app = unique_app();

    let secret = "whsec_test_stripe_integration";
    std::env::set_var("STRIPE_WEBHOOK_SECRET", secret);

    let body = br#"{"id":"evt_stripe_001","type":"payment_intent.succeeded"}"#;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let timestamp = now.to_string();
    let signed_payload = format!("{}.{}", timestamp, std::str::from_utf8(body).unwrap());
    let sig = hmac_sha256_hex(secret, signed_payload.as_bytes());

    let mut headers = HashMap::new();
    headers.insert(
        "stripe-signature".to_string(),
        format!("t={},v1={}", timestamp, sig),
    );

    let req = IngestWebhookRequest {
        app_id: app.clone(),
        system: "stripe".to_string(),
        event_type: Some("payment_intent.succeeded".to_string()),
        idempotency_key: Some("evt_stripe_001".to_string()),
        raw_payload: serde_json::from_slice(body).unwrap(),
        headers,
    };

    let svc = WebhookService::new(pool.clone());
    let result = svc.ingest(req, body).await.expect("stripe ingest failed");
    assert!(!result.is_duplicate);
    assert!(result.ingest_id > 0);

    cleanup(&pool, &app).await;
    std::env::remove_var("STRIPE_WEBHOOK_SECRET");
}

// ============================================================================
// 10. Stripe signature verification — bad HMAC rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_stripe_bad_signature() {
    let pool = setup_db().await;
    let app = unique_app();

    std::env::set_var("STRIPE_WEBHOOK_SECRET", "whsec_correct");

    let body = b"{}";
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut headers = HashMap::new();
    headers.insert(
        "stripe-signature".to_string(),
        format!(
            "t={},v1={}",
            now,
            "deadbeef00000000000000000000000000000000000000000000000000000000"
        ),
    );

    let req = IngestWebhookRequest {
        app_id: app.clone(),
        system: "stripe".to_string(),
        event_type: None,
        idempotency_key: None,
        raw_payload: serde_json::json!({}),
        headers,
    };

    let svc = WebhookService::new(pool.clone());
    let err = svc.ingest(req, body).await;
    assert!(
        matches!(err, Err(WebhookError::SignatureVerification(_))),
        "expected SignatureVerification error, got: {:?}",
        err
    );

    cleanup(&pool, &app).await;
    std::env::remove_var("STRIPE_WEBHOOK_SECRET");
}

// ============================================================================
// 11. Stripe signature verification — missing header rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_stripe_missing_header() {
    let pool = setup_db().await;
    let app = unique_app();

    std::env::set_var("STRIPE_WEBHOOK_SECRET", "whsec_test");

    let req = IngestWebhookRequest {
        app_id: app.clone(),
        system: "stripe".to_string(),
        event_type: None,
        idempotency_key: None,
        raw_payload: serde_json::json!({}),
        headers: HashMap::new(), // no stripe-signature header
    };

    let svc = WebhookService::new(pool.clone());
    let err = svc.ingest(req, b"{}").await;
    assert!(
        matches!(err, Err(WebhookError::SignatureVerification(_))),
        "expected SignatureVerification for missing header, got: {:?}",
        err
    );

    cleanup(&pool, &app).await;
    std::env::remove_var("STRIPE_WEBHOOK_SECRET");
}

// ============================================================================
// 12. Stripe signature verification — expired timestamp rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_stripe_expired_timestamp() {
    let pool = setup_db().await;
    let app = unique_app();

    let secret = "whsec_expired_test";
    std::env::set_var("STRIPE_WEBHOOK_SECRET", secret);

    let body = b"{}";
    let old_timestamp = "1000000"; // long expired
    let signed_payload = format!("{}.{}", old_timestamp, "{}");
    let sig = hmac_sha256_hex(secret, signed_payload.as_bytes());

    let mut headers = HashMap::new();
    headers.insert(
        "stripe-signature".to_string(),
        format!("t={},v1={}", old_timestamp, sig),
    );

    let req = IngestWebhookRequest {
        app_id: app.clone(),
        system: "stripe".to_string(),
        event_type: None,
        idempotency_key: None,
        raw_payload: serde_json::json!({}),
        headers,
    };

    let svc = WebhookService::new(pool.clone());
    let err = svc.ingest(req, body).await;
    assert!(
        matches!(err, Err(WebhookError::SignatureVerification(_))),
        "expected SignatureVerification for expired timestamp, got: {:?}",
        err
    );

    cleanup(&pool, &app).await;
    std::env::remove_var("STRIPE_WEBHOOK_SECRET");
}

// ============================================================================
// 13. GitHub signature verification — valid HMAC
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_github_valid_signature() {
    let pool = setup_db().await;
    let app = unique_app();

    let secret = "github_webhook_secret_test";
    std::env::set_var("GITHUB_WEBHOOK_SECRET", secret);

    let body = br#"{"action":"opened","pull_request":{}}"#;
    let sig = hmac_sha256_hex(secret, body);

    let mut headers = HashMap::new();
    headers.insert(
        "x-hub-signature-256".to_string(),
        format!("sha256={}", sig),
    );

    let req = IngestWebhookRequest {
        app_id: app.clone(),
        system: "github".to_string(),
        event_type: Some("pull_request".to_string()),
        idempotency_key: Some("gh-delivery-001".to_string()),
        raw_payload: serde_json::from_slice(body).unwrap(),
        headers,
    };

    let svc = WebhookService::new(pool.clone());
    let result = svc.ingest(req, body).await.expect("github ingest failed");
    assert!(!result.is_duplicate);

    cleanup(&pool, &app).await;
    std::env::remove_var("GITHUB_WEBHOOK_SECRET");
}

// ============================================================================
// 14. GitHub signature verification — bad HMAC rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_github_bad_signature() {
    let pool = setup_db().await;
    let app = unique_app();

    std::env::set_var("GITHUB_WEBHOOK_SECRET", "real_secret");

    let body = b"{}";
    let mut headers = HashMap::new();
    headers.insert(
        "x-hub-signature-256".to_string(),
        "sha256=0000000000000000000000000000000000000000000000000000000000000000"
            .to_string(),
    );

    let req = IngestWebhookRequest {
        app_id: app.clone(),
        system: "github".to_string(),
        event_type: None,
        idempotency_key: None,
        raw_payload: serde_json::json!({}),
        headers,
    };

    let svc = WebhookService::new(pool.clone());
    let err = svc.ingest(req, body).await;
    assert!(
        matches!(err, Err(WebhookError::SignatureVerification(_))),
        "expected SignatureVerification for bad github HMAC, got: {:?}",
        err
    );

    cleanup(&pool, &app).await;
    std::env::remove_var("GITHUB_WEBHOOK_SECRET");
}

// ============================================================================
// 15. Routing: stripe payment_intent.succeeded → payment.received
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_routing_stripe_payment_received() {
    let pool = setup_db().await;
    let app = unique_app();

    // Use internal system so we skip real Stripe signature verification
    // but test the routing logic with a stripe-like event_type mapping
    // via "internal" passthrough which routes the event_type as-is.
    // Instead, directly test the routing function.
    use integrations_rs::domain::webhooks::routing::map_to_domain_event;

    let result = map_to_domain_event("stripe", Some("payment_intent.succeeded"));
    assert_eq!(result, Some("payment.received".to_string()));

    let result = map_to_domain_event("stripe", Some("payment_intent.payment_failed"));
    assert_eq!(result, Some("payment.failed".to_string()));

    let result = map_to_domain_event("stripe", Some("invoice.payment_succeeded"));
    assert_eq!(result, Some("invoice.paid.external".to_string()));

    let result = map_to_domain_event("stripe", Some("customer.subscription.created"));
    assert_eq!(result, Some("subscription.created.external".to_string()));

    let result = map_to_domain_event("stripe", Some("customer.subscription.deleted"));
    assert_eq!(result, Some("subscription.cancelled.external".to_string()));

    cleanup(&pool, &app).await;
}

// ============================================================================
// 16. Routing: internal passthrough
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_routing_internal_passthrough() {
    use integrations_rs::domain::webhooks::routing::map_to_domain_event;

    let result = map_to_domain_event("internal", Some("custom.event.type"));
    assert_eq!(result, Some("custom.event.type".to_string()));

    // Internal with None event_type → no route
    let result = map_to_domain_event("internal", None);
    assert_eq!(result, None);
}

// ============================================================================
// 17. Processed_at is set after successful ingest
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_processed_at_set() {
    let pool = setup_db().await;
    let app = unique_app();

    let svc = WebhookService::new(pool.clone());
    let req = internal_req(&app, Some("evt-processed-001"), Some("test.event"));
    let result = svc.ingest(req, b"{}").await.expect("ingest failed");

    let processed: (bool,) = sqlx::query_as(
        "SELECT processed_at IS NOT NULL FROM integrations_webhook_ingest WHERE id = $1",
    )
    .bind(result.ingest_id)
    .fetch_one(&pool)
    .await
    .expect("query failed");

    assert!(processed.0, "processed_at should be set after successful ingest");

    cleanup(&pool, &app).await;
}

// ============================================================================
// 18. Tenant isolation — different app_ids cannot see each other
// ============================================================================

#[tokio::test]
#[serial]
async fn test_webhook_tenant_isolation() {
    let pool = setup_db().await;
    let app_a = unique_app();
    let app_b = unique_app();

    let svc = WebhookService::new(pool.clone());

    let req_a = internal_req(&app_a, Some("evt-iso-a"), None);
    let result_a = svc.ingest(req_a, b"{}").await.expect("ingest A failed");

    let req_b = internal_req(&app_b, Some("evt-iso-b"), None);
    let result_b = svc.ingest(req_b, b"{}").await.expect("ingest B failed");

    // App A's ingest is not visible under App B's app_id
    let cross_check: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM integrations_webhook_ingest WHERE id = $1 AND app_id = $2",
    )
    .bind(result_a.ingest_id)
    .bind(&app_b)
    .fetch_optional(&pool)
    .await
    .expect("cross-tenant query failed");
    assert!(cross_check.is_none(), "App B must not see App A's ingest");

    // App B's ingest is not visible under App A's app_id
    let cross_check_b: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM integrations_webhook_ingest WHERE id = $1 AND app_id = $2",
    )
    .bind(result_b.ingest_id)
    .bind(&app_a)
    .fetch_optional(&pool)
    .await
    .expect("cross-tenant query failed");
    assert!(
        cross_check_b.is_none(),
        "App A must not see App B's ingest"
    );

    // Outbox events are also scoped
    let a_outbox: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox WHERE app_id = $1",
    )
    .bind(&app_a)
    .fetch_one(&pool)
    .await
    .expect("outbox count failed");
    assert!(a_outbox.0 > 0, "App A should have outbox events");

    let b_outbox: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox WHERE app_id = $1",
    )
    .bind(&app_b)
    .fetch_one(&pool)
    .await
    .expect("outbox count failed");
    assert!(b_outbox.0 > 0, "App B should have outbox events");

    cleanup(&pool, &app_a).await;
    cleanup(&pool, &app_b).await;
}
