use super::*;
use serde_json::json;
use serial_test::serial;

const TEST_APP: &str = "test-webhook-svc";

fn test_db_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db".to_string()
    })
}

async fn test_pool() -> PgPool {
    let pool = sqlx::PgPool::connect(&test_db_url())
        .await
        .expect("Failed to connect to integrations test database");
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Migrations failed");
    pool
}

async fn cleanup(pool: &PgPool) {
    sqlx::query("DELETE FROM integrations_outbox WHERE app_id = $1")
        .bind(TEST_APP)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_webhook_ingest WHERE app_id = $1")
        .bind(TEST_APP)
        .execute(pool)
        .await
        .ok();
}

fn internal_req(idempotency_key: Option<&str>, event_type: Option<&str>) -> IngestWebhookRequest {
    IngestWebhookRequest {
        app_id: TEST_APP.to_string(),
        system: "internal".to_string(),
        event_type: event_type.map(str::to_string),
        idempotency_key: idempotency_key.map(str::to_string),
        raw_payload: json!({ "data": "test" }),
        headers: std::collections::HashMap::new(),
    }
}

/// Webhook endpoint persists raw payload and metadata.
#[tokio::test]
#[serial]
async fn test_webhook_ingest_persists_payload() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let svc = WebhookService::new(pool.clone());
    let body = br#"{"data":"test"}"#;
    let req = internal_req(Some("evt-persist-001"), Some("my.custom.event"));
    let result = svc.ingest(req, body).await.expect("ingest failed");

    assert!(!result.is_duplicate);
    assert!(result.ingest_id > 0);

    // Verify row was written
    let row: Option<(String, Option<String>, bool)> = sqlx::query_as(
        "SELECT system, event_type, processed_at IS NOT NULL
         FROM integrations_webhook_ingest WHERE id = $1",
    )
    .bind(result.ingest_id)
    .fetch_optional(&pool)
    .await
    .expect("query failed");

    let (system, event_type, is_processed) = row.expect("row should exist");
    assert_eq!(system, "internal");
    assert_eq!(event_type.as_deref(), Some("my.custom.event"));
    assert!(is_processed);

    cleanup(&pool).await;
}

/// Idempotency prevents replay double-processing.
#[tokio::test]
#[serial]
async fn test_webhook_idempotency_prevents_duplicate() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let svc = WebhookService::new(pool.clone());
    let body = b"{}";

    let req1 = internal_req(Some("evt-dedup-001"), None);
    let req2 = internal_req(Some("evt-dedup-001"), None);

    let r1 = svc.ingest(req1, body).await.expect("first ingest failed");
    assert!(!r1.is_duplicate);

    let r2 = svc.ingest(req2, body).await.expect("second ingest failed");
    assert!(r2.is_duplicate);
    assert_eq!(r1.ingest_id, r2.ingest_id);

    // Only one row in DB
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_webhook_ingest
         WHERE app_id = $1 AND idempotency_key = 'evt-dedup-001'",
    )
    .bind(TEST_APP)
    .fetch_one(&pool)
    .await
    .expect("count query failed");
    assert_eq!(count.0, 1);

    cleanup(&pool).await;
}

/// Routed domain event emitted via outbox (EventEnvelope compliant).
#[tokio::test]
#[serial]
async fn test_webhook_routed_event_in_outbox() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let svc = WebhookService::new(pool.clone());
    let body = b"{}";
    let req = internal_req(Some("evt-route-001"), Some("my.custom.event"));
    let result = svc.ingest(req, body).await.expect("ingest failed");

    assert!(!result.is_duplicate);

    // Check outbox has at least one event for this ingest
    let outbox_rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT event_type, aggregate_id FROM integrations_outbox
         WHERE app_id = $1 AND aggregate_type = 'webhook'",
    )
    .bind(TEST_APP)
    .fetch_all(&pool)
    .await
    .expect("outbox query failed");

    assert!(!outbox_rows.is_empty(), "outbox should contain events");
    let event_types: Vec<&str> = outbox_rows.iter().map(|(et, _)| et.as_str()).collect();
    assert!(
        event_types.contains(&"webhook.received"),
        "webhook.received must be in outbox"
    );

    cleanup(&pool).await;
}

/// Signature verification rejects unknown systems.
#[tokio::test]
#[serial]
async fn test_webhook_unsupported_system_rejected() {
    let pool = test_pool().await;
    let svc = WebhookService::new(pool.clone());

    let req = IngestWebhookRequest {
        app_id: TEST_APP.to_string(),
        system: "unknown-system".to_string(),
        event_type: None,
        idempotency_key: None,
        raw_payload: json!({}),
        headers: std::collections::HashMap::new(),
    };

    let result = svc.ingest(req, b"{}").await;
    assert!(matches!(
        result,
        Err(WebhookError::UnsupportedSystem { .. })
    ));
}
