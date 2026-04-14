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
use integrations_rs::domain::qbo::{client::QboClient, QboError, TokenProvider};
use integrations_rs::domain::webhooks::ShopifyNormalizer;
use integrations_rs::events::EVENT_TYPE_ORDER_INGESTED;
use serde_json::Value;
use serial_test::serial;
use sha2::Sha256;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

const QBO_OAUTH_ENCRYPTION_KEY: &str = "test-encryption-key-cross-module";

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
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_file_jobs WHERE tenant_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_webhook_ingest WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_external_refs WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_oauth_connections WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_connector_configs WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

/// Compute the Shopify HMAC-SHA256 signature (base64-encoded) for a body.
fn shopify_hmac_b64(secret: &str, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    STANDARD.encode(mac.finalize().into_bytes())
}

/// Seed a QBO OAuth connection with sandbox tokens.
async fn seed_qbo_connection(
    pool: &PgPool,
    app_id: &str,
    realm_id: &str,
    access_token: &str,
    refresh_token: &str,
) {
    std::env::set_var("OAUTH_ENCRYPTION_KEY", QBO_OAUTH_ENCRYPTION_KEY);
    sqlx::query(
        r#"INSERT INTO integrations_oauth_connections
            (app_id, provider, realm_id,
             access_token, refresh_token,
             access_token_expires_at, refresh_token_expires_at,
             scopes_granted, connection_status)
           VALUES ($1, 'quickbooks', $2,
             pgp_sym_encrypt($3, 'test-encryption-key-cross-module'),
             pgp_sym_encrypt($4, 'test-encryption-key-cross-module'),
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
    .bind(access_token)
    .bind(refresh_token)
    .execute(pool)
    .await
    .expect("seed QBO OAuth connection");
}

/// Seed an AR customer → QBO customer mapping.
async fn seed_customer_ref(
    pool: &PgPool,
    app_id: &str,
    ar_customer_id: &str,
    qbo_customer_id: &str,
) {
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

struct SandboxTokenProvider {
    access_token: RwLock<String>,
    refresh_tok: RwLock<String>,
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
    tokens_path: PathBuf,
}

impl SandboxTokenProvider {
    fn load() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        dotenvy::from_path(root.join(".env.qbo-sandbox")).expect(".env.qbo-sandbox not found");

        let client_id = std::env::var("QBO_CLIENT_ID").expect("QBO_CLIENT_ID");
        let client_secret = std::env::var("QBO_CLIENT_SECRET").expect("QBO_CLIENT_SECRET");

        let tokens_path = root.join(".qbo-tokens.json");
        let content = std::fs::read_to_string(&tokens_path).expect(".qbo-tokens.json");
        let tokens: Value = serde_json::from_str(&content).expect("invalid tokens JSON");

        Self {
            access_token: RwLock::new(tokens["access_token"].as_str().unwrap().into()),
            refresh_tok: RwLock::new(tokens["refresh_token"].as_str().unwrap().into()),
            client_id,
            client_secret,
            http: reqwest::Client::new(),
            tokens_path,
        }
    }

    fn realm_id(&self) -> String {
        let content = std::fs::read_to_string(&self.tokens_path).unwrap();
        let tokens: Value = serde_json::from_str(&content).unwrap();
        tokens["realm_id"].as_str().unwrap().to_string()
    }

    async fn tokens(&self) -> (String, String) {
        (
            self.access_token.read().await.clone(),
            self.refresh_tok.read().await.clone(),
        )
    }
}

#[async_trait::async_trait]
impl TokenProvider for SandboxTokenProvider {
    async fn get_token(&self) -> Result<String, QboError> {
        Ok(self.access_token.read().await.clone())
    }

    async fn refresh_token(&self) -> Result<String, QboError> {
        let rt = self.refresh_tok.read().await.clone();
        let resp = self
            .http
            .post("https://oauth.platform.intuit.com/oauth2/v1/tokens/bearer")
            .basic_auth(&self.client_id, Some(&self.client_secret))
            .form(&[("grant_type", "refresh_token"), ("refresh_token", &rt)])
            .send()
            .await
            .map_err(|e| QboError::TokenError(e.to_string()))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(QboError::TokenError(format!("Refresh failed: {}", body)));
        }

        let tr: Value = resp
            .json()
            .await
            .map_err(|e| QboError::TokenError(e.to_string()))?;
        let new_at = tr["access_token"]
            .as_str()
            .ok_or_else(|| QboError::TokenError("no access_token".into()))?
            .to_string();
        let new_rt = tr["refresh_token"]
            .as_str()
            .ok_or_else(|| QboError::TokenError("no refresh_token".into()))?
            .to_string();

        *self.access_token.write().await = new_at.clone();
        *self.refresh_tok.write().await = new_rt.clone();

        if let Ok(content) = std::fs::read_to_string(&self.tokens_path) {
            if let Ok(mut existing) = serde_json::from_str::<Value>(&content) {
                existing["access_token"] = Value::String(new_at.clone());
                existing["refresh_token"] = Value::String(new_rt);
                if let Some(v) = tr.get("expires_in") {
                    existing["expires_in"] = v.clone();
                }
                if let Some(v) = tr.get("x_refresh_token_expires_in") {
                    existing["x_refresh_token_expires_in"] = v.clone();
                }
                let _ = std::fs::write(
                    &self.tokens_path,
                    serde_json::to_string_pretty(&existing).unwrap(),
                );
            }
        }

        Ok(new_at)
    }
}

fn skip_unless_sandbox() -> bool {
    std::env::var("QBO_SANDBOX").map_or(true, |v| v != "1")
}

fn make_client() -> (QboClient, Arc<SandboxTokenProvider>) {
    let provider = Arc::new(SandboxTokenProvider::load());
    let base_url = std::env::var("QBO_SANDBOX_BASE")
        .unwrap_or_else(|_| "https://sandbox-quickbooks.api.intuit.com/v3".into());
    let realm_id = provider.realm_id();
    let client = QboClient::new(&base_url, &realm_id, provider.clone());
    (client, provider)
}

async fn first_customer_id(client: &QboClient) -> String {
    let response = client
        .query("SELECT * FROM Customer MAXRESULTS 1")
        .await
        .expect("customer query failed");
    response["QueryResponse"]["Customer"]
        .as_array()
        .and_then(|customers| customers.first())
        .and_then(|customer| customer["Id"].as_str())
        .expect("sandbox should have at least one customer")
        .to_string()
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

    assert_eq!(
        parser_type, "shopify_order",
        "parser_type must be shopify_order"
    );
    assert!(
        file_ref.contains(order_id),
        "file_ref must include order_id"
    );

    // Verify integrations.order.ingested outbox event with correct line items
    let events = outbox_events(&pool, &app_id, EVENT_TYPE_ORDER_INGESTED).await;
    assert_eq!(
        events.len(),
        1,
        "exactly one order.ingested event must be in outbox"
    );

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
        .normalize(
            &raw_body,
            &order_body,
            &headers,
            &app_id,
            "orders/create",
            &connector_config,
        )
        .await
        .expect("first normalize");
    assert!(!r1.is_duplicate);

    // Replay the same webhook
    let r2 = normalizer
        .normalize(
            &raw_body,
            &order_body,
            &headers,
            &app_id,
            "orders/create",
            &connector_config,
        )
        .await
        .expect("second normalize must not error");
    assert!(
        r2.is_duplicate,
        "replayed webhook must be identified as duplicate"
    );

    // Only one file_job and one order.ingested event
    let (fj_count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM integrations_file_jobs WHERE tenant_id = $1")
            .bind(&app_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        fj_count, 1,
        "idempotent: must not create duplicate file_jobs"
    );

    let events = outbox_events(&pool, &app_id, EVENT_TYPE_ORDER_INGESTED).await;
    assert_eq!(
        events.len(),
        1,
        "idempotent: must not duplicate order.ingested events"
    );

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

    if skip_unless_sandbox() {
        eprintln!("Skipping QBO sandbox test (set QBO_SANDBOX=1 to run)");
        return;
    }

    let (client, provider) = make_client();
    provider
        .refresh_token()
        .await
        .expect("token refresh via OAuth failed");
    let realm_id = provider.realm_id();
    let ar_customer_id = "cust-xmod-001";
    let ar_invoice_id = "inv-xmod-001";
    let qbo_customer_id = first_customer_id(&client).await;
    let (access_token, refresh_token) = provider.tokens().await;

    seed_qbo_connection(&pool, &app_id, &realm_id, &access_token, &refresh_token).await;
    seed_customer_ref(&pool, &app_id, ar_customer_id, &qbo_customer_id).await;

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

    process_ar_invoice_opened(
        &pool,
        &msg,
        &std::env::var("QBO_SANDBOX_BASE")
            .unwrap_or_else(|_| "https://sandbox-quickbooks.api.intuit.com/v3".into()),
    )
    .await
    .expect("process_ar_invoice_opened must succeed");

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
    assert!(
        !external_id.is_empty(),
        "sandbox should return a QBO invoice id"
    );

    // integrations.qbo.invoice_created event must be in outbox
    let events = outbox_events(&pool, &app_id, EVENT_TYPE_QBO_INVOICE_CREATED).await;
    assert_eq!(
        events.len(),
        1,
        "qbo.invoice_created event must be in outbox"
    );

    let evt_payload = &events[0]["payload"];
    assert_eq!(
        evt_payload["ar_invoice_id"].as_str().unwrap(),
        ar_invoice_id
    );
    assert_eq!(evt_payload["qbo_invoice_id"].as_str().unwrap(), external_id);
    assert_eq!(evt_payload["realm_id"].as_str().unwrap(), realm_id);

    cleanup(&pool, &app_id).await;
}
