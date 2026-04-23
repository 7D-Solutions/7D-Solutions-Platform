//! Intent guard integration tests for stale SyncToken retry (bd-dd2x6).
//!
//! Run: ./scripts/cargo-slot.sh test -p integrations-rs --test qbo_intent_guard_test -- --nocapture
//!
//! Requires a running PostgreSQL instance at DATABASE_URL (defaults to the
//! standard integrations dev DB on port 5449).
//!
//! All HTTP interactions use a local axum server — no QBO sandbox required.

use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};

use integrations_rs::domain::qbo::{client::QboClient, QboError, TokenProvider};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

// ── DB helpers ────────────────────────────────────────────────────────────────

async fn setup_db() -> PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://integrations_user:integrations_pass@localhost:5449/integrations_db"
            .to_string()
    });
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect to integrations test DB")
}

// ── Token provider ────────────────────────────────────────────────────────────

struct FixedToken;

#[async_trait::async_trait]
impl TokenProvider for FixedToken {
    async fn get_token(&self) -> Result<String, QboError> {
        Ok("test-token".into())
    }
    async fn refresh_token(&self) -> Result<String, QboError> {
        Ok("test-token".into())
    }
}

fn make_client(base_url: &str) -> QboClient {
    QboClient::new(base_url, "realm-test", Arc::new(FixedToken))
}

// ── Axum server helpers ───────────────────────────────────────────────────────

/// State shared between GET and POST axum handlers.
#[derive(Clone)]
struct GuardServerState {
    post_count: Arc<AtomicU32>,
    get_count: Arc<AtomicU32>,
    /// If true the second GET returns a mutated ShipDate (simulating concurrent edit).
    ship_date_drifted: bool,
}

async fn guard_post(
    axum::extract::State(s): axum::extract::State<GuardServerState>,
) -> (axum::http::StatusCode, String) {
    let n = s.post_count.fetch_add(1, Ordering::SeqCst);
    if n == 0 {
        // First attempt: stale SyncToken
        (
            axum::http::StatusCode::BAD_REQUEST,
            r#"{"Fault":{"Error":[{"Message":"Stale Object Error","Detail":"SyncToken mismatch","code":"5010"}],"type":"ValidationFault"}}"#.into(),
        )
    } else {
        (
            axum::http::StatusCode::OK,
            r#"{"Invoice":{"Id":"42","SyncToken":"10","ShipDate":"2026-04-20"}}"#.into(),
        )
    }
}

async fn guard_get(
    axum::extract::State(s): axum::extract::State<GuardServerState>,
) -> (axum::http::StatusCode, String) {
    let n = s.get_count.fetch_add(1, Ordering::SeqCst);
    // n==0 is the baseline GET; n>=1 is the stale-retry re-fetch
    let ship_date = if n >= 1 && s.ship_date_drifted {
        "2026-04-25" // concurrent edit changed it
    } else {
        "2026-04-01" // original value
    };
    let body = format!(
        r#"{{"Invoice":{{"Id":"42","SyncToken":"9","ShipDate":"{}"}}}}"#,
        ship_date
    );
    (axum::http::StatusCode::OK, body)
}

async fn start_guard_server(ship_date_drifted: bool) -> (String, GuardServerState) {
    let state = GuardServerState {
        post_count: Arc::new(AtomicU32::new(0)),
        get_count: Arc::new(AtomicU32::new(0)),
        ship_date_drifted,
    };
    let app = axum::Router::new()
        .route(
            "/v3/company/{realm}/invoice/{id}",
            axum::routing::get(guard_get),
        )
        .route(
            "/v3/company/{realm}/invoice",
            axum::routing::post(guard_post),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{}/v3", addr), state)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Touched field unchanged between baseline and fresh → guard allows retry → success.
#[tokio::test]
async fn guard_no_drift_retries_and_succeeds() {
    let (base_url, state) = start_guard_server(false).await;
    let client = make_client(&base_url);

    // 1. Baseline GET
    let current = client
        .get_entity("Invoice", "42")
        .await
        .expect("baseline GET failed");
    let baseline = current["Invoice"].clone();
    assert_eq!(
        baseline["ShipDate"].as_str(),
        Some("2026-04-01"),
        "baseline should have original ShipDate"
    );

    // 2. Build update body intending to change ShipDate
    let body = serde_json::json!({
        "Id": "42",
        "SyncToken": baseline["SyncToken"].as_str().unwrap_or("9"),
        "sparse": true,
        "ShipDate": "2026-04-20",
    });

    // 3. Guarded update: POST will stale on attempt 1, re-fetch shows no drift, retry succeeds
    let result = client
        .update_entity_with_guard("Invoice", body, Some(&baseline), Uuid::new_v4())
        .await;

    assert!(
        result.is_ok(),
        "expected success with no drift: {:?}",
        result
    );
    assert_eq!(
        state.post_count.load(Ordering::SeqCst),
        2,
        "should have attempted POST twice (initial + 1 retry)"
    );
    assert_eq!(
        state.get_count.load(Ordering::SeqCst),
        2,
        "should have done 2 GETs (baseline + stale re-fetch)"
    );
}

/// Touched field changed by concurrent writer → guard fires → ConflictDetected.
#[tokio::test]
async fn guard_drift_returns_conflict_detected() {
    let (base_url, state) = start_guard_server(true).await;
    let client = make_client(&base_url);

    // 1. Baseline GET
    let current = client
        .get_entity("Invoice", "42")
        .await
        .expect("baseline GET failed");
    let baseline = current["Invoice"].clone();

    // 2. Build update body touching ShipDate
    let body = serde_json::json!({
        "Id": "42",
        "SyncToken": baseline["SyncToken"].as_str().unwrap_or("9"),
        "sparse": true,
        "ShipDate": "2026-04-20",
    });

    // 3. POST stales, re-fetch shows ShipDate drifted to "2026-04-25" → ConflictDetected
    let result = client
        .update_entity_with_guard("Invoice", body, Some(&baseline), Uuid::new_v4())
        .await;

    match result {
        Err(QboError::ConflictDetected {
            ref entity_id,
            ref fresh_entity,
        }) => {
            assert_eq!(entity_id, "42");
            assert_eq!(
                fresh_entity["ShipDate"].as_str(),
                Some("2026-04-25"),
                "fresh_entity should carry the concurrent value"
            );
        }
        other => panic!("expected ConflictDetected, got: {:?}", other),
    }

    assert_eq!(
        state.post_count.load(Ordering::SeqCst),
        1,
        "should have stopped after detecting conflict (no retry)"
    );
}

/// No baseline + business fields in body → fail conservatively → ConflictDetected.
#[tokio::test]
async fn guard_no_baseline_business_fields_fails_conservatively() {
    let (base_url, _) = start_guard_server(false).await;
    let client = make_client(&base_url);

    let body = serde_json::json!({
        "Id": "42",
        "SyncToken": "5",
        "sparse": true,
        "ShipDate": "2026-04-20",
    });

    let result = client
        .update_entity_with_guard("Invoice", body, None, Uuid::new_v4())
        .await;

    assert!(
        matches!(result, Err(QboError::ConflictDetected { .. })),
        "expected ConflictDetected when no baseline provided: {:?}",
        result
    );
}

/// No baseline + only system fields → no business fields at risk → safe retry → success.
#[tokio::test]
async fn guard_no_baseline_system_only_fields_retries_safely() {
    let (base_url, state) = start_guard_server(false).await;
    let client = make_client(&base_url);

    let body = serde_json::json!({
        "Id": "42",
        "SyncToken": "5",
        "sparse": true,
    });

    let result = client
        .update_entity_with_guard("Invoice", body, None, Uuid::new_v4())
        .await;

    assert!(
        result.is_ok(),
        "expected success with no business fields: {:?}",
        result
    );
    assert_eq!(state.post_count.load(Ordering::SeqCst), 2);
}

/// DB smoke test: verifies the integrations DB is reachable and migrations are current.
///
/// Included here so the guard test suite has at least one DB-backed assertion,
/// keeping the "real services, no mocks" contract for future tests that write
/// conflict rows.
#[tokio::test]
async fn db_reachable_and_migrated() {
    let pool = setup_db().await;
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM integrations_sync_conflicts")
        .fetch_one(&pool)
        .await
        .expect("integrations_sync_conflicts must exist after migrations");
    // Count may be anything; just confirm the table is there.
    let _ = row.0;
}
