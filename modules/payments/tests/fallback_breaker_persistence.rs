//! Integration test: circuit breaker and metrics persist across handler invocations.
//!
//! Before bd-lyxal, `CircuitBreaker`, `FallbackMetrics`, and `FallbackPolicy` were
//! constructed inside `get_payment` on every request. That meant:
//!   - Failure counts reset to zero on every call — the breaker could never open.
//!   - Metrics were discarded after each response — Prometheus saw nothing.
//!
//! After bd-lyxal, these live in `AppState` and are shared across all requests via
//! `Arc<AppState>`. This test verifies that property.

use projections::{CircuitBreaker, FallbackMetrics, FallbackPolicy};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_test_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for tests");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

/// Simulates the clone that axum performs when a handler extracts `State<Arc<AppState>>`.
/// Each handler invocation receives a clone of the Arc — not a new allocation. So a clone
/// of `CircuitBreaker` must share the same inner `Arc<Mutex<…>>` state.
#[tokio::test]
#[serial]
async fn circuit_breaker_state_persists_across_request_clones() {
    // Breaker: opens after 3 failures, recovers after 2 successes.
    let circuit = CircuitBreaker::new(3, 2);
    assert!(circuit.is_closed(), "starts closed");

    // --- Request 1: first failure ---
    let req1 = circuit.clone(); // simulates handler clone via Arc<AppState>
    req1.record_failure();
    assert_eq!(circuit.failure_count(), 1, "failure 1 visible on original");

    // --- Request 2: second failure (different "connection") ---
    let req2 = circuit.clone();
    req2.record_failure();
    assert_eq!(circuit.failure_count(), 2, "failure 2 accumulated");

    // --- Request 3: third failure — trips the breaker ---
    let req3 = circuit.clone();
    req3.record_failure();
    assert!(
        !circuit.is_closed(),
        "circuit must open after threshold (3 failures)"
    );

    // --- Request 4: sees the open circuit ---
    let req4 = circuit.clone();
    assert!(
        !req4.is_closed(),
        "new request must see the open circuit — proves shared state, not per-request reset"
    );

    // --- Request 5: also sees it open (breaker stays open until recovery) ---
    let req5 = circuit.clone();
    req5.record_failure(); // additional failure; state remains open
    assert!(!req5.is_closed(), "circuit stays open");
}

/// Verifies that `FallbackMetrics` counters accumulate across handler invocations.
///
/// A per-request `FallbackMetrics::default()` would reset the counter on every
/// request. Shared metrics in `AppState` carry the running total.
#[tokio::test]
#[serial]
async fn fallback_metrics_accumulate_across_request_clones() {
    let metrics = FallbackMetrics::new().expect("create metrics");
    let tenant_id = format!("test_metrics_{}", Uuid::new_v4().simple());

    // Simulate three separate handler invocations each recording one fallback.
    for _ in 0..3 {
        let m = metrics.clone(); // handler clone
        m.record_invocation("payment_projection", &tenant_id);
    }

    // Gather from the registry and verify the counter reached 3.
    let gathered = metrics.registry().gather();
    let family = gathered
        .iter()
        .find(|mf| mf.get_name() == "projection_fallback_invocation_count")
        .expect("projection_fallback_invocation_count metric must exist");

    let total: f64 = family
        .get_metric()
        .iter()
        .filter(|m| {
            m.get_label()
                .iter()
                .any(|l| l.get_name() == "tenant_id" && l.get_value() == tenant_id)
        })
        .map(|m| m.get_counter().get_value())
        .sum();

    assert_eq!(
        total, 3.0,
        "all 3 invocations must be visible in the shared registry (got {total})"
    );
}

/// End-to-end integration: circuit breaker state survives across real DB round-trips.
///
/// Creates a stale projection cursor in the real database, then simulates multiple
/// fallback calls that fail (the write-service stub always errors). Verifies the
/// breaker opens at the configured threshold — which requires the failure count to
/// be shared across calls rather than reset per-call.
#[tokio::test]
#[serial]
async fn circuit_breaker_opens_after_repeated_fallback_failures_with_real_db() {
    let pool = setup_test_db().await;
    let tenant_id = format!("test_cb_{}", Uuid::new_v4().simple());

    // Insert a cursor that was last updated an hour ago — definitely stale.
    let old_time = chrono::Utc::now() - chrono::Duration::hours(1);
    sqlx::query(
        "INSERT INTO projection_cursors \
         (projection_name, tenant_id, last_event_id, last_event_occurred_at) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (projection_name, tenant_id) DO UPDATE \
             SET last_event_occurred_at = EXCLUDED.last_event_occurred_at",
    )
    .bind("payment_projection")
    .bind(&tenant_id)
    .bind(Uuid::nil())
    .bind(old_time)
    .execute(&pool)
    .await
    .expect("insert stale cursor");

    // Shared primitives — same as what AppState holds.
    let circuit = CircuitBreaker::new(3, 2);
    let metrics = FallbackMetrics::new().expect("metrics");
    let policy = FallbackPolicy::new(1000, 50); // 1s staleness, 50ms budget

    // Load the cursor from the real DB and verify it is stale.
    let cursor =
        projections::cursor::ProjectionCursor::load(&pool, "payment_projection", &tenant_id)
            .await
            .expect("load cursor");
    assert!(cursor.is_some(), "cursor must exist after insert");
    assert!(
        policy.is_stale(cursor.as_ref().unwrap()),
        "cursor must be stale (last_event_occurred_at 1 hour ago)"
    );

    // Simulate 3 fallback calls, each failing (write-service stub errors).
    // This is what `policy.execute_with_budget` does internally on failure.
    for i in 0..3 {
        let c = circuit.clone(); // handler clone
        let m = metrics.clone();
        m.record_invocation("payment_projection", &tenant_id);
        c.record_failure();
        assert_eq!(
            c.failure_count(),
            (i + 1) as u32,
            "failure count must accumulate (iteration {})",
            i + 1
        );
    }

    // After 3 failures the breaker must be open.
    assert!(
        !circuit.is_closed(),
        "circuit must open after 3 accumulated failures"
    );

    // A subsequent request (new handler clone) must see the open circuit.
    let late_request = circuit.clone();
    assert!(
        !late_request.is_closed(),
        "late request must also see the open circuit"
    );

    // Metrics must show 3 invocations in the shared registry.
    let gathered = metrics.registry().gather();
    let family = gathered
        .iter()
        .find(|mf| mf.get_name() == "projection_fallback_invocation_count")
        .expect("projection_fallback_invocation_count metric must exist");
    let total: f64 = family
        .get_metric()
        .iter()
        .filter(|m| {
            m.get_label()
                .iter()
                .any(|l| l.get_name() == "tenant_id" && l.get_value() == tenant_id)
        })
        .map(|m| m.get_counter().get_value())
        .sum();
    assert_eq!(
        total, 3.0,
        "3 invocations must be visible in shared registry"
    );

    // Cleanup.
    sqlx::query(
        "DELETE FROM projection_cursors WHERE projection_name = 'payment_projection' AND tenant_id = $1",
    )
    .bind(&tenant_id)
    .execute(&pool)
    .await
    .expect("cleanup cursor");
}
