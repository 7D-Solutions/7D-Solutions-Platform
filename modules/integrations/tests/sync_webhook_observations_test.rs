//! Integration tests: QBO webhook normalization writes canonical observation rows.
//!
//! These tests exercise the fetch-and-observe path added to `QboNormalizer`:
//! - Non-delete events trigger a GET fetch from QBO (local axum server) followed
//!   by an observation upsert with `source_channel = "webhook"`.
//! - Delete events write a tombstone observation directly without fetching.
//! - Duplicate webhook deliveries collapse to a single observation via fingerprint
//!   uniqueness (the upsert ON CONFLICT semantics).
//!
//! No mocks, no stubs — real Postgres DB at DATABASE_URL.  A local axum server
//! stands in for the QBO REST API because Intuit's sandbox cannot be forced to
//! return specific entity shapes on demand.
//!
//! Run: ./scripts/cargo-slot.sh test -p integrations-rs -- sync_webhook_observations --nocapture

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::routing::get;
use axum::Router;
use integrations_rs::domain::sync::observations;
use integrations_rs::domain::webhooks::QboNormalizer;
use serde_json::json;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::OnceCell;
use uuid::Uuid;

// ── Shared pool ───────────────────────────────────────────────────────────────

static TEST_POOL: OnceCell<sqlx::PgPool> = OnceCell::const_new();

async fn init_pool() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db".to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(90))
        .connect(&url)
        .await
        .expect("Failed to connect to integrations test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run integrations migrations");
    pool
}

async fn setup_db() -> sqlx::PgPool {
    TEST_POOL.get_or_init(init_pool).await.clone()
}

// ── DB helpers ────────────────────────────────────────────────────────────────

const TEST_ENC_KEY: &str = "test-enc-key-webhook-obs";

fn unique_app() -> String {
    format!("whobs-{}", Uuid::new_v4().simple())
}

fn unique_realm() -> String {
    format!("realm-whobs-{}", Uuid::new_v4().simple())
}

async fn seed_connection(pool: &sqlx::PgPool, app_id: &str, realm_id: &str) {
    std::env::set_var("OAUTH_ENCRYPTION_KEY", TEST_ENC_KEY);
    sqlx::query(
        "INSERT INTO integrations_oauth_connections
         (app_id, provider, realm_id, access_token, refresh_token,
          access_token_expires_at, refresh_token_expires_at, scopes_granted,
          connection_status)
         VALUES ($1, 'quickbooks', $2,
                 pgp_sym_encrypt('fake-access-token', $3),
                 pgp_sym_encrypt('fake-refresh-token', $3),
                 NOW() + INTERVAL '1 hour', NOW() + INTERVAL '100 days',
                 'com.intuit.quickbooks.accounting', 'connected')
         ON CONFLICT (app_id, provider) DO UPDATE
             SET realm_id = EXCLUDED.realm_id,
                 access_token = EXCLUDED.access_token,
                 refresh_token = EXCLUDED.refresh_token,
                 access_token_expires_at = EXCLUDED.access_token_expires_at,
                 connection_status = 'connected'",
    )
    .bind(app_id)
    .bind(realm_id)
    .bind(TEST_ENC_KEY)
    .execute(pool)
    .await
    .expect("seed connection");
}

async fn cleanup(pool: &sqlx::PgPool, app_id: &str, _realm_id: &str) {
    sqlx::query("DELETE FROM integrations_sync_observations WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_sync_conflicts WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_sync_push_attempts WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
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
    sqlx::query("DELETE FROM integrations_webhook_batch_ingest WHERE payload->>'app_id' = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query(
        "DELETE FROM integrations_oauth_connections \
         WHERE provider = 'quickbooks' AND app_id = $1",
    )
    .bind(app_id)
    .execute(pool)
    .await
    .ok();
}

// ── Local QBO API server ──────────────────────────────────────────────────────

/// Entity responses returned by the mock QBO API server.
#[derive(Clone)]
struct MockEntities {
    /// Keyed by `(entity_type_lower, entity_id)`.
    pub responses: Arc<std::collections::HashMap<(String, String), serde_json::Value>>,
}

async fn handle_get_entity(
    Path((_realm, entity_type, entity_id)): Path<(String, String, String)>,
    State(state): State<MockEntities>,
) -> (axum::http::StatusCode, axum::Json<serde_json::Value>) {
    let key = (entity_type.to_lowercase(), entity_id.clone());
    match state.responses.get(&key) {
        Some(entity) => {
            let entity_key = capitalize(&entity_type);
            (
                axum::http::StatusCode::OK,
                axum::Json(json!({ entity_key: entity })),
            )
        }
        None => (
            axum::http::StatusCode::NOT_FOUND,
            axum::Json(json!({ "error": "not found" })),
        ),
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().chain(c).collect(),
    }
}

/// Spin up a local axum server at a random port.
///
/// Returns `(base_url, server_join_handle)`.
async fn start_mock_qbo(
    responses: HashMap<(String, String), serde_json::Value>,
) -> (String, tokio::task::JoinHandle<()>) {
    let state = MockEntities {
        responses: Arc::new(responses),
    };
    let app = Router::new()
        .route(
            "/v3/company/{realm}/{entity_type}/{entity_id}",
            get(handle_get_entity),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("mock QBO server crashed")
    });
    (format!("http://{}/v3", addr), handle)
}

// ── CloudEvent factory ────────────────────────────────────────────────────────

fn cloud_event(id: &str, event_type: &str, entity_id: &str, realm_id: &str) -> serde_json::Value {
    json!({
        "id": id,
        "type": event_type,
        "time": "2026-04-20T10:00:00Z",
        "intuitentityid": entity_id,
        "intuitaccountid": realm_id,
        "data": {}
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn webhook_non_delete_writes_observation_with_webhook_channel() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let realm_id = unique_realm();
    cleanup(&pool, &app_id, &realm_id).await;
    seed_connection(&pool, &app_id, &realm_id).await;

    let entity_id = "cust-1";
    let mut responses = HashMap::new();
    responses.insert(
        ("customer".to_string(), entity_id.to_string()),
        json!({
            "Id": entity_id,
            "DisplayName": "Acme Corp",
            "SyncToken": "5",
            "MetaData": {
                "LastUpdatedTime": "2026-04-20T10:00:00Z",
                "CreateTime": "2026-04-01T00:00:00Z"
            }
        }),
    );

    let (base_url, _srv) = start_mock_qbo(responses).await;
    let normalizer = QboNormalizer::new_with_base_url(pool.clone(), base_url);

    let events = json!([cloud_event(
        "ev-1",
        "qbo.customer.created.v1",
        entity_id,
        &realm_id
    )]);
    let body = serde_json::to_vec(&events).expect("serialize");
    normalizer
        .normalize(&body, &events, &HashMap::new())
        .await
        .expect("normalize");

    // Allow the async fetch task to write the observation.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let obs =
        observations::get_latest_for_entity(&pool, &app_id, "quickbooks", "customer", entity_id)
            .await
            .expect("query observation")
            .expect("observation must exist");

    assert_eq!(
        obs.source_channel, "webhook",
        "source_channel must be 'webhook'"
    );
    assert!(!obs.is_tombstone, "must not be a tombstone");
    assert_eq!(obs.entity_id, entity_id);
    assert_eq!(obs.provider, "quickbooks");
    assert_eq!(obs.fingerprint, "st:5", "fingerprint must use SyncToken");

    cleanup(&pool, &app_id, &realm_id).await;
}

#[tokio::test]
#[serial]
async fn webhook_delete_writes_tombstone_without_fetch() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let realm_id = unique_realm();
    cleanup(&pool, &app_id, &realm_id).await;
    seed_connection(&pool, &app_id, &realm_id).await;

    let entity_id = "inv-99";
    // No entry in mock server — a fetch would fail.  Tombstone must be written without fetching.
    let (base_url, _srv) = start_mock_qbo(HashMap::new()).await;
    let normalizer = QboNormalizer::new_with_base_url(pool.clone(), base_url);

    let events = json!([cloud_event(
        "ev-del-1",
        "qbo.invoice.deleted.v1",
        entity_id,
        &realm_id
    )]);
    let body = serde_json::to_vec(&events).expect("serialize");
    normalizer
        .normalize(&body, &events, &HashMap::new())
        .await
        .expect("normalize");

    tokio::time::sleep(Duration::from_millis(200)).await;

    let obs =
        observations::get_latest_for_entity(&pool, &app_id, "quickbooks", "invoice", entity_id)
            .await
            .expect("query observation")
            .expect("tombstone observation must exist");

    assert!(obs.is_tombstone, "must be a tombstone");
    assert_eq!(obs.source_channel, "webhook");
    assert_eq!(obs.entity_id, entity_id);

    cleanup(&pool, &app_id, &realm_id).await;
}

#[tokio::test]
#[serial]
async fn duplicate_webhook_delivery_collapses_to_one_observation() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let realm_id = unique_realm();
    cleanup(&pool, &app_id, &realm_id).await;
    seed_connection(&pool, &app_id, &realm_id).await;

    let entity_id = "pay-5";
    let sync_token = "3";
    let lut = "2026-04-20T09:00:00Z";

    let mut responses = HashMap::new();
    responses.insert(
        ("payment".to_string(), entity_id.to_string()),
        json!({
            "Id": entity_id,
            "TotalAmt": 100.0,
            "SyncToken": sync_token,
            "MetaData": {
                "LastUpdatedTime": lut,
                "CreateTime": "2026-04-01T00:00:00Z"
            }
        }),
    );

    let (base_url, _srv) = start_mock_qbo(responses).await;

    // First delivery.
    let events = json!([cloud_event(
        "ev-pay-a",
        "qbo.payment.created.v1",
        entity_id,
        &realm_id
    )]);
    let body = serde_json::to_vec(&events).expect("serialize");
    QboNormalizer::new_with_base_url(pool.clone(), base_url.clone())
        .normalize(&body, &events, &HashMap::new())
        .await
        .expect("normalize first");

    // Second delivery — different event id (would be different Intuit POST) but same entity state.
    let events2 = json!([cloud_event(
        "ev-pay-b",
        "qbo.payment.created.v1",
        entity_id,
        &realm_id
    )]);
    let body2 = serde_json::to_vec(&events2).expect("serialize");
    QboNormalizer::new_with_base_url(pool.clone(), base_url.clone())
        .normalize(&body2, &events2, &HashMap::new())
        .await
        .expect("normalize second");

    tokio::time::sleep(Duration::from_millis(300)).await;

    // There should be exactly one observation row for this fingerprint (SyncToken-based).
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_sync_observations \
         WHERE app_id = $1 AND provider = 'quickbooks' \
           AND entity_type = 'payment' AND entity_id = $2 \
           AND fingerprint = $3",
    )
    .bind(&app_id)
    .bind(entity_id)
    .bind(format!("st:{}", sync_token))
    .fetch_one(&pool)
    .await
    .expect("count observations");

    assert_eq!(
        count.0, 1,
        "duplicate webhook deliveries must collapse to one observation row"
    );

    cleanup(&pool, &app_id, &realm_id).await;
}

#[tokio::test]
#[serial]
async fn webhook_and_cdc_same_state_collapse_via_upsert() {
    use integrations_rs::domain::sync::dedupe::{
        compute_comparable_hash, compute_fingerprint, truncate_to_millis,
    };

    let pool = setup_db().await;
    let app_id = unique_app();
    let realm_id = unique_realm();
    cleanup(&pool, &app_id, &realm_id).await;
    seed_connection(&pool, &app_id, &realm_id).await;

    let entity_id = "item-7";
    let sync_token = "2";
    let lut_str = "2026-04-20T08:00:00Z";
    let entity = json!({
        "Id": entity_id,
        "Name": "Widget",
        "SyncToken": sync_token,
        "MetaData": {
            "LastUpdatedTime": lut_str,
            "CreateTime": "2026-01-01T00:00:00Z"
        }
    });

    // Write via CDC path using comparable fields (Id + Name only; strip MetaData/SyncToken).
    {
        let lut: chrono::DateTime<chrono::Utc> = lut_str.parse().expect("parse lut");
        let lut_ms = truncate_to_millis(lut);
        let fingerprint = compute_fingerprint(Some(sync_token), Some(lut_ms), &entity);
        let comparable = json!({ "Id": entity_id, "Name": "Widget" });
        let comparable_hash = compute_comparable_hash(&comparable, lut_ms);

        observations::upsert_observation(
            &pool,
            &app_id,
            "quickbooks",
            "item",
            entity_id,
            &fingerprint,
            lut_ms,
            &comparable_hash,
            1,
            &entity,
            "cdc",
            false,
        )
        .await
        .expect("cdc upsert");
    }

    // Process a webhook for the same entity and state — same SyncToken, same LUT.
    let mut responses = HashMap::new();
    responses.insert(("item".to_string(), entity_id.to_string()), entity.clone());
    let (base_url, _srv) = start_mock_qbo(responses).await;

    let events = json!([cloud_event(
        "ev-item-1",
        "qbo.item.updated.v1",
        entity_id,
        &realm_id
    )]);
    let body = serde_json::to_vec(&events).expect("serialize");
    QboNormalizer::new_with_base_url(pool.clone(), base_url)
        .normalize(&body, &events, &HashMap::new())
        .await
        .expect("normalize");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Still exactly one observation row (upserted, not duplicated).
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_sync_observations \
         WHERE app_id = $1 AND provider = 'quickbooks' \
           AND entity_type = 'item' AND entity_id = $2",
    )
    .bind(&app_id)
    .bind(entity_id)
    .fetch_one(&pool)
    .await
    .expect("count");

    assert_eq!(
        count.0, 1,
        "CDC + webhook for same state must be one row after upsert"
    );

    // Source channel is refreshed to 'webhook' by the upsert.
    let obs = observations::get_latest_for_entity(&pool, &app_id, "quickbooks", "item", entity_id)
        .await
        .expect("query")
        .expect("row must exist");
    assert_eq!(
        obs.source_channel, "webhook",
        "upsert must refresh source_channel to webhook"
    );

    cleanup(&pool, &app_id, &realm_id).await;
}

#[tokio::test]
#[serial]
async fn webhook_missing_entity_id_skips_observe_gracefully() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let realm_id = unique_realm();
    cleanup(&pool, &app_id, &realm_id).await;
    seed_connection(&pool, &app_id, &realm_id).await;

    let (base_url, _srv) = start_mock_qbo(HashMap::new()).await;
    let normalizer = QboNormalizer::new_with_base_url(pool.clone(), base_url);

    // Event with no intuitentityid.
    let events = json!([{
        "id": "ev-no-eid",
        "type": "qbo.customer.created.v1",
        "time": "2026-04-20T10:00:00Z",
        "intuitaccountid": realm_id,
        "data": {}
    }]);
    let body = serde_json::to_vec(&events).expect("serialize");

    // Must not fail; just skips the observe step.
    let result = normalizer
        .normalize(&body, &events, &HashMap::new())
        .await
        .expect("normalize must succeed");

    assert_eq!(
        result.events_processed, 1,
        "event was processed (ingest + outbox)"
    );

    tokio::time::sleep(Duration::from_millis(100)).await;

    // No observation row written for this app.
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM integrations_sync_observations WHERE app_id = $1")
            .bind(&app_id)
            .fetch_one(&pool)
            .await
            .expect("count");

    assert_eq!(count.0, 0, "no observation for event without entity id");

    cleanup(&pool, &app_id, &realm_id).await;
}

/// Verify that all four QBO delete event types produce tombstone observations
/// without making a network call.
#[tokio::test]
#[serial]
async fn all_delete_event_types_write_tombstones() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let realm_id = unique_realm();
    cleanup(&pool, &app_id, &realm_id).await;
    seed_connection(&pool, &app_id, &realm_id).await;

    // Empty mock server — any fetch attempt would fail.
    let (base_url, _srv) = start_mock_qbo(HashMap::new()).await;

    let cases = [
        ("qbo.customer.deleted.v1", "customer", "c-del"),
        ("qbo.invoice.deleted.v1", "invoice", "i-del"),
        ("qbo.payment.deleted.v1", "payment", "p-del"),
        ("qbo.item.deleted.v1", "item", "it-del"),
    ];

    for (event_type, _obs_entity_type, entity_id) in &cases {
        let ev_id = format!("ev-{}", Uuid::new_v4().simple());
        let events = json!([cloud_event(&ev_id, event_type, entity_id, &realm_id)]);
        let body = serde_json::to_vec(&events).expect("serialize");
        QboNormalizer::new_with_base_url(pool.clone(), base_url.clone())
            .normalize(&body, &events, &HashMap::new())
            .await
            .unwrap_or_else(|e| panic!("normalize failed for {}: {}", event_type, e));
    }

    tokio::time::sleep(Duration::from_millis(300)).await;

    for (_event_type, obs_entity_type, entity_id) in &cases {
        let obs = observations::get_latest_for_entity(
            &pool,
            &app_id,
            "quickbooks",
            obs_entity_type,
            entity_id,
        )
        .await
        .unwrap_or_else(|e| panic!("query failed for {}: {}", obs_entity_type, e))
        .unwrap_or_else(|| panic!("tombstone missing for {}/{}", obs_entity_type, entity_id));

        assert!(
            obs.is_tombstone,
            "{}/{} must be a tombstone",
            obs_entity_type, entity_id
        );
        assert_eq!(obs.source_channel, "webhook");
    }

    cleanup(&pool, &app_id, &realm_id).await;
}

/// Verify routing: delete event types map to *.deleted domain events (not *.synced).
#[test]
fn delete_event_types_map_to_deleted_domain_events() {
    use integrations_rs::domain::webhooks::routing::map_to_domain_event;

    assert_eq!(
        map_to_domain_event("quickbooks", Some("qbo.customer.deleted.v1")),
        Some("party.customer.deleted".to_string())
    );
    assert_eq!(
        map_to_domain_event("quickbooks", Some("qbo.invoice.deleted.v1")),
        Some("ar.invoice.deleted".to_string())
    );
    assert_eq!(
        map_to_domain_event("quickbooks", Some("qbo.payment.deleted.v1")),
        Some("payments.payment.deleted".to_string())
    );
    assert_eq!(
        map_to_domain_event("quickbooks", Some("qbo.item.deleted.v1")),
        Some("inventory.item.deleted".to_string())
    );
}

// ── Detector wiring tests ─────────────────────────────────────────────────────

/// Verify that a webhook observation automatically opens a conflict row when no
/// push attempt marker matches — i.e. run_detector is called by the normalizer
/// and genuine drift is detected without manual invocation.
#[tokio::test]
#[serial]
async fn webhook_observation_auto_opens_conflict_when_no_marker_match() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let realm_id = unique_realm();
    cleanup(&pool, &app_id, &realm_id).await;
    seed_connection(&pool, &app_id, &realm_id).await;

    let entity_id = "cust-detector-auto";
    let sync_token = "tok-no-marker-99";

    let mut responses = HashMap::new();
    responses.insert(
        ("customer".to_string(), entity_id.to_string()),
        json!({
            "Id": entity_id,
            "DisplayName": "Drift Corp",
            "SyncToken": sync_token,
            "MetaData": {
                "LastUpdatedTime": "2026-04-21T10:00:00Z",
                "CreateTime": "2026-04-01T00:00:00Z"
            }
        }),
    );

    let (base_url, _srv) = start_mock_qbo(responses).await;
    let normalizer = QboNormalizer::new_with_base_url(pool.clone(), base_url);

    let events = json!([cloud_event(
        "ev-det-auto",
        "qbo.customer.updated.v1",
        entity_id,
        &realm_id
    )]);
    let body = serde_json::to_vec(&events).expect("serialize");
    normalizer
        .normalize(&body, &events, &HashMap::new())
        .await
        .expect("normalize");

    // Allow async fetch-and-observe + detector to run.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let conflict_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_sync_conflicts \
         WHERE app_id = $1 AND provider = 'quickbooks' \
           AND entity_type = 'customer' AND entity_id = $2",
    )
    .bind(&app_id)
    .bind(entity_id)
    .fetch_one(&pool)
    .await
    .expect("count conflicts");

    assert_eq!(
        conflict_count.0, 1,
        "webhook observation with no marker match must auto-open a conflict row"
    );

    cleanup(&pool, &app_id, &realm_id).await;
}

/// Verify that a webhook observation suppresses the conflict (self-echo) when a
/// succeeded push attempt with a matching sync token exists — the detector is
/// called by the normalizer and correctly recognises the self-echo.
#[tokio::test]
#[serial]
async fn webhook_observation_suppresses_self_echo_when_marker_matches() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let realm_id = unique_realm();
    cleanup(&pool, &app_id, &realm_id).await;
    seed_connection(&pool, &app_id, &realm_id).await;

    let entity_id = "cust-self-echo";
    let sync_token = "tok-echo-matched-7";

    // Seed a succeeded push attempt with the sync token that the webhook will carry.
    sqlx::query(
        r#"
        INSERT INTO integrations_sync_push_attempts (
            app_id, provider, entity_type, entity_id, operation,
            authority_version, request_fingerprint, status,
            result_sync_token, completed_at
        )
        VALUES ($1, 'quickbooks', 'customer', $2, 'update',
                1, 'fp-echo', 'succeeded',
                $3, NOW())
        "#,
    )
    .bind(&app_id)
    .bind(entity_id)
    .bind(sync_token)
    .execute(&pool)
    .await
    .expect("seed succeeded push attempt");

    let mut responses = HashMap::new();
    responses.insert(
        ("customer".to_string(), entity_id.to_string()),
        json!({
            "Id": entity_id,
            "DisplayName": "Echo Corp",
            "SyncToken": sync_token,
            "MetaData": {
                "LastUpdatedTime": "2026-04-21T11:00:00Z",
                "CreateTime": "2026-04-01T00:00:00Z"
            }
        }),
    );

    let (base_url, _srv) = start_mock_qbo(responses).await;
    let normalizer = QboNormalizer::new_with_base_url(pool.clone(), base_url);

    let events = json!([cloud_event(
        "ev-echo-match",
        "qbo.customer.updated.v1",
        entity_id,
        &realm_id
    )]);
    let body = serde_json::to_vec(&events).expect("serialize");
    normalizer
        .normalize(&body, &events, &HashMap::new())
        .await
        .expect("normalize");

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Self-echo: no conflict row must have been created.
    let conflict_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_sync_conflicts \
         WHERE app_id = $1 AND provider = 'quickbooks' \
           AND entity_type = 'customer' AND entity_id = $2",
    )
    .bind(&app_id)
    .bind(entity_id)
    .fetch_one(&pool)
    .await
    .expect("count conflicts");

    assert_eq!(
        conflict_count.0, 0,
        "self-echo must not open a conflict row when a succeeded marker matches"
    );

    cleanup(&pool, &app_id, &realm_id).await;
}
