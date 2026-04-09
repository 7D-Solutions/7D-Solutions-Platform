//! QBO outbound invoice sync integration tests (bd-yvc71).
//!
//! Run: ./scripts/cargo-slot.sh test -p integrations-rs -- qbo_outbound --nocapture
//!
//! Requires a running PostgreSQL instance at DATABASE_URL (defaults to the
//! standard integrations dev DB on port 5449).
//!
//! Tests 2-4 use a local axum stub as the QBO REST API — no sandbox credentials
//! needed.  The DB-backed token path is exercised by seeding the oauth
//! connections table with a pgcrypto-encrypted test token.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use event_bus::BusMessage;
use integrations_rs::domain::qbo::outbound::{
    process_ar_invoice_opened, EVENT_TYPE_QBO_INVOICE_CREATED, EVENT_TYPE_QBO_INVOICE_SYNC_FAILED,
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

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
async fn seed_oauth_connection(pool: &PgPool, app_id: &str, realm_id: &str) {
    // Set the encryption key so DbTokenProvider can decrypt
    std::env::set_var("OAUTH_ENCRYPTION_KEY", "test-encryption-key-for-qbo-outbound");

    sqlx::query(
        r#"
        INSERT INTO integrations_oauth_connections
            (app_id, provider, realm_id,
             access_token, refresh_token,
             access_token_expires_at, refresh_token_expires_at,
             scopes_granted, connection_status)
        VALUES
            ($1, 'quickbooks', $2,
             pgp_sym_encrypt('test-access-token', 'test-encryption-key-for-qbo-outbound'),
             pgp_sym_encrypt('test-refresh-token', 'test-encryption-key-for-qbo-outbound'),
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
    .execute(pool)
    .await
    .expect("seed oauth connection");
}

/// Seed a QBO customer mapping: ar_customer:customer_id → qbo:qbo_customer_id.
async fn seed_customer_ref(pool: &PgPool, app_id: &str, ar_customer_id: &str, qbo_customer_id: &str) {
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

/// Start a local axum server that simulates the QBO Invoice POST endpoint.
/// Returns (base_url, post_call_count).
async fn start_qbo_mock(realm_id: &str, qbo_invoice_id: &str) -> (String, Arc<AtomicU32>) {
    let call_count = Arc::new(AtomicU32::new(0));

    #[derive(Clone)]
    struct State {
        call_count: Arc<AtomicU32>,
        qbo_invoice_id: String,
    }

    async fn handle_invoice_post(
        axum::extract::State(s): axum::extract::State<State>,
    ) -> (axum::http::StatusCode, String) {
        s.call_count.fetch_add(1, Ordering::SeqCst);
        let body = serde_json::json!({
            "Invoice": {
                "Id": s.qbo_invoice_id,
                "SyncToken": "0",
                "DocNumber": "TEST-001"
            }
        })
        .to_string();
        (axum::http::StatusCode::OK, body)
    }

    let state = State {
        call_count: call_count.clone(),
        qbo_invoice_id: qbo_invoice_id.to_string(),
    };

    let path = format!("/v3/company/{realm_id}/invoice");
    let app = axum::Router::new()
        .route(&path, axum::routing::post(handle_invoice_post))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind QBO mock");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("QBO mock server")
    });

    (format!("http://{}/v3", addr), call_count)
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
async fn qbo_outbound_create_invoice_returns_valid_id() {
    use integrations_rs::domain::qbo::{
        client::{QboClient, QboInvoicePayload, QboLineItem},
        TokenProvider, QboError,
    };

    struct FixedTokenProvider;

    #[async_trait::async_trait]
    impl TokenProvider for FixedTokenProvider {
        async fn get_token(&self) -> Result<String, QboError> {
            Ok("test-token".into())
        }
        async fn refresh_token(&self) -> Result<String, QboError> {
            Ok("test-token".into())
        }
    }

    // Start a mock QBO server
    let realm_id = "realm-test-42";
    let expected_id = "INV-QBO-999";
    let (base_url, call_count) = start_qbo_mock(realm_id, expected_id).await;

    let client = QboClient::new(&base_url, realm_id, Arc::new(FixedTokenProvider));

    let payload = QboInvoicePayload {
        customer_ref: "1".to_string(),
        line_items: vec![QboLineItem {
            amount: 150.00,
            description: Some("Test service".to_string()),
            item_ref: Some("1".to_string()),
        }],
        due_date: Some("2026-12-31".to_string()),
        doc_number: Some("AR-001".to_string()),
    };

    let result = client.create_invoice(&payload).await;
    assert!(result.is_ok(), "create_invoice failed: {:?}", result);

    let invoice = result.unwrap();
    assert_eq!(
        invoice["Id"].as_str(),
        Some(expected_id),
        "returned invoice Id should match mock response"
    );
    assert_eq!(call_count.load(Ordering::SeqCst), 1, "exactly 1 POST to QBO");
}

// ============================================================================
// Test 2 — consumer happy path: AR event → QBO invoice + external ref + outbox
// ============================================================================

#[tokio::test]
#[serial]
async fn qbo_outbound_consumer_creates_invoice_and_stores_ref() {
    let pool = setup_db().await;
    let app_id = unique_tenant();
    let invoice_id = format!("inv-{}", Uuid::new_v4().simple());
    let customer_id = format!("cust-{}", Uuid::new_v4().simple());
    let realm_id = format!("realm-{}", Uuid::new_v4().simple());
    let qbo_customer_id = "42";
    let qbo_invoice_id = format!("qbo-inv-{}", Uuid::new_v4().simple());

    // Seed: customer mapping + QBO OAuth connection
    seed_customer_ref(&pool, &app_id, &customer_id, qbo_customer_id).await;
    seed_oauth_connection(&pool, &app_id, &realm_id).await;

    // Start QBO mock — the realm_id must match what the consumer will look up
    let (base_url, call_count) = start_qbo_mock(&realm_id, &qbo_invoice_id).await;

    let msg = make_ar_invoice_message(&app_id, &invoice_id, &customer_id, 25000, None);
    let result = process_ar_invoice_opened(&pool, &msg, &base_url).await;
    assert!(result.is_ok(), "process_ar_invoice_opened failed: {:?}", result);

    // QBO was called exactly once
    assert_eq!(call_count.load(Ordering::SeqCst), 1, "expected 1 QBO API call");

    // External ref stored
    let stored = find_external_ref(&pool, &app_id, "ar_invoice", &invoice_id, "qbo_invoice").await;
    assert_eq!(
        stored.as_deref(),
        Some(qbo_invoice_id.as_str()),
        "external ref should be stored with QBO invoice Id"
    );

    // Outbox event emitted
    let outbox = outbox_events(&pool, &app_id, EVENT_TYPE_QBO_INVOICE_CREATED).await;
    assert!(!outbox.is_empty(), "integrations.qbo.invoice_created outbox event should exist");

    let event_payload = &outbox[0];
    assert_eq!(
        event_payload["payload"]["ar_invoice_id"].as_str(),
        Some(invoice_id.as_str())
    );
    assert_eq!(
        event_payload["payload"]["qbo_invoice_id"].as_str(),
        Some(qbo_invoice_id.as_str())
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
    let invoice_id = format!("inv-{}", Uuid::new_v4().simple());
    let customer_id = format!("cust-{}", Uuid::new_v4().simple());
    let realm_id = format!("realm-{}", Uuid::new_v4().simple());
    let existing_qbo_id = "ALREADY-CREATED-99";

    // Pre-seed: the invoice is already mapped (simulates a previous successful sync)
    seed_customer_ref(&pool, &app_id, &customer_id, "42").await;
    seed_oauth_connection(&pool, &app_id, &realm_id).await;
    seed_invoice_ref(&pool, &app_id, &invoice_id, existing_qbo_id).await;

    // The QBO mock should NOT be called
    let (base_url, call_count) = start_qbo_mock(&realm_id, "should-not-be-used").await;

    let msg = make_ar_invoice_message(&app_id, &invoice_id, &customer_id, 25000, None);
    let result = process_ar_invoice_opened(&pool, &msg, &base_url).await;
    assert!(result.is_ok(), "idempotent call should succeed: {:?}", result);

    // QBO was NOT called
    assert_eq!(call_count.load(Ordering::SeqCst), 0, "QBO must not be called for duplicate event");

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
    let invoice_id = format!("inv-{}", Uuid::new_v4().simple());
    let customer_id = format!("cust-unmapped-{}", Uuid::new_v4().simple()); // no mapping seeded
    let realm_id = format!("realm-{}", Uuid::new_v4().simple());

    seed_oauth_connection(&pool, &app_id, &realm_id).await;
    // Deliberately NOT seeding a customer ref

    let (base_url, call_count) = start_qbo_mock(&realm_id, "should-not-be-used").await;

    let msg = make_ar_invoice_message(&app_id, &invoice_id, &customer_id, 10000, None);
    let result = process_ar_invoice_opened(&pool, &msg, &base_url).await;
    assert!(
        result.is_ok(),
        "missing customer should not propagate as error: {:?}",
        result
    );

    // QBO was NOT called
    assert_eq!(call_count.load(Ordering::SeqCst), 0, "QBO must not be called when customer is unmapped");

    // No invoice external ref created
    let stored = find_external_ref(&pool, &app_id, "ar_invoice", &invoice_id, "qbo_invoice").await;
    assert!(stored.is_none(), "no invoice ref should be stored for unmapped customer");

    // Error event emitted to outbox
    let errors = outbox_events(&pool, &app_id, EVENT_TYPE_QBO_INVOICE_SYNC_FAILED).await;
    assert!(
        !errors.is_empty(),
        "integrations.qbo.invoice_sync_failed outbox event should exist"
    );

    let err_payload = &errors[0]["payload"];
    assert_eq!(err_payload["ar_invoice_id"].as_str(), Some(invoice_id.as_str()));
    assert_eq!(err_payload["ar_customer_id"].as_str(), Some(customer_id.as_str()));
    assert_eq!(err_payload["reason"].as_str(), Some("no_qbo_customer_mapping"));

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
