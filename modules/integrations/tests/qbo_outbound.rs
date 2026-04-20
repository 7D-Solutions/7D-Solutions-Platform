//! QBO outbound invoice sync integration tests (bd-yvc71, bd-xgbji).
//!
//! Run: ./scripts/cargo-slot.sh test -p integrations-rs -- qbo_outbound --nocapture
//!
//! Requires a running PostgreSQL instance at DATABASE_URL (defaults to the
//! standard integrations dev DB on port 5449).
//!
//! Tests use the real QBO sandbox when `QBO_SANDBOX=1` is set.

use integrations_rs::domain::qbo::{client::QboClient, QboError, TokenProvider};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use event_bus::BusMessage;
use integrations_rs::domain::qbo::outbound::{
    legacy_consumers_enabled, process_ar_invoice_opened, process_order_ingested,
    EVENT_TYPE_QBO_INVOICE_CREATED, EVENT_TYPE_QBO_INVOICE_SYNC_FAILED,
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

const QBO_OAUTH_ENCRYPTION_KEY: &str = "test-encryption-key-for-qbo-outbound";

// ============================================================================
// Test helpers
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
        .expect("run integrations migrations");
    pool
}

fn unique_tenant() -> String {
    format!("qbo-ob-{}", Uuid::new_v4().simple())
}

fn short_id(prefix: &str) -> String {
    format!("{}-{}", prefix, &Uuid::new_v4().simple().to_string()[..8])
}

/// Remove all DB state created by a qbo_outbound test for a given app_id.
///
/// Must be called at the end of every DB-backed test to prevent stale OAuth
/// connections accumulating and disrupting `oauth_integration` tests (which
/// use `get_refresh_candidates` with a 10-minute look-ahead window — a
/// connection seeded with `NOW() + 1 hour` enters that window after ~50 min).
async fn cleanup_tenant(pool: &PgPool, app_id: &str) {
    sqlx::query("DELETE FROM integrations_oauth_connections WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_external_refs WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_outbox WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

/// Seed a QBO OAuth connection for `app_id` with `realm_id`.
/// Uses a known test encryption key and a plaintext test token.
async fn seed_oauth_connection(
    pool: &PgPool,
    app_id: &str,
    realm_id: &str,
    access_token: &str,
    refresh_token: &str,
) {
    // Set the encryption key so DbTokenProvider can decrypt the sandbox token.
    std::env::set_var("OAUTH_ENCRYPTION_KEY", QBO_OAUTH_ENCRYPTION_KEY);

    sqlx::query(
        r#"
        DELETE FROM integrations_oauth_connections
        WHERE provider = 'quickbooks' AND realm_id = $1
        "#,
    )
    .bind(realm_id)
    .execute(pool)
    .await
    .expect("clear stale sandbox oauth connection");

    sqlx::query(
        r#"
        INSERT INTO integrations_oauth_connections
            (app_id, provider, realm_id,
             access_token, refresh_token,
             access_token_expires_at, refresh_token_expires_at,
             scopes_granted, connection_status)
        VALUES
            ($1, 'quickbooks', $2,
             pgp_sym_encrypt($3, 'test-encryption-key-for-qbo-outbound'),
             pgp_sym_encrypt($4, 'test-encryption-key-for-qbo-outbound'),
             NOW() + INTERVAL '1 hour',
             NOW() + INTERVAL '90 days',
             'com.intuit.quickbooks.accounting',
             'connected')
        ON CONFLICT (app_id, provider) DO UPDATE
            SET realm_id = EXCLUDED.realm_id,
                connection_status = 'connected',
                access_token = EXCLUDED.access_token,
                refresh_token = EXCLUDED.refresh_token,
                updated_at = NOW()
        "#,
    )
    .bind(app_id)
    .bind(realm_id)
    .bind(access_token)
    .bind(refresh_token)
    .execute(pool)
    .await
    .expect("seed oauth connection");
}

/// Seed a QBO customer mapping: ar_customer:customer_id → qbo:qbo_customer_id.
async fn seed_customer_ref(
    pool: &PgPool,
    app_id: &str,
    ar_customer_id: &str,
    qbo_customer_id: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO integrations_external_refs
            (app_id, entity_type, entity_id, system, external_id, created_at, updated_at)
        VALUES ($1, 'ar_customer', $2, 'qbo', $3, NOW(), NOW())
        ON CONFLICT (app_id, system, external_id) DO NOTHING
        "#,
    )
    .bind(app_id)
    .bind(ar_customer_id)
    .bind(qbo_customer_id)
    .execute(pool)
    .await
    .expect("seed customer ref");
}

/// Seed an existing ar_invoice → qbo_invoice mapping (simulates already-synced state).
async fn seed_invoice_ref(pool: &PgPool, app_id: &str, ar_invoice_id: &str, qbo_invoice_id: &str) {
    sqlx::query(
        r#"
        INSERT INTO integrations_external_refs
            (app_id, entity_type, entity_id, system, external_id, created_at, updated_at)
        VALUES ($1, 'ar_invoice', $2, 'qbo_invoice', $3, NOW(), NOW())
        ON CONFLICT (app_id, system, external_id) DO NOTHING
        "#,
    )
    .bind(app_id)
    .bind(ar_invoice_id)
    .bind(qbo_invoice_id)
    .execute(pool)
    .await
    .expect("seed invoice ref");
}

/// Build a synthetic NATS BusMessage for ar.events.ar.invoice_opened.
fn make_ar_invoice_message(
    app_id: &str,
    invoice_id: &str,
    customer_id: &str,
    amount_cents: i64,
    due_at: Option<&str>,
) -> BusMessage {
    let payload = serde_json::json!({
        "event_id": Uuid::new_v4().to_string(),
        "event_type": "ar.invoice_opened",
        "occurred_at": "2026-04-08T12:00:00Z",
        "tenant_id": app_id,
        "source_module": "ar",
        "source_version": "1.0.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "payload": {
            "invoice_id": invoice_id,
            "customer_id": customer_id,
            "app_id": app_id,
            "amount_cents": amount_cents,
            "currency": "usd",
            "created_at": "2026-04-08T12:00:00",
            "due_at": due_at,
            "paid_at": null
        }
    });
    BusMessage::new(
        "ar.events.ar.invoice_opened".to_string(),
        serde_json::to_vec(&payload).unwrap(),
    )
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

/// Query the outbox for events of a given type for an app_id.
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

/// Query integrations_external_refs for a specific mapping.
async fn find_external_ref(
    pool: &PgPool,
    app_id: &str,
    entity_type: &str,
    entity_id: &str,
    system: &str,
) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT external_id FROM integrations_external_refs
         WHERE app_id = $1 AND entity_type = $2 AND entity_id = $3 AND system = $4",
    )
    .bind(app_id)
    .bind(entity_type)
    .bind(entity_id)
    .bind(system)
    .fetch_optional(pool)
    .await
    .unwrap_or(None)
}

// ============================================================================
// Test 1 — create_invoice() creates a QBO invoice and returns a valid Id
// ============================================================================

#[tokio::test]
#[serial]
async fn qbo_outbound_create_invoice_returns_valid_id() {
    use integrations_rs::domain::qbo::client::{QboInvoicePayload, QboLineItem};

    if skip_unless_sandbox() {
        eprintln!("Skipping QBO sandbox test (set QBO_SANDBOX=1 to run)");
        return;
    }

    let (client, provider) = make_client();
    provider
        .refresh_token()
        .await
        .expect("token refresh via OAuth failed");
    let customer_ref = first_customer_id(&client).await;

    let payload = QboInvoicePayload {
        customer_ref,
        line_items: vec![QboLineItem {
            amount: 150.00,
            description: Some("Test service".to_string()),
            item_ref: Some(
                std::env::var("QBO_DEFAULT_ITEM_REF").unwrap_or_else(|_| "1".to_string()),
            ),
        }],
        due_date: Some("2026-12-31".to_string()),
        doc_number: Some("AR-001".to_string()),
    };

    let result = client.create_invoice(&payload).await;
    assert!(result.is_ok(), "create_invoice failed: {:?}", result);

    let invoice = result.unwrap();
    assert!(
        invoice["Id"].as_str().is_some(),
        "sandbox should return an invoice Id"
    );
}

// ============================================================================
// Test 2 — consumer happy path: AR event → QBO invoice + external ref + outbox
// ============================================================================

#[tokio::test]
#[serial]
async fn qbo_outbound_consumer_creates_invoice_and_stores_ref() {
    let pool = setup_db().await;
    let app_id = unique_tenant();
    let invoice_id = short_id("inv");
    let customer_id = format!("cust-{}", Uuid::new_v4().simple());
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
    let qbo_customer_id = first_customer_id(&client).await;

    // Seed: customer mapping + QBO OAuth connection
    seed_customer_ref(&pool, &app_id, &customer_id, &qbo_customer_id).await;
    let (access_token, refresh_token) = provider.tokens().await;
    seed_oauth_connection(&pool, &app_id, &realm_id, &access_token, &refresh_token).await;

    let msg = make_ar_invoice_message(&app_id, &invoice_id, &customer_id, 25000, None);
    let result = process_ar_invoice_opened(
        &pool,
        &msg,
        &std::env::var("QBO_SANDBOX_BASE")
            .unwrap_or_else(|_| "https://sandbox-quickbooks.api.intuit.com/v3".into()),
    )
    .await;
    assert!(
        result.is_ok(),
        "process_ar_invoice_opened failed: {:?}",
        result
    );

    // External ref stored
    let stored = find_external_ref(&pool, &app_id, "ar_invoice", &invoice_id, "qbo_invoice").await;
    let qbo_invoice_id = stored.as_deref().expect("QBO invoice ref should exist");
    assert!(
        !qbo_invoice_id.is_empty(),
        "sandbox should emit a QBO invoice id"
    );

    // Outbox event emitted
    let outbox = outbox_events(&pool, &app_id, EVENT_TYPE_QBO_INVOICE_CREATED).await;
    assert!(
        !outbox.is_empty(),
        "integrations.qbo.invoice_created outbox event should exist"
    );

    let event_payload = &outbox[0];
    assert_eq!(
        event_payload["payload"]["ar_invoice_id"].as_str(),
        Some(invoice_id.as_str())
    );
    assert_eq!(
        event_payload["payload"]["qbo_invoice_id"].as_str(),
        Some(qbo_invoice_id)
    );

    cleanup_tenant(&pool, &app_id).await;
}

// ============================================================================
// Test 3 — idempotency: re-publishing the same event is a no-op
// ============================================================================

#[tokio::test]
#[serial]
async fn qbo_outbound_consumer_idempotent_on_duplicate_event() {
    let pool = setup_db().await;
    let app_id = unique_tenant();
    let invoice_id = short_id("inv");
    let customer_id = format!("cust-{}", Uuid::new_v4().simple());
    let existing_qbo_id = "ALREADY-CREATED-99";

    // Pre-seed: the invoice is already mapped (simulates a previous successful sync)
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
    let qbo_customer_id = first_customer_id(&client).await;
    seed_customer_ref(&pool, &app_id, &customer_id, &qbo_customer_id).await;
    let (access_token, refresh_token) = provider.tokens().await;
    seed_oauth_connection(&pool, &app_id, &realm_id, &access_token, &refresh_token).await;
    seed_invoice_ref(&pool, &app_id, &invoice_id, existing_qbo_id).await;

    let msg = make_ar_invoice_message(&app_id, &invoice_id, &customer_id, 25000, None);
    let result = process_ar_invoice_opened(
        &pool,
        &msg,
        &std::env::var("QBO_SANDBOX_BASE")
            .unwrap_or_else(|_| "https://sandbox-quickbooks.api.intuit.com/v3".into()),
    )
    .await;
    assert!(
        result.is_ok(),
        "idempotent call should succeed: {:?}",
        result
    );

    // The existing external ref is unchanged
    let stored = find_external_ref(&pool, &app_id, "ar_invoice", &invoice_id, "qbo_invoice").await;
    assert_eq!(stored.as_deref(), Some(existing_qbo_id));

    cleanup_tenant(&pool, &app_id).await;
}

// ============================================================================
// Test 4 — missing customer mapping: consumer emits error event, no QBO call
// ============================================================================

#[tokio::test]
#[serial]
async fn qbo_outbound_consumer_missing_customer_emits_error_event() {
    let pool = setup_db().await;
    let app_id = unique_tenant();
    let invoice_id = short_id("inv");
    let customer_id = format!("cust-unmapped-{}", Uuid::new_v4().simple()); // no mapping seeded
    if skip_unless_sandbox() {
        eprintln!("Skipping QBO sandbox test (set QBO_SANDBOX=1 to run)");
        return;
    }
    let (_, provider) = make_client();
    provider
        .refresh_token()
        .await
        .expect("token refresh via OAuth failed");
    let realm_id = provider.realm_id();
    let (access_token, refresh_token) = provider.tokens().await;
    seed_oauth_connection(&pool, &app_id, &realm_id, &access_token, &refresh_token).await;
    // Deliberately NOT seeding a customer ref

    let msg = make_ar_invoice_message(&app_id, &invoice_id, &customer_id, 10000, None);
    let result = process_ar_invoice_opened(
        &pool,
        &msg,
        &std::env::var("QBO_SANDBOX_BASE")
            .unwrap_or_else(|_| "https://sandbox-quickbooks.api.intuit.com/v3".into()),
    )
    .await;
    assert!(
        result.is_ok(),
        "missing customer should not propagate as error: {:?}",
        result
    );

    // No invoice external ref created
    let stored = find_external_ref(&pool, &app_id, "ar_invoice", &invoice_id, "qbo_invoice").await;
    assert!(
        stored.is_none(),
        "no invoice ref should be stored for unmapped customer"
    );

    // Error event emitted to outbox
    let errors = outbox_events(&pool, &app_id, EVENT_TYPE_QBO_INVOICE_SYNC_FAILED).await;
    assert!(
        !errors.is_empty(),
        "integrations.qbo.invoice_sync_failed outbox event should exist"
    );

    let err_payload = &errors[0]["payload"];
    assert_eq!(
        err_payload["ar_invoice_id"].as_str(),
        Some(invoice_id.as_str())
    );
    assert_eq!(
        err_payload["ar_customer_id"].as_str(),
        Some(customer_id.as_str())
    );
    assert_eq!(
        err_payload["reason"].as_str(),
        Some("no_qbo_customer_mapping")
    );

    cleanup_tenant(&pool, &app_id).await;
}

// ============================================================================
// Test 5 — consumer shuts down gracefully (no panic, no crash)
// ============================================================================

#[tokio::test]
async fn qbo_outbound_consumer_shuts_down_gracefully() {
    use event_bus::InMemoryBus;
    use integrations_rs::domain::qbo::outbound::spawn_outbound_consumer;
    use tokio::sync::watch;

    let pool = setup_db().await;
    let bus: Arc<dyn event_bus::EventBus> = Arc::new(InMemoryBus::new());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let handle = spawn_outbound_consumer(pool, bus, shutdown_rx);

    // Signal shutdown immediately — consumer should drain and exit cleanly
    shutdown_tx.send(true).unwrap();

    let result = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
    assert!(result.is_ok(), "consumer should shut down within 5 seconds");
    assert!(result.unwrap().is_ok(), "consumer task should not panic");
}

// ============================================================================
// Helpers for order-ingested tests
// ============================================================================

/// Seed a marketplace_customer → QBO customer mapping.
async fn seed_marketplace_customer_ref(
    pool: &PgPool,
    app_id: &str,
    customer_ref: &str,
    qbo_customer_id: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO integrations_external_refs
            (app_id, entity_type, entity_id, system, external_id, created_at, updated_at)
        VALUES ($1, 'marketplace_customer', $2, 'qbo', $3, NOW(), NOW())
        ON CONFLICT (app_id, system, external_id) DO NOTHING
        "#,
    )
    .bind(app_id)
    .bind(customer_ref)
    .bind(qbo_customer_id)
    .execute(pool)
    .await
    .expect("seed marketplace customer ref");
}

/// Seed an existing marketplace_order → qbo_invoice mapping (simulates already-synced state).
async fn seed_marketplace_order_ref(
    pool: &PgPool,
    app_id: &str,
    order_id: &str,
    qbo_invoice_id: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO integrations_external_refs
            (app_id, entity_type, entity_id, system, external_id, created_at, updated_at)
        VALUES ($1, 'marketplace_order', $2, 'qbo_invoice', $3, NOW(), NOW())
        ON CONFLICT (app_id, system, external_id) DO NOTHING
        "#,
    )
    .bind(app_id)
    .bind(order_id)
    .bind(qbo_invoice_id)
    .execute(pool)
    .await
    .expect("seed marketplace order ref");
}

/// Build a synthetic NATS BusMessage for integrations.order.ingested.
fn make_order_ingested_message(
    app_id: &str,
    order_id: &str,
    source: &str,
    customer_ref: Option<&str>,
    line_items: &[(
        /* title */ &str,
        /* price */ &str,
        /* qty */ u32,
    )],
) -> BusMessage {
    let items: Vec<serde_json::Value> = line_items
        .iter()
        .map(|(title, price, qty)| {
            serde_json::json!({
                "product_id": "prod-1",
                "variant_id": "var-1",
                "title": title,
                "quantity": qty,
                "price": price,
                "sku": null
            })
        })
        .collect();

    let payload = serde_json::json!({
        "event_id": Uuid::new_v4().to_string(),
        "event_type": "integrations.order.ingested",
        "occurred_at": "2026-04-09T10:00:00Z",
        "tenant_id": app_id,
        "source_module": "integrations",
        "source_version": "2.3.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "payload": {
            "tenant_id": app_id,
            "source": source,
            "order_id": order_id,
            "order_number": 1001_u64,
            "financial_status": "paid",
            "line_items": items,
            "customer_ref": customer_ref,
            "file_job_id": Uuid::new_v4().to_string(),
            "ingested_at": "2026-04-09T10:00:00Z"
        }
    });
    BusMessage::new(
        "integrations.order.ingested".to_string(),
        serde_json::to_vec(&payload).unwrap(),
    )
}

// ============================================================================
// Test 6 — order.ingested happy path: creates QBO invoice and stores ext ref
// ============================================================================

#[tokio::test]
#[serial]
async fn qbo_outbound_order_ingested_creates_invoice_and_stores_ref() {
    let pool = setup_db().await;
    let app_id = unique_tenant();
    let order_id = short_id("ord");
    let customer_ref = format!("buyer-{}@example.com", Uuid::new_v4().simple());
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
    let qbo_customer_id = first_customer_id(&client).await;
    let (access_token, refresh_token) = provider.tokens().await;

    seed_marketplace_customer_ref(&pool, &app_id, &customer_ref, &qbo_customer_id).await;
    seed_oauth_connection(&pool, &app_id, &realm_id, &access_token, &refresh_token).await;

    let msg = make_order_ingested_message(
        &app_id,
        &order_id,
        "shopify",
        Some(&customer_ref),
        &[("Widget A", "29.99", 2), ("Widget B", "9.99", 1)],
    );
    let result = process_order_ingested(
        &pool,
        &msg,
        &std::env::var("QBO_SANDBOX_BASE")
            .unwrap_or_else(|_| "https://sandbox-quickbooks.api.intuit.com/v3".into()),
    )
    .await;
    assert!(
        result.is_ok(),
        "process_order_ingested failed: {:?}",
        result
    );

    // External ref stored under marketplace_order entity type
    let stored = find_external_ref(
        &pool,
        &app_id,
        "marketplace_order",
        &order_id,
        "qbo_invoice",
    )
    .await;
    let qbo_invoice_id = stored.as_deref().expect("QBO invoice ref should exist");
    assert!(
        !qbo_invoice_id.is_empty(),
        "sandbox should emit a QBO invoice id"
    );

    // Outbox event emitted
    let outbox = outbox_events(&pool, &app_id, EVENT_TYPE_QBO_INVOICE_CREATED).await;
    assert!(
        !outbox.is_empty(),
        "integrations.qbo.invoice_created outbox event should exist"
    );
    assert_eq!(
        outbox[0]["payload"]["qbo_invoice_id"].as_str(),
        Some(qbo_invoice_id)
    );

    cleanup_tenant(&pool, &app_id).await;
}

// ============================================================================
// Test 7 — source == "qbo" is skipped (circular creation guard)
// ============================================================================

#[tokio::test]
#[serial]
async fn qbo_outbound_order_ingested_skips_qbo_source() {
    let pool = setup_db().await;
    let app_id = unique_tenant();
    let order_id = short_id("ord");
    let customer_ref = format!("buyer-{}@example.com", Uuid::new_v4().simple());
    if skip_unless_sandbox() {
        eprintln!("Skipping QBO sandbox test (set QBO_SANDBOX=1 to run)");
        return;
    }
    let (_, provider) = make_client();
    provider
        .refresh_token()
        .await
        .expect("token refresh via OAuth failed");
    let realm_id = provider.realm_id();
    let (access_token, refresh_token) = provider.tokens().await;
    seed_marketplace_customer_ref(&pool, &app_id, &customer_ref, "99").await;
    seed_oauth_connection(&pool, &app_id, &realm_id, &access_token, &refresh_token).await;

    let msg = make_order_ingested_message(
        &app_id,
        &order_id,
        "qbo", // source is qbo — must be skipped
        Some(&customer_ref),
        &[("Widget", "10.00", 1)],
    );
    let result = process_order_ingested(
        &pool,
        &msg,
        &std::env::var("QBO_SANDBOX_BASE")
            .unwrap_or_else(|_| "https://sandbox-quickbooks.api.intuit.com/v3".into()),
    )
    .await;
    assert!(
        result.is_ok(),
        "qbo-sourced order should return Ok: {:?}",
        result
    );

    let stored = find_external_ref(
        &pool,
        &app_id,
        "marketplace_order",
        &order_id,
        "qbo_invoice",
    )
    .await;
    assert!(
        stored.is_none(),
        "no external ref should be created for skipped order"
    );

    cleanup_tenant(&pool, &app_id).await;
}

// ============================================================================
// Test 8 — idempotency: second order.ingested for same order is a no-op
// ============================================================================

#[tokio::test]
#[serial]
async fn qbo_outbound_order_ingested_idempotent_on_duplicate() {
    let pool = setup_db().await;
    let app_id = unique_tenant();
    let order_id = short_id("ord");
    let customer_ref = format!("buyer-{}@example.com", Uuid::new_v4().simple());
    if skip_unless_sandbox() {
        eprintln!("Skipping QBO sandbox test (set QBO_SANDBOX=1 to run)");
        return;
    }
    let existing_qbo_id = "ALREADY-SYNCED-ORDER-88";

    let (_, provider) = make_client();
    provider
        .refresh_token()
        .await
        .expect("token refresh via OAuth failed");
    let realm_id = provider.realm_id();
    let (access_token, refresh_token) = provider.tokens().await;

    seed_marketplace_customer_ref(&pool, &app_id, &customer_ref, "42").await;
    seed_oauth_connection(&pool, &app_id, &realm_id, &access_token, &refresh_token).await;
    seed_marketplace_order_ref(&pool, &app_id, &order_id, existing_qbo_id).await;

    let msg = make_order_ingested_message(
        &app_id,
        &order_id,
        "shopify",
        Some(&customer_ref),
        &[("Widget", "20.00", 1)],
    );
    let result = process_order_ingested(
        &pool,
        &msg,
        &std::env::var("QBO_SANDBOX_BASE")
            .unwrap_or_else(|_| "https://sandbox-quickbooks.api.intuit.com/v3".into()),
    )
    .await;
    assert!(
        result.is_ok(),
        "idempotent call should succeed: {:?}",
        result
    );

    let stored = find_external_ref(
        &pool,
        &app_id,
        "marketplace_order",
        &order_id,
        "qbo_invoice",
    )
    .await;
    assert_eq!(
        stored.as_deref(),
        Some(existing_qbo_id),
        "existing ref must be unchanged"
    );

    cleanup_tenant(&pool, &app_id).await;
}

// ============================================================================
// Test 9 — missing customer mapping emits error event
// ============================================================================

#[tokio::test]
#[serial]
async fn qbo_outbound_order_ingested_missing_customer_emits_error_event() {
    let pool = setup_db().await;
    let app_id = unique_tenant();
    let order_id = short_id("ord");
    let customer_ref = format!("unknown-{}@example.com", Uuid::new_v4().simple());
    if skip_unless_sandbox() {
        eprintln!("Skipping QBO sandbox test (set QBO_SANDBOX=1 to run)");
        return;
    }

    let (_, provider) = make_client();
    provider
        .refresh_token()
        .await
        .expect("token refresh via OAuth failed");
    let realm_id = provider.realm_id();
    let (access_token, refresh_token) = provider.tokens().await;
    seed_oauth_connection(&pool, &app_id, &realm_id, &access_token, &refresh_token).await;
    // Deliberately NOT seeding a marketplace_customer ref

    let msg = make_order_ingested_message(
        &app_id,
        &order_id,
        "amazon",
        Some(&customer_ref),
        &[("Widget", "5.00", 3)],
    );
    let result = process_order_ingested(
        &pool,
        &msg,
        &std::env::var("QBO_SANDBOX_BASE")
            .unwrap_or_else(|_| "https://sandbox-quickbooks.api.intuit.com/v3".into()),
    )
    .await;
    assert!(
        result.is_ok(),
        "missing customer should not propagate as error: {:?}",
        result
    );

    let stored = find_external_ref(
        &pool,
        &app_id,
        "marketplace_order",
        &order_id,
        "qbo_invoice",
    )
    .await;
    assert!(
        stored.is_none(),
        "no invoice ref should be stored for unmapped customer"
    );

    let errors = outbox_events(&pool, &app_id, EVENT_TYPE_QBO_INVOICE_SYNC_FAILED).await;
    assert!(!errors.is_empty(), "sync_failed outbox event should exist");
    assert_eq!(
        errors[0]["payload"]["reason"].as_str(),
        Some("no_qbo_customer_mapping")
    );

    cleanup_tenant(&pool, &app_id).await;
}

// ============================================================================
// Test 10 — cutover feature flag: legacy consumers default OFF
// ============================================================================

#[tokio::test]
#[serial]
async fn qbo_legacy_consumers_flag_is_off_by_default() {
    std::env::remove_var("QBO_LEGACY_CONSUMERS_ENABLED");
    assert!(
        !legacy_consumers_enabled(),
        "legacy consumers must be OFF when QBO_LEGACY_CONSUMERS_ENABLED is unset"
    );
}

#[tokio::test]
#[serial]
async fn qbo_legacy_consumers_flag_on_when_set_to_1() {
    std::env::set_var("QBO_LEGACY_CONSUMERS_ENABLED", "1");
    let enabled = legacy_consumers_enabled();
    std::env::remove_var("QBO_LEGACY_CONSUMERS_ENABLED");
    assert!(enabled, "legacy consumers must be ON when QBO_LEGACY_CONSUMERS_ENABLED=1");
}

#[tokio::test]
#[serial]
async fn qbo_legacy_consumers_flag_off_when_flag_is_not_1() {
    for val in &["0", "true", "yes"] {
        std::env::set_var("QBO_LEGACY_CONSUMERS_ENABLED", val);
        let enabled = legacy_consumers_enabled();
        std::env::remove_var("QBO_LEGACY_CONSUMERS_ENABLED");
        assert!(
            !enabled,
            "legacy consumers must be OFF for QBO_LEGACY_CONSUMERS_ENABLED={val}"
        );
    }
}
