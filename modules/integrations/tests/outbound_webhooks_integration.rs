//! Integrated tests for outbound webhook management (bd-3jmnf).
//!
//! Covers:
//!  1.  Webhook CRUD E2E: create, update URL, delete, verify each state
//!  2.  Delivery log: record successful delivery, verify log persisted
//!  3.  Failed delivery: record failure, verify error + retry metadata
//!  4.  Tenant isolation: tenant_A webhook invisible to tenant_B
//!  5.  Idempotency: same idempotency_key → no duplicate
//!  6.  Outbox event: webhook creation emits outbound_webhook.created

use chrono::Utc;
use integrations_rs::domain::outbound_webhooks::{
    CreateOutboundWebhookRequest, OutboundWebhookService, RecordDeliveryRequest,
    UpdateOutboundWebhookRequest,
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

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

fn unique_tenant() -> String {
    format!("tenant-{}", Uuid::new_v4().simple())
}

async fn cleanup(pool: &sqlx::PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM integrations_outbound_webhook_deliveries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_outbox WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_outbound_webhooks WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// 1. Webhook CRUD E2E
// ============================================================================

#[tokio::test]
#[serial]
async fn test_outbound_webhook_crud_e2e() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    cleanup(&pool, &tenant).await;

    let svc = OutboundWebhookService::new(pool.clone());

    // ── Create ──────────────────────────────────────────────────────────
    let (webhook, raw_secret) = svc
        .create(CreateOutboundWebhookRequest {
            tenant_id: tenant.clone(),
            url: "https://example.com/hook".into(),
            event_types: vec!["order.created".into(), "order.shipped".into()],
            description: Some("Order events".into()),
            idempotency_key: None,
        })
        .await
        .expect("create failed");

    assert!(!raw_secret.is_empty(), "raw secret must be returned");
    assert!(raw_secret.starts_with("whsec_"), "secret must have prefix");
    assert_eq!(webhook.tenant_id, tenant);
    assert_eq!(webhook.url, "https://example.com/hook");
    assert_eq!(webhook.status, "active");

    // ── Read ────────────────────────────────────────────────────────────
    let fetched = svc
        .get(&tenant, webhook.id)
        .await
        .expect("get failed")
        .expect("webhook should exist");
    assert_eq!(fetched.id, webhook.id);
    assert_eq!(fetched.url, "https://example.com/hook");

    // ── Update URL ──────────────────────────────────────────────────────
    let updated = svc
        .update(UpdateOutboundWebhookRequest {
            id: webhook.id,
            tenant_id: tenant.clone(),
            url: Some("https://example.com/hook-v2".into()),
            event_types: None,
            status: None,
            description: None,
        })
        .await
        .expect("update failed");

    assert_eq!(updated.url, "https://example.com/hook-v2");
    assert_eq!(updated.status, "active");

    // ── Delete ──────────────────────────────────────────────────────────
    svc.delete(&tenant, webhook.id)
        .await
        .expect("delete failed");

    let gone = svc.get(&tenant, webhook.id).await.expect("get failed");
    assert!(gone.is_none(), "deleted webhook should not be returned");

    // ── List should be empty ────────────────────────────────────────────
    let list = svc.list(&tenant).await.expect("list failed");
    assert!(list.is_empty(), "no webhooks should remain after delete");

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 2. Delivery log — success
// ============================================================================

#[tokio::test]
#[serial]
async fn test_outbound_webhook_delivery_log_success() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    cleanup(&pool, &tenant).await;

    let svc = OutboundWebhookService::new(pool.clone());

    let (webhook, _) = svc
        .create(CreateOutboundWebhookRequest {
            tenant_id: tenant.clone(),
            url: "https://example.com/delivery-test".into(),
            event_types: vec!["invoice.paid".into()],
            description: None,
            idempotency_key: None,
        })
        .await
        .expect("create failed");

    let delivery = svc
        .record_delivery(RecordDeliveryRequest {
            webhook_id: webhook.id,
            tenant_id: tenant.clone(),
            event_type: "invoice.paid".into(),
            payload: serde_json::json!({"invoice_id": "inv-001"}),
            status_code: Some(200),
            response_body: Some("OK".into()),
            error_message: None,
            attempt_number: 1,
            next_retry_at: None,
            delivered_at: Some(Utc::now()),
        })
        .await
        .expect("record delivery failed");

    assert_eq!(delivery.status_code, Some(200));
    assert_eq!(delivery.response_body.as_deref(), Some("OK"));
    assert!(delivery.error_message.is_none());
    assert!(delivery.delivered_at.is_some());
    assert_eq!(delivery.attempt_number, 1);

    // Verify via list
    let deliveries = svc
        .list_deliveries(&tenant, webhook.id)
        .await
        .expect("list deliveries failed");
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].status_code, Some(200));

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 3. Failed delivery — error details and retry metadata
// ============================================================================

#[tokio::test]
#[serial]
async fn test_outbound_webhook_delivery_failure() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    cleanup(&pool, &tenant).await;

    let svc = OutboundWebhookService::new(pool.clone());

    let (webhook, _) = svc
        .create(CreateOutboundWebhookRequest {
            tenant_id: tenant.clone(),
            url: "https://example.com/fail-test".into(),
            event_types: vec!["shipment.failed".into()],
            description: None,
            idempotency_key: None,
        })
        .await
        .expect("create failed");

    let retry_at = Utc::now() + chrono::Duration::seconds(60);

    let delivery = svc
        .record_delivery(RecordDeliveryRequest {
            webhook_id: webhook.id,
            tenant_id: tenant.clone(),
            event_type: "shipment.failed".into(),
            payload: serde_json::json!({"shipment_id": "shp-999"}),
            status_code: Some(503),
            response_body: Some("Service Unavailable".into()),
            error_message: Some("Connection timeout after 30s".into()),
            attempt_number: 1,
            next_retry_at: Some(retry_at),
            delivered_at: None,
        })
        .await
        .expect("record delivery failed");

    assert_eq!(delivery.status_code, Some(503));
    assert_eq!(
        delivery.error_message.as_deref(),
        Some("Connection timeout after 30s")
    );
    assert_eq!(delivery.attempt_number, 1);
    assert!(delivery.next_retry_at.is_some(), "retry time must be set");
    assert!(
        delivery.delivered_at.is_none(),
        "delivered_at must be None on failure"
    );

    // Record a retry attempt
    let retry = svc
        .record_delivery(RecordDeliveryRequest {
            webhook_id: webhook.id,
            tenant_id: tenant.clone(),
            event_type: "shipment.failed".into(),
            payload: serde_json::json!({"shipment_id": "shp-999"}),
            status_code: Some(200),
            response_body: Some("OK".into()),
            error_message: None,
            attempt_number: 2,
            next_retry_at: None,
            delivered_at: Some(Utc::now()),
        })
        .await
        .expect("retry delivery failed");

    assert_eq!(retry.attempt_number, 2);
    assert!(retry.delivered_at.is_some());

    // Total deliveries should be 2
    let all = svc
        .list_deliveries(&tenant, webhook.id)
        .await
        .expect("list failed");
    assert_eq!(all.len(), 2);

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 4. Tenant isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_outbound_webhook_tenant_isolation() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();
    cleanup(&pool, &tenant_a).await;
    cleanup(&pool, &tenant_b).await;

    let svc = OutboundWebhookService::new(pool.clone());

    // Tenant A creates a webhook
    let (wh_a, _) = svc
        .create(CreateOutboundWebhookRequest {
            tenant_id: tenant_a.clone(),
            url: "https://example.com/tenant-a".into(),
            event_types: vec!["a.event".into()],
            description: None,
            idempotency_key: None,
        })
        .await
        .expect("create A failed");

    // Tenant B creates a webhook
    let (wh_b, _) = svc
        .create(CreateOutboundWebhookRequest {
            tenant_id: tenant_b.clone(),
            url: "https://example.com/tenant-b".into(),
            event_types: vec!["b.event".into()],
            description: None,
            idempotency_key: None,
        })
        .await
        .expect("create B failed");

    // Tenant B cannot see Tenant A's webhook
    let cross = svc.get(&tenant_b, wh_a.id).await.expect("get failed");
    assert!(cross.is_none(), "Tenant B must not see Tenant A's webhook");

    // Tenant A cannot see Tenant B's webhook
    let cross_b = svc.get(&tenant_a, wh_b.id).await.expect("get failed");
    assert!(cross_b.is_none(), "Tenant A must not see Tenant B's webhook");

    // Listing is scoped
    let list_a = svc.list(&tenant_a).await.expect("list A failed");
    assert_eq!(list_a.len(), 1);
    assert_eq!(list_a[0].id, wh_a.id);

    let list_b = svc.list(&tenant_b).await.expect("list B failed");
    assert_eq!(list_b.len(), 1);
    assert_eq!(list_b[0].id, wh_b.id);

    // Tenant B cannot delete Tenant A's webhook
    let del_result = svc.delete(&tenant_b, wh_a.id).await;
    assert!(
        del_result.is_err(),
        "Tenant B must not be able to delete Tenant A's webhook"
    );

    // Tenant B cannot update Tenant A's webhook
    let upd_result = svc
        .update(UpdateOutboundWebhookRequest {
            id: wh_a.id,
            tenant_id: tenant_b.clone(),
            url: Some("https://evil.com/steal".into()),
            event_types: None,
            status: None,
            description: None,
        })
        .await;
    assert!(
        upd_result.is_err(),
        "Tenant B must not be able to update Tenant A's webhook"
    );

    cleanup(&pool, &tenant_a).await;
    cleanup(&pool, &tenant_b).await;
}

// ============================================================================
// 5. Idempotency — same key, no duplicate
// ============================================================================

#[tokio::test]
#[serial]
async fn test_outbound_webhook_idempotency() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    cleanup(&pool, &tenant).await;

    let svc = OutboundWebhookService::new(pool.clone());
    let idem_key = format!("idem-{}", Uuid::new_v4().simple());

    // First creation
    let (wh1, secret1) = svc
        .create(CreateOutboundWebhookRequest {
            tenant_id: tenant.clone(),
            url: "https://example.com/idempotent".into(),
            event_types: vec!["order.created".into()],
            description: None,
            idempotency_key: Some(idem_key.clone()),
        })
        .await
        .expect("first create failed");

    assert!(!secret1.is_empty());

    // Second creation with same key — should return existing, no secret
    let (wh2, secret2) = svc
        .create(CreateOutboundWebhookRequest {
            tenant_id: tenant.clone(),
            url: "https://example.com/idempotent-different".into(),
            event_types: vec!["different.event".into()],
            description: None,
            idempotency_key: Some(idem_key.clone()),
        })
        .await
        .expect("second create failed");

    assert_eq!(wh1.id, wh2.id, "same webhook must be returned");
    assert!(
        secret2.is_empty(),
        "no secret on idempotent duplicate return"
    );
    // URL should be the original, not the second request's
    assert_eq!(wh2.url, "https://example.com/idempotent");

    // Only one webhook in list
    let list = svc.list(&tenant).await.expect("list failed");
    assert_eq!(list.len(), 1, "only one webhook should exist");

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 6. Outbox event — creation emits outbound_webhook.created
// ============================================================================

#[tokio::test]
#[serial]
async fn test_outbound_webhook_outbox_event() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    cleanup(&pool, &tenant).await;

    let svc = OutboundWebhookService::new(pool.clone());

    let (webhook, _) = svc
        .create(CreateOutboundWebhookRequest {
            tenant_id: tenant.clone(),
            url: "https://example.com/outbox-test".into(),
            event_types: vec!["test.event".into()],
            description: None,
            idempotency_key: None,
        })
        .await
        .expect("create failed");

    // Check outbox for the creation event
    let events: Vec<(String, String, String)> = sqlx::query_as(
        r#"SELECT event_type, aggregate_type, aggregate_id
           FROM integrations_outbox
           WHERE app_id = $1
           ORDER BY created_at"#,
    )
    .bind(&tenant)
    .fetch_all(&pool)
    .await
    .expect("outbox query failed");

    assert!(!events.is_empty(), "outbox should contain events");

    let has_created = events.iter().any(|(et, at, aid)| {
        et == "outbound_webhook.created"
            && at == "outbound_webhook"
            && aid == &webhook.id.to_string()
    });

    assert!(
        has_created,
        "outbox must contain outbound_webhook.created event with correct aggregate, got: {:?}",
        events
    );

    // Verify the event payload contains tenant_id
    let payload_row: Option<(serde_json::Value,)> = sqlx::query_as(
        r#"SELECT payload FROM integrations_outbox
           WHERE app_id = $1 AND event_type = 'outbound_webhook.created'"#,
    )
    .bind(&tenant)
    .fetch_optional(&pool)
    .await
    .expect("payload query failed");

    let (payload,) = payload_row.expect("outbox event should exist");
    let payload_str = payload.to_string();
    assert!(
        payload_str.contains(&tenant),
        "payload must contain tenant_id"
    );

    cleanup(&pool, &tenant).await;
}
