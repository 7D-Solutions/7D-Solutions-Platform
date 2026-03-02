//! E2E proof: Platform billing invoices tenant under PLATFORM merchant_context
//!
//! Verifies (bd-228l):
//! - Invoice is created in AR under PLATFORM app_id only
//! - No platform-billing invoice created under the tenant's own app_id
//! - Events emitted to provisioning_outbox carry merchant_context=PLATFORM

mod common;

use axum::extract::State;
use axum::Json;
use common::{get_ar_pool, get_tenant_registry_pool};
use control_plane::clients::ar::PLATFORM_APP_ID;
use control_plane::handlers::platform_billing_run::{
    platform_billing_run, PlatformBillingRunRequest,
};
use control_plane::state::AppState;
use std::sync::Arc;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

/// Build the billing period for a test tenant.
/// Uses a far-future year (2099) and a month derived from the tenant UUID
/// to avoid collisions with other tests or real billing runs.
fn test_period(tenant_id: Uuid) -> String {
    let month = (tenant_id.as_u128() % 12 + 1) as u32;
    format!("2099-{:02}", month)
}

/// Insert a test tenant in 'active' state with the given plan/product codes.
async fn insert_active_tenant(
    tr_pool: &sqlx::PgPool,
    tenant_id: Uuid,
    tenant_app_id: &str,
    product_code: &str,
    plan_code: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO tenants
            (tenant_id, status, environment, module_schema_versions,
             product_code, plan_code, app_id, created_at, updated_at)
        VALUES ($1, 'active', 'development', '{}'::jsonb, $2, $3, $4, NOW(), NOW())
        "#,
    )
    .bind(tenant_id)
    .bind(product_code)
    .bind(plan_code)
    .bind(tenant_app_id)
    .execute(tr_pool)
    .await
    .expect("insert test tenant");
}

/// Remove all test data created by this test.
async fn cleanup(
    tr_pool: &sqlx::PgPool,
    ar_pool: &sqlx::PgPool,
    tenant_id: Uuid,
    tenant_app_id: &str,
    period: &str,
) {
    // Delete AR invoices created by the platform runner for this tenant/period
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1 AND correlation_id = $2")
        .bind(PLATFORM_APP_ID)
        .bind(format!("plat-{}-{}", tenant_id, period))
        .execute(ar_pool)
        .await
        .ok();

    // Delete the PLATFORM-side AR customer for this tenant
    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1 AND external_customer_id = $2")
        .bind(PLATFORM_APP_ID)
        .bind(tenant_id.to_string())
        .execute(ar_pool)
        .await
        .ok();

    // Delete any stray invoices under the tenant's own app_id (should be none)
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1 AND correlation_id LIKE $2")
        .bind(tenant_app_id)
        .bind(format!("plat-{}-%", tenant_id))
        .execute(ar_pool)
        .await
        .ok();

    // Delete outbox events
    sqlx::query(
        "DELETE FROM provisioning_outbox WHERE tenant_id = $1 AND event_type = 'platform.billing.invoice_created'",
    )
    .bind(tenant_id)
    .execute(tr_pool)
    .await
    .ok();

    // Delete tenant
    sqlx::query("DELETE FROM tenants WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(tr_pool)
        .await
        .ok();
}

// ============================================================================
// Tests
// ============================================================================

/// Primary assertion: platform billing creates an invoice under PLATFORM app_id only.
#[tokio::test]
async fn test_platform_billing_invoices_tenant() {
    let tr_pool = get_tenant_registry_pool().await;
    let ar_pool = get_ar_pool().await;

    let tenant_id = Uuid::new_v4();
    let tenant_app_id = format!("app-{}", &tenant_id.to_string().replace('-', "")[..12]);
    let period = test_period(tenant_id);

    insert_active_tenant(&tr_pool, tenant_id, &tenant_app_id, "starter", "monthly").await;

    let state = Arc::new(AppState {
        pool: tr_pool.clone(),
        ar_pool: Some(ar_pool.clone()),
    });

    // -----------------------------------------------------------------------
    // Run platform billing
    // -----------------------------------------------------------------------
    let result = platform_billing_run(
        State(state.clone()),
        Json(PlatformBillingRunRequest {
            period: period.clone(),
        }),
    )
    .await
    .expect("platform_billing_run should succeed");

    let (status_code, Json(resp)) = result;
    assert_eq!(
        status_code,
        axum::http::StatusCode::OK,
        "billing run should return 200 OK"
    );
    assert_eq!(
        resp.merchant_context, "PLATFORM",
        "response must declare merchant_context=PLATFORM"
    );

    // Our tenant must appear in the processed list
    let entry = resp
        .processed
        .iter()
        .find(|e| e.tenant_id == tenant_id)
        .unwrap_or_else(|| {
            panic!(
                "tenant {} not in processed list; skipped={:?}",
                tenant_id, resp.skipped
            )
        });

    // Starter plan is seeded at 2900 minor units in cp_plans
    assert_eq!(
        entry.amount_cents, 2900,
        "starter plan fee must be 2900 minor units (from cp_plans)"
    );

    // -----------------------------------------------------------------------
    // Assert 1: Invoice exists under PLATFORM app_id
    // -----------------------------------------------------------------------
    let platform_invoice_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_invoices WHERE app_id = $1 AND correlation_id = $2",
    )
    .bind(PLATFORM_APP_ID)
    .bind(format!("plat-{}-{}", tenant_id, period))
    .fetch_one(&ar_pool)
    .await
    .expect("query platform invoice");

    assert_eq!(
        platform_invoice_count, 1,
        "exactly one invoice must exist under PLATFORM app_id"
    );

    // -----------------------------------------------------------------------
    // Assert 2: No platform-billing invoice under the tenant's own app_id
    // -----------------------------------------------------------------------
    let tenant_invoice_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_invoices WHERE app_id = $1 AND correlation_id LIKE $2",
    )
    .bind(&tenant_app_id)
    .bind(format!("plat-{}-%", tenant_id))
    .fetch_one(&ar_pool)
    .await
    .expect("query tenant app_id invoices");

    assert_eq!(
        tenant_invoice_count, 0,
        "platform billing must NOT create invoices under the tenant's own app_id"
    );

    // -----------------------------------------------------------------------
    // Assert 3: Outbox event carries merchant_context=PLATFORM
    // -----------------------------------------------------------------------
    let payload: serde_json::Value = sqlx::query_scalar(
        r#"
        SELECT payload
        FROM provisioning_outbox
        WHERE tenant_id = $1
          AND event_type = 'platform.billing.invoice_created'
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(tenant_id)
    .fetch_one(&tr_pool)
    .await
    .expect("query outbox event");

    assert_eq!(
        payload["merchant_context"].as_str(),
        Some("PLATFORM"),
        "outbox event payload must carry merchant_context=PLATFORM"
    );
    assert_eq!(
        payload["app_id"].as_str(),
        Some(PLATFORM_APP_ID),
        "outbox event payload must carry app_id=platform"
    );
    assert_eq!(
        payload["period"].as_str(),
        Some(period.as_str()),
        "outbox event payload must carry correct period"
    );

    // -----------------------------------------------------------------------
    // Assert 4: Idempotency — re-running the same period produces no new invoices
    // -----------------------------------------------------------------------
    let result2 = platform_billing_run(
        State(state.clone()),
        Json(PlatformBillingRunRequest {
            period: period.clone(),
        }),
    )
    .await
    .expect("second billing run should succeed");

    let (_, Json(resp2)) = result2;
    assert!(
        resp2
            .skipped
            .iter()
            .any(|e| e.tenant_id == tenant_id && e.reason == "already_billed"),
        "second run must skip the tenant with reason=already_billed"
    );

    // Invoice count must still be exactly 1
    let count_after_rerun: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_invoices WHERE app_id = $1 AND correlation_id = $2",
    )
    .bind(PLATFORM_APP_ID)
    .bind(format!("plat-{}-{}", tenant_id, period))
    .fetch_one(&ar_pool)
    .await
    .expect("query invoice count after rerun");

    assert_eq!(
        count_after_rerun, 1,
        "idempotent rerun must not create duplicate invoices"
    );

    // -----------------------------------------------------------------------
    // Cleanup
    // -----------------------------------------------------------------------
    cleanup(&tr_pool, &ar_pool, tenant_id, &tenant_app_id, &period).await;
}
