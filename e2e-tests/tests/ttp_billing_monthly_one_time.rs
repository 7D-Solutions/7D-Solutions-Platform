//! E2E proof: TTP tenant billing — monthly + one-time charges with TENANT merchant_context
//!
//! Verifies (bd-2hdr):
//! - Billing run produces exactly one finalized AR invoice per party per period.
//! - One-time charge transitions pending → billed with ar_invoice_id set.
//! - Rerun is idempotent: was_noop=true, no duplicate invoices or charges.
//! - EventEnvelope carries merchant_context=TENANT(tenant_id).
//!
//! **Requirements:**
//! - TTP postgres running at localhost:5450 (or TTP_DATABASE_URL)
//! - AR service running at localhost:8086 (or AR_BASE_URL)
//! - Tenant-registry service running at localhost:8092 (or TENANT_REGISTRY_URL)
//! - Tenant-registry postgres running at localhost:5441 (or TENANT_REGISTRY_DATABASE_URL)
//! - AR postgres running at localhost:5434 (or AR_DATABASE_URL)

mod common;

use common::{get_ar_pool, get_tenant_registry_pool, wait_for_db_ready};
use event_bus::MerchantContext;
use sqlx::PgPool;
use ttp_rs::{
    clients::ar::ArClient,
    clients::tenant_registry::TenantRegistryClient,
    domain::billing::{derive_item_key, run_billing},
    events::{create_ttp_envelope, BillingRunCompleted, BILLING_RUN_COMPLETED},
};
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

fn get_ttp_db_url() -> String {
    std::env::var("TTP_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://ttp_user:ttp_pass@localhost:5450/ttp_db".to_string())
}

fn get_ar_base_url() -> String {
    std::env::var("AR_BASE_URL").unwrap_or_else(|_| "http://localhost:8086".to_string())
}

fn get_tenant_registry_url() -> String {
    std::env::var("TENANT_REGISTRY_URL").unwrap_or_else(|_| "http://localhost:8092".to_string())
}

async fn get_ttp_pool() -> PgPool {
    wait_for_db_ready("ttp", &get_ttp_db_url()).await
}

async fn run_ttp_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/ttp/db/migrations")
        .run(pool)
        .await
        .expect("TTP migrations failed");
}

/// Insert a test tenant into tenant-registry with a known app_id so the
/// tenant-registry service can serve GET /api/tenants/{id}/app-id.
async fn insert_test_tenant(tr_pool: &PgPool, tenant_id: Uuid, app_id: &str) {
    sqlx::query(
        r#"
        INSERT INTO tenants
            (tenant_id, status, environment, module_schema_versions,
             product_code, plan_code, app_id, created_at, updated_at)
        VALUES ($1, 'active', 'development', '{}'::jsonb, 'business', 'monthly', $2, NOW(), NOW())
        ON CONFLICT (tenant_id) DO UPDATE SET app_id = EXCLUDED.app_id
        "#,
    )
    .bind(tenant_id)
    .bind(app_id)
    .execute(tr_pool)
    .await
    .expect("insert test tenant into tenant-registry");
}

/// Seed TTP data for one party: customer + monthly agreement + one-time charge.
///
/// Returns the charge_id of the one-time charge so the test can assert on it.
async fn seed_ttp_party(pool: &PgPool, tenant_id: Uuid, party_id: Uuid) -> Uuid {
    sqlx::query(
        r#"
        INSERT INTO ttp_customers (tenant_id, party_id, status)
        VALUES ($1, $2, 'active')
        ON CONFLICT (tenant_id, party_id) DO NOTHING
        "#,
    )
    .bind(tenant_id)
    .bind(party_id)
    .execute(pool)
    .await
    .expect("seed ttp_customers");

    // Monthly service agreement: 12 000 minor units ($120.00)
    sqlx::query(
        r#"
        INSERT INTO ttp_service_agreements
            (tenant_id, party_id, plan_code, amount_minor, currency,
             billing_cycle, status, effective_from)
        VALUES ($1, $2, 'standard-monthly', 12000, 'usd', 'monthly', 'active', '2026-01-01')
        "#,
    )
    .bind(tenant_id)
    .bind(party_id)
    .execute(pool)
    .await
    .expect("seed ttp_service_agreements");

    // One-time charge: 5 000 minor units ($50.00)
    sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO ttp_one_time_charges
            (tenant_id, party_id, description, amount_minor, currency, status)
        VALUES ($1, $2, 'Setup fee (E2E test)', 5000, 'usd', 'pending')
        RETURNING charge_id
        "#,
    )
    .bind(tenant_id)
    .bind(party_id)
    .fetch_one(pool)
    .await
    .expect("seed ttp_one_time_charges")
}

/// Remove all test artifacts for the tenant from TTP, tenant-registry, and AR.
async fn cleanup(
    ttp_pool: &PgPool,
    tr_pool: &PgPool,
    ar_pool: &PgPool,
    tenant_id: Uuid,
    party_id: Uuid,
) {
    // TTP: items first (FK → runs), then runs, charges, agreements, customers
    sqlx::query(
        r#"
        DELETE FROM ttp_billing_run_items
        WHERE run_id IN (SELECT run_id FROM ttp_billing_runs WHERE tenant_id = $1)
        "#,
    )
    .bind(tenant_id)
    .execute(ttp_pool)
    .await
    .ok();

    sqlx::query("DELETE FROM ttp_billing_runs WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(ttp_pool)
        .await
        .ok();

    sqlx::query("DELETE FROM ttp_one_time_charges WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(ttp_pool)
        .await
        .ok();

    sqlx::query("DELETE FROM ttp_service_agreements WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(ttp_pool)
        .await
        .ok();

    sqlx::query("DELETE FROM ttp_customers WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(ttp_pool)
        .await
        .ok();

    // Tenant-registry
    sqlx::query("DELETE FROM tenants WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(tr_pool)
        .await
        .ok();

    // AR: invoices created by TTP use app_id='test-app' (AR placeholder) and
    // external_customer_id = party_id string.
    let party_id_str = party_id.to_string();
    sqlx::query(
        r#"
        DELETE FROM ar_invoices
        WHERE app_id = 'test-app'
          AND ar_customer_id IN (
              SELECT id FROM ar_customers
              WHERE app_id = 'test-app' AND external_customer_id = $1
          )
        "#,
    )
    .bind(&party_id_str)
    .execute(ar_pool)
    .await
    .ok();

    sqlx::query("DELETE FROM ar_customers WHERE app_id = 'test-app' AND external_customer_id = $1")
        .bind(&party_id_str)
        .execute(ar_pool)
        .await
        .ok();
}

// ============================================================================
// Test
// ============================================================================

/// Full-spine E2E proof for TTP tenant billing.
///
/// Covers the monthly agreement + one-time charge path with idempotency and
/// TENANT merchant_context assertion.
#[tokio::test]
async fn test_ttp_billing_monthly_one_time() {
    let ttp_pool = get_ttp_pool().await;
    let tr_pool = get_tenant_registry_pool().await;
    let ar_pool = get_ar_pool().await;

    // Ensure TTP schema is present
    run_ttp_migrations(&ttp_pool).await;

    let tenant_id = Uuid::new_v4();
    let party_id = Uuid::new_v4();
    let app_id = format!("app-{}", &tenant_id.to_string().replace('-', "")[..12]);

    // Use year 2099 + month derived from tenant UUID to avoid period collisions
    let billing_period = format!("2099-{:02}", (tenant_id.as_u128() % 12 + 1) as u8);
    let idempotency_key = format!("ttp-e2e-{}", tenant_id);

    // -----------------------------------------------------------------------
    // Setup: seed tenant-registry + TTP data
    // -----------------------------------------------------------------------
    insert_test_tenant(&tr_pool, tenant_id, &app_id).await;
    let charge_id = seed_ttp_party(&ttp_pool, tenant_id, party_id).await;

    let registry = TenantRegistryClient::new(get_tenant_registry_url());
    let ar = ArClient::new(get_ar_base_url());
    let svc_claims = platform_sdk::PlatformClient::service_claims(tenant_id);

    // -----------------------------------------------------------------------
    // Run 1: initial billing run
    // -----------------------------------------------------------------------
    let summary = run_billing(
        &ttp_pool,
        &registry,
        &ar,
        &svc_claims,
        tenant_id,
        &billing_period,
        &idempotency_key,
    )
    .await
    .unwrap_or_else(|e| panic!("billing run failed: {:?}", e));

    assert!(!summary.was_noop, "first run must not be a no-op");
    assert_eq!(summary.parties_billed, 1, "exactly one party billed");
    // Agreement 12 000 + one-time 5 000 = 17 000
    assert_eq!(
        summary.total_amount_minor, 17000,
        "total must be agreement (12 000) + one-time (5 000) = 17 000"
    );
    assert_eq!(summary.currency, "usd");

    let run_id = summary.run_id;

    // -----------------------------------------------------------------------
    // Assert 1: billing run completed with exactly one invoiced item
    // -----------------------------------------------------------------------
    let run_status: String =
        sqlx::query_scalar("SELECT status FROM ttp_billing_runs WHERE run_id = $1")
            .bind(run_id)
            .fetch_one(&ttp_pool)
            .await
            .expect("query ttp_billing_runs status");

    assert_eq!(
        run_status, "completed",
        "billing run must be in completed state"
    );

    let invoiced_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ttp_billing_run_items WHERE run_id = $1 AND status = 'invoiced'",
    )
    .bind(run_id)
    .fetch_one(&ttp_pool)
    .await
    .expect("query ttp_billing_run_items count");

    assert_eq!(
        invoiced_count, 1,
        "exactly one invoiced item per party per billing period"
    );

    // -----------------------------------------------------------------------
    // Assert 2: one-time charge transitioned pending → billed with ar_invoice_id
    // -----------------------------------------------------------------------
    let (charge_status, charge_ar_invoice_id): (String, Option<Uuid>) = sqlx::query_as(
        "SELECT status, ar_invoice_id FROM ttp_one_time_charges WHERE charge_id = $1",
    )
    .bind(charge_id)
    .fetch_one(&ttp_pool)
    .await
    .expect("query ttp_one_time_charges");

    assert_eq!(
        charge_status, "billed",
        "one-time charge must transition pending → billed"
    );
    assert!(
        charge_ar_invoice_id.is_some(),
        "one-time charge must have ar_invoice_id set after billing"
    );

    // -----------------------------------------------------------------------
    // Assert 3: AR invoice is finalized (status=open) in AR DB
    //
    // correlation_id in AR = derive_item_key(run_id, party_id) — SHA256 hex of
    // "{run_id}/{party_id}".  AR uses app_id='test-app' as placeholder.
    // -----------------------------------------------------------------------
    let item_key = derive_item_key(run_id, party_id);

    let ar_invoice_status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM ar_invoices WHERE app_id = 'test-app' AND correlation_id = $1 LIMIT 1",
    )
    .bind(&item_key)
    .fetch_optional(&ar_pool)
    .await
    .expect("query ar_invoices by correlation_id");

    assert_eq!(
        ar_invoice_status.as_deref(),
        Some("open"),
        "AR invoice must be finalized (status=open) after TTP billing run"
    );

    // -----------------------------------------------------------------------
    // Assert 4: idempotency — rerun produces no duplicates
    // -----------------------------------------------------------------------
    let summary2 = run_billing(
        &ttp_pool,
        &registry,
        &ar,
        &svc_claims,
        tenant_id,
        &billing_period,
        &idempotency_key,
    )
    .await
    .unwrap_or_else(|e| panic!("second billing run failed: {:?}", e));

    assert!(
        summary2.was_noop,
        "second run for same period must be a no-op"
    );
    assert_eq!(
        summary2.run_id, run_id,
        "run_id must be stable across idempotent reruns"
    );

    // Billing run items must remain at exactly 1
    let item_count_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ttp_billing_run_items WHERE run_id = $1 AND status = 'invoiced'",
    )
    .bind(run_id)
    .fetch_one(&ttp_pool)
    .await
    .expect("query billing run items after rerun");

    assert_eq!(
        item_count_after, 1,
        "idempotent rerun must not create additional billing items"
    );

    // One-time charge still billed (not re-billed)
    let billed_charge_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ttp_one_time_charges WHERE charge_id = $1 AND status = 'billed'",
    )
    .bind(charge_id)
    .fetch_one(&ttp_pool)
    .await
    .expect("query charge count after rerun");

    assert_eq!(
        billed_charge_count, 1,
        "charge must remain in billed state, not re-billed on idempotent rerun"
    );

    // AR invoice count for correlation_id must remain at 1
    let ar_invoice_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_invoices WHERE app_id = 'test-app' AND correlation_id = $1",
    )
    .bind(&item_key)
    .fetch_one(&ar_pool)
    .await
    .expect("query ar_invoices count after rerun");

    assert_eq!(
        ar_invoice_count, 1,
        "idempotent rerun must not create duplicate AR invoices"
    );

    // -----------------------------------------------------------------------
    // Assert 5: EventEnvelope carries merchant_context=TENANT(tenant_id)
    //
    // TTP events are emitted via create_ttp_envelope which sets TENANT context.
    // This assertion proves the mechanism is correct for the billing run event.
    // -----------------------------------------------------------------------
    let payload = BillingRunCompleted {
        run_id,
        tenant_id,
        billing_period: billing_period.clone(),
        parties_billed: summary.parties_billed,
        total_amount_minor: summary.total_amount_minor,
        currency: summary.currency.clone(),
    };

    let envelope = create_ttp_envelope(
        tenant_id,
        BILLING_RUN_COMPLETED,
        &idempotency_key,
        "billing",
        payload,
    );

    assert_eq!(
        envelope.merchant_context,
        Some(MerchantContext::Tenant(tenant_id.to_string())),
        "TTP billing envelope must carry merchant_context=TENANT(tenant_id)"
    );
    assert_eq!(
        envelope.tenant_id,
        tenant_id.to_string(),
        "envelope tenant_id must match the billed tenant"
    );
    assert_eq!(
        envelope.source_module, "ttp",
        "envelope source_module must be 'ttp'"
    );

    // -----------------------------------------------------------------------
    // Cleanup
    // -----------------------------------------------------------------------
    cleanup(&ttp_pool, &tr_pool, &ar_pool, tenant_id, party_id).await;
}
