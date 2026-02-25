//! Integration tests for GET /api/reporting/forecast endpoint (bd-22lo).
//!
//! Tests exercise the probabilistic cash forecast through the HTTP stack.

mod helpers;

use axum::{body::Body, http::Request};
use helpers::{
    body_json, build_test_app, seed_open_invoice, seed_payment_history,
    setup_db, unique_tenant,
};
use serial_test::serial;
use tower::ServiceExt;

// ── Empty state ─────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn forecast_empty_tenant_returns_200_with_empty_results() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let app = build_test_app(pool);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/forecast")
                .header("x-tenant-id", tid.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let json = body_json(resp).await;
    assert!(json["results"].as_array().unwrap().is_empty());
}

// ── Default horizons ────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn forecast_default_horizons_are_7_14_30_60_90() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tid_str = tid.to_string();
    let app = build_test_app(pool.clone());

    // Seed enough history for profile + one open invoice
    for (i, days) in [10, 20, 30].iter().enumerate() {
        seed_payment_history(
            &pool, &tid_str, "cust-h", &format!("hist-h{i}"),
            "USD", 10000, *days,
        )
        .await;
    }
    seed_open_invoice(&pool, &tid_str, "open-h1", "cust-h", "USD", 50000, 5).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/forecast")
                .header("x-tenant-id", tid_str)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let json = body_json(resp).await;
    let results = json["results"].as_array().unwrap();
    assert!(!results.is_empty(), "Should have at least one currency group");

    let horizons = results[0]["horizons"].as_array().unwrap();
    let days: Vec<u64> = horizons
        .iter()
        .map(|h| h["days"].as_u64().unwrap())
        .collect();
    assert_eq!(days, vec![7, 14, 30, 60, 90]);
}

// ── Custom horizons ─────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn forecast_custom_horizons_respected() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tid_str = tid.to_string();
    let app = build_test_app(pool.clone());

    for (i, days) in [10, 20, 30].iter().enumerate() {
        seed_payment_history(
            &pool, &tid_str, "cust-c", &format!("hist-c{i}"),
            "USD", 10000, *days,
        )
        .await;
    }
    seed_open_invoice(&pool, &tid_str, "open-c1", "cust-c", "USD", 40000, 5).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/forecast?horizons=7,30")
                .header("x-tenant-id", tid_str)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let json = body_json(resp).await;
    let horizons = json["results"][0]["horizons"].as_array().unwrap();
    let days: Vec<u64> = horizons.iter().map(|h| h["days"].as_u64().unwrap()).collect();
    assert_eq!(days, vec![7, 30]);
}

// ── Invalid horizons ────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn forecast_invalid_horizons_returns_400() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let app = build_test_app(pool);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/forecast?horizons=abc")
                .header("x-tenant-id", tid.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
}

// ── Expected cents computation ──────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn forecast_with_profile_computes_expected_cents() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tid_str = tid.to_string();
    let app = build_test_app(pool.clone());

    // Profile: customer pays in [10, 20, 30] days
    for (i, days) in [10, 20, 30].iter().enumerate() {
        seed_payment_history(
            &pool, &tid_str, "cust-e", &format!("hist-e{i}"),
            "USD", 10000, *days,
        )
        .await;
    }

    // Open invoice: 5 days old, $500
    seed_open_invoice(&pool, &tid_str, "open-e1", "cust-e", "USD", 50000, 5).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/forecast?horizons=30")
                .header("x-tenant-id", tid_str)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let json = body_json(resp).await;
    let h30 = &json["results"][0]["horizons"][0];
    assert_eq!(h30["days"].as_u64().unwrap(), 30);
    // P(30|age=5): F(35)=3/3=1.0, F(5)=0/3=0 → P=1.0 → expected = 50000
    assert_eq!(h30["expected_cents"].as_i64().unwrap(), 50000);
}

// ── At-risk flagging ────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn forecast_at_risk_flagged_for_overdue() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tid_str = tid.to_string();
    let app = build_test_app(pool.clone());

    // Profile: very fast payer [5, 6, 7] days
    for (i, days) in [5, 6, 7].iter().enumerate() {
        seed_payment_history(
            &pool, &tid_str, "cust-risk", &format!("hist-r{i}"),
            "USD", 10000, *days,
        )
        .await;
    }

    // Invoice is 10 days old (way past all observations)
    seed_open_invoice(&pool, &tid_str, "open-r1", "cust-risk", "USD", 80000, 10).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/forecast?horizons=30")
                .header("x-tenant-id", tid_str)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let json = body_json(resp).await;
    let at_risk = json["results"][0]["at_risk"].as_array().unwrap();
    assert_eq!(at_risk.len(), 1);
    assert_eq!(at_risk[0]["invoice_id"], "open-r1");
}

// ── Multi-currency grouping ─────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn forecast_multi_currency_groups() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tid_str = tid.to_string();
    let app = build_test_app(pool.clone());

    // USD profile
    for (i, days) in [10, 20, 30].iter().enumerate() {
        seed_payment_history(
            &pool, &tid_str, "cust-mc", &format!("hist-mc-usd-{i}"),
            "USD", 10000, *days,
        )
        .await;
    }
    // EUR profile
    for (i, days) in [15, 25, 35].iter().enumerate() {
        seed_payment_history(
            &pool, &tid_str, "cust-mc", &format!("hist-mc-eur-{i}"),
            "EUR", 10000, *days,
        )
        .await;
    }

    seed_open_invoice(&pool, &tid_str, "open-mc-usd", "cust-mc", "USD", 50000, 0).await;
    seed_open_invoice(&pool, &tid_str, "open-mc-eur", "cust-mc", "EUR", 60000, 0).await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/forecast?horizons=30")
                .header("x-tenant-id", tid_str)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let json = body_json(resp).await;
    let results = json["results"].as_array().unwrap();
    assert_eq!(results.len(), 2, "Should have USD and EUR groups");
    let currencies: Vec<&str> = results.iter().map(|r| r["currency"].as_str().unwrap()).collect();
    assert!(currencies.contains(&"USD"));
    assert!(currencies.contains(&"EUR"));
}

// ── Auth ────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn forecast_no_auth_returns_401() {
    let pool = setup_db().await;
    let app = build_test_app(pool);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/forecast")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 401);
}

// ── Tenant isolation ────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn forecast_tenant_isolation() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();
    let app = build_test_app(pool.clone());

    // Seed data only for tenant A
    for (i, days) in [10, 20, 30].iter().enumerate() {
        seed_payment_history(
            &pool, &tid_a.to_string(), "cust-iso", &format!("hist-iso{i}"),
            "USD", 10000, *days,
        )
        .await;
    }
    seed_open_invoice(&pool, &tid_a.to_string(), "open-iso", "cust-iso", "USD", 50000, 0).await;

    // Tenant A: has results
    let resp_a = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/reporting/forecast?horizons=30")
                .header("x-tenant-id", tid_a.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let json_a = body_json(resp_a).await;
    assert!(!json_a["results"].as_array().unwrap().is_empty());

    // Tenant B: empty
    let resp_b = app
        .oneshot(
            Request::builder()
                .uri("/api/reporting/forecast?horizons=30")
                .header("x-tenant-id", tid_b.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let json_b = body_json(resp_b).await;
    assert!(json_b["results"].as_array().unwrap().is_empty());
}
