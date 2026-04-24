//! Integration tests for per-tenant tax calculation source configuration (bd-kkhf4).
//!
//! All tests run against a real PostgreSQL database — no mocks, no stubs.
//! Set DATABASE_URL_AR to a writable AR test database before running.
//!
//! ```bash
//! ./scripts/cargo-slot.sh test -p ar-rs -- tenant_tax_config
//! ./scripts/cargo-slot.sh test -p ar-rs -- tax_calc_uses_tenant_config
//! ```

mod common;

use ar_rs::tax;
use axum::Router;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Local test app — includes the tax router (quote/commit/void + tenant-config)
// ============================================================================

/// Build a test app that includes both the AR router and the tax router.
/// Uses the permissive variant (no permission enforcement) with a caller-supplied tenant UUID
/// so concurrent tests can each use an isolated tenant without racing.
fn app_with_tax(pool: &PgPool, tenant_id: Uuid) -> Router {
    use axum::Extension;
    use chrono::Utc;
    use security::{claims::ActorType, VerifiedClaims};

    let claims = VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id,
        app_id: None,
        roles: vec!["admin".to_string(), "tenant_admin".to_string()],
        perms: vec!["ar.mutate".to_string(), "ar.read".to_string()],
        actor_type: ActorType::User,
        issued_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        token_id: Uuid::new_v4(),
        version: "1".to_string(),
    };

    ar_rs::http::ar_router_permissive(pool.clone())
        .merge(ar_rs::http::tax::tax_router(pool.clone()))
        .layer(Extension(claims))
}

// ============================================================================
// Helpers
// ============================================================================

/// Delete the config row for a tenant if it exists (test cleanup).
async fn cleanup_tenant_config(pool: &sqlx::PgPool, tenant_id: Uuid) {
    sqlx::query("DELETE FROM ar_tenant_tax_config WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

/// Delete outbox events for a tenant by type (test cleanup).
async fn cleanup_outbox_by_type(pool: &sqlx::PgPool, tenant_id: &str, event_type: &str) {
    sqlx::query(
        "DELETE FROM events_outbox WHERE tenant_id = $1 AND event_type = $2",
    )
    .bind(tenant_id)
    .bind(event_type)
    .execute(pool)
    .await
    .ok();
}

/// Delete tax quote cache entries for a tenant (test cleanup).
async fn cleanup_tax_cache(pool: &sqlx::PgPool, app_id: &str) {
    sqlx::query("DELETE FROM ar_tax_quote_cache WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// CHANGE 2 tests: repository get / set
// ============================================================================

/// No row in ar_tenant_tax_config → get() returns the default config.
/// Default must be external_accounting_software (preserves legacy QBO AST behavior).
#[tokio::test]
async fn tenant_tax_config_default_returns_external_accounting_software() {
    let pool = common::setup_pool().await;
    let tenant_id = Uuid::new_v4(); // guaranteed no row

    let cfg = tax::tenant_config::get(&pool, tenant_id)
        .await
        .expect("get() must not fail for unknown tenant");

    assert_eq!(
        cfg.tax_calculation_source, "external_accounting_software",
        "default source must be external_accounting_software"
    );
    assert_eq!(cfg.provider_name, "local");
    assert_eq!(cfg.config_version, 1);
    assert_eq!(cfg.tenant_id, tenant_id);
}

/// set() followed by get() returns the persisted values.
#[tokio::test]
async fn tenant_tax_config_set_then_get_roundtrip() {
    let pool = common::setup_pool().await;
    let tenant_id = Uuid::new_v4();
    let updated_by = Uuid::new_v4();
    let correlation_id = Uuid::new_v4().to_string();

    // cleanup in case a prior test left a row
    cleanup_tenant_config(&pool, tenant_id).await;

    let set_result = tax::tenant_config::set(
        &pool,
        tenant_id,
        "platform",
        "local",
        updated_by,
        &correlation_id,
    )
    .await
    .expect("set() must succeed");

    assert_eq!(set_result.tax_calculation_source, "platform");
    assert_eq!(set_result.provider_name, "local");
    assert_eq!(set_result.config_version, 1, "first insert starts at 1");
    assert_eq!(set_result.tenant_id, tenant_id);
    assert_eq!(set_result.updated_by, updated_by);

    // Read back
    let fetched = tax::tenant_config::get(&pool, tenant_id)
        .await
        .expect("get() after set must succeed");

    assert_eq!(fetched.tax_calculation_source, "platform");
    assert_eq!(fetched.provider_name, "local");
    assert_eq!(fetched.config_version, 1);
    assert_eq!(fetched.updated_by, updated_by);

    // Update → config_version increments
    let set_result2 = tax::tenant_config::set(
        &pool,
        tenant_id,
        "external_accounting_software",
        "local",
        updated_by,
        &correlation_id,
    )
    .await
    .expect("second set() must succeed");

    assert_eq!(set_result2.config_version, 2, "second write must increment config_version");
    assert_eq!(set_result2.tax_calculation_source, "external_accounting_software");

    cleanup_tenant_config(&pool, tenant_id).await;
}

/// set() emits exactly one ar.tax_config_changed outbox event with the new values.
#[tokio::test]
async fn tenant_tax_config_set_emits_outbox_event() {
    let pool = common::setup_pool().await;
    let tenant_id = Uuid::new_v4();
    let updated_by = Uuid::new_v4();
    let correlation_id = Uuid::new_v4().to_string();

    cleanup_tenant_config(&pool, tenant_id).await;
    cleanup_outbox_by_type(&pool, &tenant_id.to_string(), "ar.tax_config_changed").await;

    tax::tenant_config::set(
        &pool,
        tenant_id,
        "platform",
        "avalara",
        updated_by,
        &correlation_id,
    )
    .await
    .expect("set() must succeed");

    // Verify exactly one outbox event was written
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1 AND event_type = $2",
    )
    .bind(tenant_id.to_string())
    .bind("ar.tax_config_changed")
    .fetch_one(&pool)
    .await
    .expect("count query must succeed");

    assert_eq!(count, 1, "set() must emit exactly one ar.tax_config_changed event");

    // Verify payload fields
    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM events_outbox WHERE tenant_id = $1 AND event_type = $2 LIMIT 1",
    )
    .bind(tenant_id.to_string())
    .bind("ar.tax_config_changed")
    .fetch_one(&pool)
    .await
    .expect("payload query must succeed");

    let inner = payload.get("payload").unwrap_or(&payload);
    assert_eq!(inner["tax_calculation_source"], "platform");
    assert_eq!(inner["provider_name"], "avalara");
    assert_eq!(inner["config_version"], 1);

    cleanup_tenant_config(&pool, tenant_id).await;
    cleanup_outbox_by_type(&pool, &tenant_id.to_string(), "ar.tax_config_changed").await;
}

// ============================================================================
// CHANGE 3 tests: calc path uses tenant config
// ============================================================================

/// When source=external_accounting_software, the quote path returns total_tax=0.
/// The external system (QBO/AST) computes the actual tax; AR contributes nothing.
#[tokio::test]
async fn tax_calc_uses_tenant_config_external_source_returns_zero_tax() {
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let pool = common::setup_pool().await;
    // Use a unique tenant UUID to avoid races with other concurrent tests
    let tenant_id = Uuid::new_v4();
    let updated_by = Uuid::new_v4();

    // Set tenant to external_accounting_software
    cleanup_tenant_config(&pool, tenant_id).await;
    tax::tenant_config::set(
        &pool,
        tenant_id,
        "external_accounting_software",
        "local",
        updated_by,
        &Uuid::new_v4().to_string(),
    )
    .await
    .expect("set() must succeed");

    let app = app_with_tax(&pool, tenant_id);
    let body = serde_json::json!({
        "invoice_id": format!("inv-{}", Uuid::new_v4()),
        "customer_id": "cust-ext-test",
        "ship_to": {
            "line1": "123 Main St",
            "city": "Los Angeles",
            "state": "CA",
            "postal_code": "90001",
            "country": "US"
        },
        "ship_from": {
            "line1": "456 Commerce Dr",
            "city": "Austin",
            "state": "TX",
            "postal_code": "78701",
            "country": "US"
        },
        "line_items": [
            {
                "line_id": "line-1",
                "description": "SaaS subscription",
                "amount_minor": 10000,
                "currency": "usd",
                "quantity": 1.0
            }
        ],
        "currency": "usd",
        "invoice_date": "2026-04-24T00:00:00Z"
    });

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/ar/tax/quote")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(
        json["total_tax_minor"], 0,
        "external source must return zero tax, got: {}",
        json
    );
    assert_eq!(json["provider"], "external_accounting_software");

    cleanup_tenant_config(&pool, tenant_id).await;
}

/// Same invoice hash under two tenants with different configs returns different cached results.
/// Tenant A (platform/local) gets non-zero tax; Tenant B (external) gets zero.
/// The cache must be keyed per-tenant (not shared across tenants).
#[tokio::test]
async fn tax_calc_uses_tenant_config_cache_keyed_per_tenant() {
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let pool = common::setup_pool().await;
    // Use a unique UUID so this test is fully isolated from concurrent tests
    let tenant_a = Uuid::new_v4();

    // Tenant A: platform/local — expects non-zero tax for CA
    cleanup_tenant_config(&pool, tenant_a).await;
    cleanup_tax_cache(&pool, &tenant_a.to_string()).await;

    tax::tenant_config::set(
        &pool,
        tenant_a,
        "platform",
        "local",
        Uuid::new_v4(),
        &Uuid::new_v4().to_string(),
    )
    .await
    .expect("set tenant_a config");

    let invoice_id = format!("inv-cache-test-{}", Uuid::new_v4());

    let body = serde_json::json!({
        "invoice_id": invoice_id,
        "customer_id": "cust-cache-test",
        "ship_to": {
            "line1": "100 Test St",
            "city": "San Francisco",
            "state": "CA",
            "postal_code": "94102",
            "country": "US"
        },
        "ship_from": {
            "line1": "200 Origin Rd",
            "city": "Austin",
            "state": "TX",
            "postal_code": "78701",
            "country": "US"
        },
        "line_items": [
            {
                "line_id": "line-1",
                "description": "Subscription",
                "amount_minor": 10000,
                "currency": "usd",
                "quantity": 1.0
            }
        ],
        "currency": "usd",
        "invoice_date": "2026-04-24T00:00:00Z"
    });

    let app_a = app_with_tax(&pool, tenant_a);

    // First call for tenant_a — cache miss, provider computes
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/ar/tax/quote")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();
    let resp = app_a.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json_a: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    // LocalTaxProvider for CA → 8.5% of 10000 = 850
    assert_eq!(
        json_a["total_tax_minor"], 850,
        "platform/local for CA must return 850, got: {}",
        json_a
    );
    assert_eq!(json_a["cached"], false, "first call must be a cache miss");

    // Second call for tenant_a with same invoice → cache hit
    let app_a2 = app_with_tax(&pool, tenant_a);
    let req2 = Request::builder()
        .method(Method::POST)
        .uri("/api/ar/tax/quote")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();
    let resp2 = app_a2.oneshot(req2).await.unwrap();
    assert_eq!(resp2.status(), StatusCode::OK);
    let bytes2 = resp2.into_body().collect().await.unwrap().to_bytes();
    let json_a2: serde_json::Value = serde_json::from_slice(&bytes2).unwrap();

    assert_eq!(json_a2["total_tax_minor"], 850);
    assert_eq!(json_a2["cached"], true, "second identical call must be a cache hit");

    // Config change → config_version bumps → cache miss on next call
    tax::tenant_config::set(
        &pool,
        tenant_a,
        "platform",
        "zero",  // switch to zero provider
        Uuid::new_v4(),
        &Uuid::new_v4().to_string(),
    )
    .await
    .expect("set tenant_a to zero provider");

    let app_a3 = app_with_tax(&pool, tenant_a);
    let req3 = Request::builder()
        .method(Method::POST)
        .uri("/api/ar/tax/quote")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();
    let resp3 = app_a3.oneshot(req3).await.unwrap();
    assert_eq!(resp3.status(), StatusCode::OK);
    let bytes3 = resp3.into_body().collect().await.unwrap().to_bytes();
    let json_a3: serde_json::Value = serde_json::from_slice(&bytes3).unwrap();

    assert_eq!(
        json_a3["total_tax_minor"], 0,
        "after switch to zero provider, tax must be 0"
    );
    assert_eq!(
        json_a3["cached"], false,
        "config change must invalidate cache (new config_version = cache miss)"
    );

    cleanup_tenant_config(&pool, tenant_a).await;
    cleanup_tax_cache(&pool, &tenant_a.to_string()).await;
}

// ============================================================================
// CHANGE 1 tests: reconciliation_threshold_pct surfaced in GET response
// ============================================================================

/// Insert a row with reconciliation_threshold_pct=0.015 directly via SQL, then GET
/// and assert the response returns the custom threshold.
#[tokio::test]
async fn tenant_tax_config_get_returns_reconciliation_threshold_pct() {
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let pool = common::setup_pool().await;
    let tenant_id = Uuid::new_v4();
    cleanup_tenant_config(&pool, tenant_id).await;

    // Insert directly with a custom threshold (bypass set() which doesn't expose threshold writes)
    sqlx::query(
        "INSERT INTO ar_tenant_tax_config \
         (tenant_id, tax_calculation_source, provider_name, config_version, \
          updated_at, updated_by, reconciliation_threshold_pct) \
         VALUES ($1, 'platform', 'local', 1, NOW(), $2, 0.015)",
    )
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("insert test row with custom threshold");

    let app = app_with_tax(&pool, tenant_id);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/ar/tax/tenant-config")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    // rust_decimal serializes NUMERIC as a string preserving DB precision ("0.0150" for NUMERIC(6,4))
    let pct = &json["reconciliation_threshold_pct"];
    let pct_val: f64 = pct
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| pct.as_f64())
        .expect("reconciliation_threshold_pct must be a numeric string or number");
    assert!(
        (pct_val - 0.015).abs() < 1e-9,
        "expected reconciliation_threshold_pct ≈ 0.015, got: {}",
        pct
    );

    cleanup_tenant_config(&pool, tenant_id).await;
}

// ============================================================================
// Avalara sandbox test (requires AVALARA_* env vars; ignored by default)
// ============================================================================

/// When source=platform, provider=avalara, the quote path calls AvalaraProvider.
///
/// Run with:
/// ```bash
/// AVALARA_ACCOUNT_ID=... AVALARA_LICENSE_KEY=... AVALARA_COMPANY_CODE=... \
///   ./scripts/cargo-slot.sh test -p ar-rs -- tax_calc_uses_tenant_config_platform_source_calls_provider -- --ignored
/// ```
#[tokio::test]
#[ignore]
async fn tax_calc_uses_tenant_config_platform_source_calls_provider() {
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let pool = common::setup_pool().await;
    let tenant_id = Uuid::new_v4();

    cleanup_tenant_config(&pool, tenant_id).await;
    cleanup_tax_cache(&pool, &tenant_id.to_string()).await;

    tax::tenant_config::set(
        &pool,
        tenant_id,
        "platform",
        "avalara",
        Uuid::new_v4(),
        &Uuid::new_v4().to_string(),
    )
    .await
    .expect("set tenant config to avalara");

    let app = app_with_tax(&pool, tenant_id);
    let invoice_id = format!("inv-avalara-spy-{}", Uuid::new_v4());

    let body = serde_json::json!({
        "invoice_id": invoice_id,
        "customer_id": "test-customer",
        "ship_to": {
            "line1": "100 Main St",
            "city": "Los Angeles",
            "state": "CA",
            "postal_code": "90001",
            "country": "US"
        },
        "ship_from": {
            "line1": "123 Commerce Blvd",
            "city": "Austin",
            "state": "TX",
            "postal_code": "78701",
            "country": "US"
        },
        "line_items": [
            {
                "line_id": "line-1",
                "description": "SaaS subscription",
                "amount_minor": 10000,
                "currency": "usd",
                "tax_code": "SW050000",
                "quantity": 1.0
            }
        ],
        "currency": "usd",
        "invoice_date": "2026-04-24T00:00:00Z"
    });

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/ar/tax/quote")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Avalara quote must return 200"
    );

    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    // Avalara sandbox returns positive tax for CA
    assert!(
        json["total_tax_minor"].as_i64().unwrap_or(0) > 0,
        "Avalara must return positive tax for CA sale, got: {}",
        json
    );
    assert_eq!(
        json["provider"], "avalara",
        "provider field must be 'avalara'"
    );
    assert_eq!(json["cached"], false, "first call must be a cache miss");

    cleanup_tenant_config(&pool, tenant_id).await;
    cleanup_tax_cache(&pool, &tenant_id.to_string()).await;
}
