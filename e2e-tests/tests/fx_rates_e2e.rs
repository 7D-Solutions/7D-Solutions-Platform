/// FX Rate Store E2E Tests (Phase 23a, bd-104)
///
/// Verifies:
/// 1. POST /api/gl/fx-rates creates a rate and returns rate_id
/// 2. Duplicate idempotency_key insert is a no-op (created=false)
/// 3. GET /api/gl/fx-rates/latest returns correct latest-as-of rate
/// 4. fx.rate_updated outbox event is emitted atomically on insert
/// 5. Rate queries are deterministic for time-versioned snapshots
///
/// Run with: cargo test -p e2e-tests fx_rates_e2e -- --nocapture
mod common;

use chrono::{Duration, Utc};
use jsonwebtoken::EncodingKey;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::process::Command;
use std::time::Duration as StdDuration;
use uuid::Uuid;

// ============================================================================
// API Types
// ============================================================================

#[derive(Debug, Serialize)]
struct CreateFxRateRequest {
    tenant_id: String,
    base_currency: String,
    quote_currency: String,
    rate: f64,
    effective_at: String,
    source: String,
    idempotency_key: String,
}

#[derive(Debug, Deserialize)]
struct CreateFxRateResponse {
    rate_id: Uuid,
    created: bool,
}

#[derive(Debug, Deserialize)]
struct FxRateResponse {
    id: Uuid,
    tenant_id: String,
    base_currency: String,
    quote_currency: String,
    rate: f64,
    inverse_rate: f64,
    effective_at: String,
    source: String,
    created_at: String,
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    error: String,
}

// ============================================================================
// Infrastructure
// ============================================================================

fn gl_base_url() -> String {
    std::env::var("GL_BASE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string())
}

async fn connect_gl_db() -> PgPool {
    common::get_gl_pool().await
}

async fn wait_for_service_healthy(container: &str, timeout_secs: u64) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + StdDuration::from_secs(timeout_secs);
    loop {
        let output = Command::new("docker")
            .args(["inspect", "--format", "{{.State.Health.Status}}", container])
            .output()
            .map_err(|e| format!("Failed to inspect container: {}", e))?;
        let health = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if health == "healthy" {
            return Ok(());
        }
        if tokio::time::Instant::now() > deadline {
            return Err(format!(
                "Timeout waiting for {} to be healthy (status: {})",
                container, health
            ));
        }
        tokio::time::sleep(StdDuration::from_millis(500)).await;
    }
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn test_create_fx_rate_and_query_latest() {
    wait_for_service_healthy("7d-gl", 10)
        .await
        .expect("GL service not healthy");

    let key = match common::dev_private_key() {
        Some(k) => k,
        None => {
            eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
            return;
        }
    };

    let client = reqwest::Client::new();
    let base_url = gl_base_url();
    let tenant_id = format!("test-fx-{}", Uuid::new_v4());
    let jwt = common::make_service_jwt(&key, &tenant_id, &["gl.post", "gl.read"]);

    // ── Step 1: Create an EUR/USD rate ──────────────────────────────────────
    let effective_t1 = Utc::now() - Duration::hours(2);
    let idem_key_1 = format!("fx-test-{}", Uuid::new_v4());

    let resp = client
        .post(format!("{}/api/gl/fx-rates", base_url))
        .bearer_auth(&jwt)
        .json(&serde_json::json!({
            "tenant_id": tenant_id,
            "base_currency": "EUR",
            "quote_currency": "USD",
            "rate": 1.085,
            "effective_at": effective_t1.to_rfc3339(),
            "source": "ecb",
            "idempotency_key": idem_key_1
        }))
        .send()
        .await
        .expect("Failed to POST fx rate");

    assert_eq!(resp.status(), 200, "Expected 200, got {}", resp.status());
    let body: CreateFxRateResponse = resp.json().await.expect("parse response");
    assert!(body.created, "First insert should be created=true");
    let rate_id_1 = body.rate_id;
    println!("Created rate 1: {}", rate_id_1);

    // ── Step 2: Duplicate idempotency_key → no-op ──────────────────────────
    let resp = client
        .post(format!("{}/api/gl/fx-rates", base_url))
        .bearer_auth(&jwt)
        .json(&serde_json::json!({
            "tenant_id": tenant_id,
            "base_currency": "EUR",
            "quote_currency": "USD",
            "rate": 1.090,  // different rate, same key
            "effective_at": effective_t1.to_rfc3339(),
            "source": "ecb",
            "idempotency_key": idem_key_1
        }))
        .send()
        .await
        .expect("Failed to POST duplicate fx rate");

    assert_eq!(resp.status(), 200);
    let body: CreateFxRateResponse = resp.json().await.expect("parse response");
    assert!(!body.created, "Duplicate key should be created=false");
    println!("Duplicate insert correctly returned created=false");

    // ── Step 3: Create a newer rate for the same pair ──────────────────────
    let effective_t2 = Utc::now() - Duration::hours(1);
    let idem_key_2 = format!("fx-test-{}", Uuid::new_v4());

    let resp = client
        .post(format!("{}/api/gl/fx-rates", base_url))
        .bearer_auth(&jwt)
        .json(&serde_json::json!({
            "tenant_id": tenant_id,
            "base_currency": "EUR",
            "quote_currency": "USD",
            "rate": 1.092,
            "effective_at": effective_t2.to_rfc3339(),
            "source": "ecb",
            "idempotency_key": idem_key_2
        }))
        .send()
        .await
        .expect("Failed to POST second fx rate");

    assert_eq!(resp.status(), 200);
    let body: CreateFxRateResponse = resp.json().await.expect("parse response");
    assert!(body.created);
    let rate_id_2 = body.rate_id;
    println!("Created rate 2: {}", rate_id_2);

    // ── Step 4: GET latest (as-of now) should return the newer rate ────────
    let resp = client
        .get(format!(
            "{}/api/gl/fx-rates/latest?tenant_id={}&base_currency=EUR&quote_currency=USD",
            base_url, tenant_id
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("Failed to GET latest rate");

    assert_eq!(resp.status(), 200);
    let latest: FxRateResponse = resp.json().await.expect("parse latest rate");
    assert_eq!(latest.id, rate_id_2, "Should return the newer rate");
    assert!((latest.rate - 1.092).abs() < 0.001);
    assert_eq!(latest.base_currency, "EUR");
    assert_eq!(latest.quote_currency, "USD");
    assert_eq!(latest.source, "ecb");
    println!(
        "Latest rate: {}/{} = {} (effective {})",
        latest.base_currency, latest.quote_currency, latest.rate, latest.effective_at
    );

    // ── Step 5: GET latest as-of between t1 and t2 → should return rate 1 ─
    let as_of_between = effective_t1 + Duration::minutes(30);
    let resp = client
        .get(format!("{}/api/gl/fx-rates/latest", base_url))
        .bearer_auth(&jwt)
        .query(&[
            ("tenant_id", tenant_id.as_str()),
            ("base_currency", "EUR"),
            ("quote_currency", "USD"),
            ("as_of", &as_of_between.to_rfc3339()),
        ])
        .send()
        .await
        .expect("Failed to GET rate as-of");

    assert_eq!(resp.status(), 200);
    let historical: FxRateResponse = resp.json().await.expect("parse historical rate");
    assert_eq!(
        historical.id, rate_id_1,
        "As-of query between t1 and t2 should return rate 1"
    );
    assert!((historical.rate - 1.085).abs() < 0.001);
    println!(
        "Historical as-of query returned correct rate: {}",
        historical.rate
    );

    // ── Step 6: GET latest for non-existent pair → 404 ─────────────────────
    let resp = client
        .get(format!(
            "{}/api/gl/fx-rates/latest?tenant_id={}&base_currency=JPY&quote_currency=CHF",
            base_url, tenant_id
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("Failed to GET non-existent rate");

    assert_eq!(resp.status(), 404, "Non-existent pair should return 404");
    println!("Non-existent pair correctly returned 404");

    println!("PASS: create + idempotency + latest-as-of queries");
}

#[tokio::test]
async fn test_fx_rate_outbox_event_emitted() {
    wait_for_service_healthy("7d-gl", 10)
        .await
        .expect("GL service not healthy");

    let key = match common::dev_private_key() {
        Some(k) => k,
        None => {
            eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
            return;
        }
    };

    let gl_pool = connect_gl_db().await;
    let client = reqwest::Client::new();
    let base_url = gl_base_url();
    let tenant_id = format!("test-fx-outbox-{}", Uuid::new_v4());
    let jwt = common::make_service_jwt(&key, &tenant_id, &["gl.post", "gl.read"]);
    let idem_key = format!("fx-outbox-{}", Uuid::new_v4());

    // Create a rate
    let resp = client
        .post(format!("{}/api/gl/fx-rates", base_url))
        .bearer_auth(&jwt)
        .json(&serde_json::json!({
            "tenant_id": tenant_id,
            "base_currency": "GBP",
            "quote_currency": "EUR",
            "rate": 1.172,
            "effective_at": Utc::now().to_rfc3339(),
            "source": "manual",
            "idempotency_key": idem_key
        }))
        .send()
        .await
        .expect("Failed to POST fx rate");

    assert_eq!(resp.status(), 200);
    let body: CreateFxRateResponse = resp.json().await.expect("parse response");
    assert!(body.created);

    // Verify outbox event exists for this rate
    let row = sqlx::query(
        r#"
        SELECT event_type, aggregate_type, aggregate_id, mutation_class, payload
        FROM events_outbox
        WHERE aggregate_id = $1 AND event_type = 'fx.rate_updated'
        "#,
    )
    .bind(body.rate_id.to_string())
    .fetch_optional(&gl_pool)
    .await
    .expect("Failed to query outbox");

    let row = row.expect("Outbox event not found for fx.rate_updated");
    let event_type: String = row.get("event_type");
    let aggregate_type: String = row.get("aggregate_type");
    let mutation_class: Option<String> = row.get("mutation_class");
    let payload: serde_json::Value = row.get("payload");

    assert_eq!(event_type, "fx.rate_updated");
    assert_eq!(aggregate_type, "fx_rate");
    assert_eq!(mutation_class.as_deref(), Some("DATA_MUTATION"));
    assert_eq!(payload["base_currency"], "GBP");
    assert_eq!(payload["quote_currency"], "EUR");
    assert!((payload["rate"].as_f64().unwrap() - 1.172).abs() < 0.001);

    println!("PASS: fx.rate_updated outbox event emitted atomically");
}

#[tokio::test]
async fn test_fx_rate_validation_rejects_bad_input() {
    wait_for_service_healthy("7d-gl", 10)
        .await
        .expect("GL service not healthy");

    let key = match common::dev_private_key() {
        Some(k) => k,
        None => {
            eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
            return;
        }
    };

    let client = reqwest::Client::new();
    let base_url = gl_base_url();
    let tenant_id = format!("test-fx-val-{}", Uuid::new_v4());
    let jwt = common::make_service_jwt(&key, &tenant_id, &["gl.post", "gl.read"]);

    // Same base and quote currency should fail
    let resp = client
        .post(format!("{}/api/gl/fx-rates", base_url))
        .bearer_auth(&jwt)
        .json(&serde_json::json!({
            "tenant_id": tenant_id,
            "base_currency": "USD",
            "quote_currency": "USD",
            "rate": 1.0,
            "effective_at": Utc::now().to_rfc3339(),
            "source": "manual",
            "idempotency_key": format!("fx-val-{}", Uuid::new_v4())
        }))
        .send()
        .await
        .expect("Failed to POST");

    assert_eq!(resp.status(), 400, "Same currencies should be rejected");

    // Negative rate should fail
    let resp = client
        .post(format!("{}/api/gl/fx-rates", base_url))
        .bearer_auth(&jwt)
        .json(&serde_json::json!({
            "tenant_id": tenant_id,
            "base_currency": "EUR",
            "quote_currency": "USD",
            "rate": -1.0,
            "effective_at": Utc::now().to_rfc3339(),
            "source": "manual",
            "idempotency_key": format!("fx-val-{}", Uuid::new_v4())
        }))
        .send()
        .await
        .expect("Failed to POST");

    assert_eq!(resp.status(), 400, "Negative rate should be rejected");

    println!("PASS: validation rejects bad inputs");
}

#[tokio::test]
async fn test_fx_rate_db_append_only() {
    wait_for_service_healthy("7d-gl", 10)
        .await
        .expect("GL service not healthy");

    let key = match common::dev_private_key() {
        Some(k) => k,
        None => {
            eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
            return;
        }
    };

    let gl_pool = connect_gl_db().await;
    let client = reqwest::Client::new();
    let base_url = gl_base_url();
    let tenant_id = format!("test-fx-append-{}", Uuid::new_v4());
    let jwt = common::make_service_jwt(&key, &tenant_id, &["gl.post", "gl.read"]);

    // Insert 3 rates for the same pair at different times
    let mut rate_ids = Vec::new();
    for i in 0..3 {
        let effective = Utc::now() - Duration::hours(3 - i);
        let resp = client
            .post(format!("{}/api/gl/fx-rates", base_url))
            .bearer_auth(&jwt)
            .json(&serde_json::json!({
                "tenant_id": tenant_id,
                "base_currency": "EUR",
                "quote_currency": "JPY",
                "rate": 160.0 + (i as f64),
                "effective_at": effective.to_rfc3339(),
                "source": "test",
                "idempotency_key": format!("fx-append-{}-{}", tenant_id, i)
            }))
            .send()
            .await
            .expect("Failed to POST");

        assert_eq!(resp.status(), 200);
        let body: CreateFxRateResponse = resp.json().await.expect("parse");
        assert!(body.created);
        rate_ids.push(body.rate_id);
    }

    // All 3 should exist in the DB (append-only, no overwrites)
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM fx_rates WHERE tenant_id = $1 AND base_currency = 'EUR' AND quote_currency = 'JPY'",
    )
    .bind(&tenant_id)
    .fetch_one(&gl_pool)
    .await
    .expect("count query");

    assert_eq!(count, 3, "All 3 rate snapshots should exist (append-only)");

    println!("PASS: all rate snapshots preserved (append-only)");
}
