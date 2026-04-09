//! Cross-module integration tests: Shopify → QBO end-to-end event chain.
//!
//! ## Full event chain
//!
//! ```
//! Shopify webhook
//!   → HMAC verify + normalize (integrations)
//!   → integrations_file_jobs row (parser_type = "shopify_order")
//!   → integrations.order.ingested outbox event (line items embedded)
//!   → [AR module creates invoice from order.ingested — tested in ar module]
//!   → ar.events.ar.invoice_opened NATS message
//!   → QBO outbound consumer (integrations)
//!   → QBO API call → Invoice.Id returned
//!   → integrations_external_refs row (ar_invoice → qbo_invoice)
//!   → integrations.qbo.invoice_created outbox event
//! ```
//!
//! These tests exercise both legs of the chain in isolation. The middle step
//! (AR creating an invoice from order.ingested) is handled by the AR module
//! and is not reproduced here.
//!
//! ## What is tested
//! 1. Shopify order webhook → file_job + order.ingested outbox event
//! 2. ar.invoice_opened → QBO outbound → external_ref + qbo.invoice_created event
//!
//! Run: ./scripts/cargo-slot.sh test -p integrations-rs --test cross_module_flow

use base64::{engine::general_purpose::STANDARD, Engine as _};
use event_bus::BusMessage;
use hmac::{Hmac, Mac};
use integrations_rs::domain::qbo::outbound::{
    process_ar_invoice_opened, EVENT_TYPE_QBO_INVOICE_CREATED,
};
use integrations_rs::domain::webhooks::ShopifyNormalizer;
use integrations_rs::events::EVENT_TYPE_ORDER_INGESTED;
use serial_test::serial;
use sha2::Sha256;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

// ============================================================================
// Shared test infrastructure
// ============================================================================

async fn setup_db() -> PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://integrations_user:integrations_pass@localhost:5449/integrations_db"
            .to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect to integrations test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool
}

fn unique_tenant() -> String {
    format!("xmod-{}", Uuid::new_v4().simple())
}

async fn cleanup(pool: &PgPool, app_id: &str) {
    sqlx::query("DELETE FROM integrations_outbox WHERE app_id = $1")
        .bind(app_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM integrations_file_jobs WHERE tenant_id = $1")
        .bind(app_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM integrations_webhook_ingest WHERE app_id = $1")
        .bind(app_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM integrations_external_refs WHERE app_id = $1")
        .bind(app_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM integrations_oauth_connections WHERE app_id = $1")
        .bind(app_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM integrations_connector_configs WHERE app_id = $1")
        .bind(app_id).execute(pool).await.ok();
}

/// Compute the Shopify HMAC-SHA256 signature (base64-encoded) for a body.
fn shopify_hmac_b64(secret: &str, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    STANDARD.encode(mac.finalize().into_bytes())
}

/// Seed a QBO OAuth connection with a test token.
async fn seed_qbo_connection(pool: &PgPool, app_id: &str, realm_id: &str) {
    std::env::set_var("OAUTH_ENCRYPTION_KEY", "test-encryption-key-cross-module");
    sqlx::query(
        r#"INSERT INTO integrations_oauth_connections
            (app_id, provider, realm_id,
             access_token, refresh_token,
             access_token_expires_at, refresh_token_expires_at,
             scopes_granted, connection_status)
           VALUES ($1, 'quickbooks', $2,
             pgp_sym_encrypt('test-access-token', 'test-encryption-key-cross-module'),
             pgp_sym_encrypt('test-refresh-token', 'test-encryption-key-cross-module'),
             NOW() + INTERVAL '1 hour', NOW() + INTERVAL '90 days',
             'com.intuit.quickbooks.accounting', 'connected')
           ON CONFLICT (app_id, provider) DO UPDATE
             SET realm_id = EXCLUDED.realm_id,
                 connection_status = 'connected',
                 access_token = EXCLUDED.access_token,
                 refresh_token = EXCLUDED.refresh_token,
                 updated_at = NOW()"#,
    )
    .bind(app_id)
    .bind(realm_id)
    .execute(pool)
    .await
    .expect("seed QBO OAuth connection");
}

/// Seed an AR customer → QBO customer mapping.
async fn seed_customer_ref(pool: &PgPool, app_id: &str, ar_customer_id: &str, qbo_customer_id: &str) {
    sqlx::query(
        r#"INSERT INTO integrations_external_refs
            (app_id, entity_type, entity_id, system, external_id, created_at, updated_at)
           VALUES ($1, 'ar_customer', $2, 'qbo', $3, NOW(), NOW())
           ON CONFLICT (app_id, system, external_id) DO NOTHING"#,
    )
    .bind(app_id)
    .bind(ar_customer_id)
    .bind(qbo_customer_id)
    .execute(pool)
    .await
    .expect("seed customer ref");
}

/// Start a minimal QBO invoice mock server.
async fn start_qbo_mock(realm_id: &str, qbo_invoice_id: &str) -> (String, Arc<AtomicU32>) {
    let call_count = Arc::new(AtomicU32::new(0));

    #[derive(Clone)]
    struct St {
        count: Arc<AtomicU32>,
        invoice_id: String,
    }

    async fn handle(axum::extract::State(s): axum::extract::State<St>) -> (axum::http::StatusCode, String) {
        s.count.fetch_add(1, Ordering::SeqCst);
        let body = serde_json::json!({"Invoice": {"Id": s.invoice_id, "SyncToken": "0"}}).to_string();
        (axum::http::StatusCode::OK, body)
    }

    let state = St { count: call_count.clone(), invoice_id: qbo_invoice_id.to_string() };
    let path = format!("/v3/company/{realm_id}/invoice");
    let app = axum::Router::new()
        .route(&path, axum::routing::post(handle))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind mock");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.ok() });

    (format!("http://{}/v3", addr), call_count)
}

async fn outbox_events(pool: &PgPool, app_id: &str, event_type: &str) -> Vec<serde_json::Value> {
    sqlx::query_as::<_, (serde_json::Value,)>(
        "SELECT payload FROM integrations_outbox WHERE app_id = $1 AND event_type = $2",
    )
    .bind(app_id)
    .bind(event_type)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|(v,)| v)
    .collect()
}

// ============================================================================
// Leg 1: Shopify webhook → file_job + order.ingested event
// ============================================================================

#[tokio::test]
#[serial]
async fn cross_module_shopify_webhook_creates_file_job_and_order_ingested_event() {
    let pool = setup_db().await;
    let app_id = unique_tenant();
    cleanup(&pool, &app_id).await;

    let webhook_secret = "shopify-test-secret-cross-module";

    // Minimal Shopify order payload
    let order_id = "6001234567890";
    let order_body = serde_json::json!({
        "id": order_id,
        "order_number": 1042,
        "financial_status": "paid",
        "email": "customer@example.com",
        "line_items": [
            {
                "id": "101",
                "product_id": "prod-1",
                "variant_id": "var-1",
                "title": "Widget Pro",
                "quantity": 2,
                "price": "49.99",
                "sku": "WGT-001"
            }
        ]
    });

    let raw_body = serde_json::to_vec(&order_body).unwrap();
    let sig = shopify_hmac_b64(webhook_secret, &raw_body);

    let mut headers = std::collections::HashMap::new();
    headers.insert("x-shopify-hmac-sha256".to_string(), sig);
    headers.insert("x-shopify-topic".to_string(), "orders/create".to_string());

    let connector_config = serde_json::json!({
        "shop_domain": "test-shop.myshopify.com",
        "webhook_secret": webhook_secret
    });

    let normalizer = ShopifyNormalizer::new(pool.clone());
    let result = normalizer
        .normalize(
            &raw_body,
            &order_body,
            &headers,
            &app_id,
            "orders/create",
            &connector_config,
        )
        .await
        .expect("normalize must succeed");

    assert!(!result.is_duplicate, "first ingest must not be a duplicate");

    // Verify file_job was created with parser_type = "shopify_order"
    let (parser_type, file_ref): (String, String) = sqlx::query_as(
        "SELECT parser_type, file_ref FROM integrations_file_jobs WHERE id = $1 AND tenant_id = $2",
    )
    .bind(result.file_job_id)
    .bind(&app_id)
    .fetch_one(&pool)
    .await
    .expect("file_job row must exist");

    assert_eq!(parser_type, "shopify_order", "parser_type must be shopify_order");
    assert!(file_ref.contains(order_id), "file_ref must include order_id");

    // Verify integrations.order.ingested outbox event with correct line items
    let events = outbox_events(&pool, &app_id, EVENT_TYPE_ORDER_INGESTED).await;
    assert_eq!(events.len(), 1, "exactly one order.ingested event must be in outbox");

    let payload = &events[0];
    assert_eq!(
        payload["payload"]["order_id"].as_str().unwrap(),
        order_id,
        "order_id must match"
    );
    assert_eq!(
        payload["payload"]["source"].as_str().unwrap(),
        "shopify",
        "source must be shopify"
    );
    let line_items = payload["payload"]["line_items"].as_array().unwrap();
    assert_eq!(line_items.len(), 1, "one line item expected");
    assert_eq!(line_items[0]["title"].as_str().unwrap(), "Widget Pro");
    assert_eq!(line_items[0]["quantity"].as_u64().unwrap(), 2);
    assert_eq!(line_items[0]["sku"].as_str().unwrap(), "WGT-001");

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn cross_module_shopify_webhook_idempotent_on_replay() {
    let pool = setup_db().await;
    let app_id = unique_tenant();
    cleanup(&pool, &app_id).await;

    let webhook_secret = "shopify-idem-secret";
    let order_id = "6009999999999";
    let order_body = serde_json::json!({
        "id": order_id,
        "order_number": 2001,
        "financial_status": "pending",
        "line_items": []
    });

    let raw_body = serde_json::to_vec(&order_body).unwrap();
    let sig = shopify_hmac_b64(webhook_secret, &raw_body);
    let mut headers = std::collections::HashMap::new();
    headers.insert("x-shopify-hmac-sha256".to_string(), sig);
    let connector_config = serde_json::json!({
        "shop_domain": "test.myshopify.com",
        "webhook_secret": webhook_secret
    });

    let normalizer = ShopifyNormalizer::new(pool.clone());

    let r1 = normalizer
        .normalize(&raw_body, &order_body, &headers, &app_id, "orders/create", &connector_config)
        .await
        .expect("first normalize");
    assert!(!r1.is_duplicate);

    // Replay the same webhook
    let r2 = normalizer
        .normalize(&raw_body, &order_body, &headers, &app_id, "orders/create", &connector_config)
        .await
        .expect("second normalize must not error");
    assert!(r2.is_duplicate, "replayed webhook must be identified as duplicate");

    // Only one file_job and one order.ingested event
    let (fj_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_file_jobs WHERE tenant_id = $1",
    )
    .bind(&app_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(fj_count, 1, "idempotent: must not create duplicate file_jobs");

    let events = outbox_events(&pool, &app_id, EVENT_TYPE_ORDER_INGESTED).await;
    assert_eq!(events.len(), 1, "idempotent: must not duplicate order.ingested events");

    cleanup(&pool, &app_id).await;
}

// ============================================================================
// Leg 2: ar.invoice_opened → QBO outbound → external_ref + qbo.invoice_created
// ============================================================================

#[tokio::test]
#[serial]
async fn cross_module_ar_invoice_opened_creates_qbo_invoice_and_outbox_event() {
    let pool = setup_db().await;
    let app_id = unique_tenant();
    cleanup(&pool, &app_id).await;

    let realm_id = "qbo-realm-cross-module";
    let ar_customer_id = "cust-xmod-001";
    let qbo_customer_id = "QBO-CUST-100";
    let ar_invoice_id = "inv-xmod-001";
    let qbo_invoice_id = "QBO-INV-9001";

    seed_qbo_connection(&pool, &app_id, realm_id).await;
    seed_customer_ref(&pool, &app_id, ar_customer_id, qbo_customer_id).await;

    let (qbo_base_url, call_count) = start_qbo_mock(realm_id, qbo_invoice_id).await;

    // Build ar.events.ar.invoice_opened NATS message
    let msg_payload = serde_json::json!({
        "event_id": Uuid::new_v4().to_string(),
        "event_type": "ar.invoice_opened",
        "occurred_at": "2026-04-09T00:00:00Z",
        "tenant_id": app_id,
        "source_module": "ar",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "payload": {
            "invoice_id": ar_invoice_id,
            "customer_id": ar_customer_id,
            "app_id": app_id,
            "amount_cents": 25000_i64,
            "currency": "usd",
            "created_at": "2026-04-09T00:00:00",
            "due_at": "2026-05-09T00:00:00",
            "paid_at": null
        }
    });
    let msg = BusMessage::new(
        "ar.events.ar.invoice_opened".to_string(),
        serde_json::to_vec(&msg_payload).unwrap(),
    );

    process_ar_invoice_opened(&pool, &msg, &qbo_base_url)
        .await
        .expect("process_ar_invoice_opened must succeed");

    // QBO mock called exactly once
    assert_eq!(call_count.load(Ordering::SeqCst), 1, "QBO mock must be called once");

    // External ref must be stored: ar_invoice → qbo_invoice
    let (system, external_id): (String, String) = sqlx::query_as(
        "SELECT system, external_id FROM integrations_external_refs \
         WHERE app_id = $1 AND entity_type = 'ar_invoice' AND entity_id = $2",
    )
    .bind(&app_id)
    .bind(ar_invoice_id)
    .fetch_one(&pool)
    .await
    .expect("external_ref must be stored");

    assert_eq!(system, "qbo_invoice");
    assert_eq!(external_id, qbo_invoice_id);

    // integrations.qbo.invoice_created event must be in outbox
    let events = outbox_events(&pool, &app_id, EVENT_TYPE_QBO_INVOICE_CREATED).await;
    assert_eq!(events.len(), 1, "qbo.invoice_created event must be in outbox");

    let evt_payload = &events[0]["payload"];
    assert_eq!(evt_payload["ar_invoice_id"].as_str().unwrap(), ar_invoice_id);
    assert_eq!(evt_payload["qbo_invoice_id"].as_str().unwrap(), qbo_invoice_id);
    assert_eq!(evt_payload["realm_id"].as_str().unwrap(), realm_id);

    cleanup(&pool, &app_id).await;
}
