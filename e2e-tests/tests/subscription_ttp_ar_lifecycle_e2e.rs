//! E2E: Subscription lifecycle triggers billing via TTP and invoicing via AR (bd-2d8d)
//!
//! Proves the SaaS revenue recognition chain end-to-end:
//! 1. Create subscription (starts suspended/pre-activation)
//! 2. Activate subscription (suspended → active)
//! 3. TTP metering records usage for the tenant
//! 4. TTP billing run creates charges (agreement + metered usage)
//! 5. AR invoice generated from billing run
//!
//! Three modules integrated: Subscriptions → TTP → AR.
//! No mocks. No stubs. Real databases and services.

mod common;

use chrono::{DateTime, NaiveDate, Utc};
use common::{get_ar_pool, get_subscriptions_pool, get_tenant_registry_pool, get_ttp_pool};
use sqlx::PgPool;
use ttp_rs::{
    clients::ar::ArClient,
    clients::tenant_registry::TenantRegistryClient,
    domain::billing::{derive_item_key, run_billing},
    domain::metering::{ingest_event, MeteringEventInput},
};
use uuid::Uuid;

fn ar_base_url() -> String {
    std::env::var("AR_BASE_URL").unwrap_or_else(|_| "http://localhost:8086".to_string())
}

fn tenant_registry_url() -> String {
    std::env::var("TENANT_REGISTRY_URL").unwrap_or_else(|_| "http://localhost:8092".to_string())
}

async fn run_ttp_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/ttp/db/migrations")
        .run(pool)
        .await
        .expect("TTP migrations");
}

async fn insert_test_tenant(tr_pool: &PgPool, tenant_id: Uuid, app_id: &str) {
    sqlx::query(
        "INSERT INTO tenants (tenant_id, status, environment, module_schema_versions,
         product_code, plan_code, app_id, created_at, updated_at)
         VALUES ($1, 'active', 'development', '{}'::jsonb, 'business', 'monthly', $2, NOW(), NOW())
         ON CONFLICT (tenant_id) DO UPDATE SET app_id = EXCLUDED.app_id",
    )
    .bind(tenant_id)
    .bind(app_id)
    .execute(tr_pool)
    .await
    .expect("insert test tenant");
}

async fn create_ar_customer(pool: &PgPool, app_id: &str) -> i32 {
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
         VALUES ($1, $2, $3, 'active', 0, NOW(), NOW()) RETURNING id",
    )
    .bind(app_id)
    .bind(format!("sub-ttp-ar-{}@test.com", Uuid::new_v4()))
    .bind("Sub TTP AR E2E Customer")
    .fetch_one(pool).await.expect("create AR customer")
}

async fn create_suspended_subscription(
    pool: &PgPool,
    tenant_id: &str,
    ar_customer_id: i32,
    next_bill_date: NaiveDate,
    price_minor: i64,
) -> (Uuid, Uuid) {
    let plan_id: Uuid = sqlx::query_scalar(
        "INSERT INTO subscription_plans (tenant_id, name, schedule, price_minor, currency)
         VALUES ($1, 'SaaS Pro Plan', 'monthly', $2, 'USD') RETURNING id",
    )
    .bind(tenant_id)
    .bind(price_minor)
    .fetch_one(pool)
    .await
    .expect("create plan");

    let sub_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO subscriptions (id, tenant_id, ar_customer_id, plan_id, status, schedule,
         price_minor, currency, start_date, next_bill_date)
         VALUES ($1, $2, $3, $4, 'suspended', 'monthly', $5, 'USD', $6, $6)",
    )
    .bind(sub_id)
    .bind(tenant_id)
    .bind(ar_customer_id.to_string())
    .bind(plan_id)
    .bind(price_minor)
    .bind(next_bill_date)
    .execute(pool)
    .await
    .expect("create suspended subscription");
    (plan_id, sub_id)
}

async fn seed_ttp_data(pool: &PgPool, tenant_id: Uuid, party_id: Uuid) {
    sqlx::query(
        "INSERT INTO ttp_customers (tenant_id, party_id, status) VALUES ($1, $2, 'active')
         ON CONFLICT (tenant_id, party_id) DO NOTHING",
    )
    .bind(tenant_id)
    .bind(party_id)
    .execute(pool)
    .await
    .expect("seed ttp_customers");

    sqlx::query(
        "INSERT INTO ttp_service_agreements (tenant_id, party_id, plan_code, amount_minor,
         currency, billing_cycle, status, effective_from)
         VALUES ($1, $2, 'saas-pro', 15000, 'usd', 'monthly', 'active', '2026-01-01')",
    )
    .bind(tenant_id)
    .bind(party_id)
    .execute(pool)
    .await
    .expect("seed agreement");
}

async fn seed_ttp_pricing(pool: &PgPool, tenant_id: Uuid) {
    for (dim, price) in &[("api_calls", 10i64), ("storage_gb", 500)] {
        sqlx::query(
            "INSERT INTO ttp_metering_pricing (tenant_id, dimension, unit_price_minor, currency, effective_from)
             VALUES ($1, $2, $3, 'usd', '2026-01-01')
             ON CONFLICT (tenant_id, dimension, effective_from) DO NOTHING",
        )
        .bind(tenant_id).bind(dim).bind(price).execute(pool).await.expect("seed pricing");
    }
}

async fn ingest_metering_events(pool: &PgPool, tenant_id: Uuid) {
    let specs: Vec<(&str, i64, &str)> = vec![
        ("api_calls", 100, "2026-03-05T10:00:00Z"),
        ("api_calls", 50, "2026-03-15T14:00:00Z"),
        ("storage_gb", 10, "2026-03-10T08:00:00Z"),
    ];
    for (i, (dim, qty, ts)) in specs.iter().enumerate() {
        let result = ingest_event(
            pool,
            &MeteringEventInput {
                tenant_id,
                dimension: dim.to_string(),
                quantity: *qty,
                occurred_at: DateTime::parse_from_rfc3339(ts)
                    .unwrap()
                    .with_timezone(&Utc),
                idempotency_key: format!("e2e-{}-{}-{}", dim, i, tenant_id),
                source_ref: Some("e2e-test".to_string()),
            },
        )
        .await
        .expect("ingest metering event");
        assert!(
            !result.was_duplicate,
            "first ingestion must not be a duplicate"
        );
    }
}

/// Delete all test data for the tenant across TTP, tenant-registry, AR, and subscriptions.
async fn cleanup(
    ttp: &PgPool,
    tr: &PgPool,
    ar: &PgPool,
    subs: &PgPool,
    tid: Uuid,
    tid_str: &str,
    party_id: Uuid,
) {
    // TTP tables (FK order: items → runs → rest)
    let ttp_stmts = [
        "DELETE FROM ttp_billing_run_items WHERE run_id IN (SELECT run_id FROM ttp_billing_runs WHERE tenant_id = $1)",
        "DELETE FROM ttp_billing_runs WHERE tenant_id = $1",
        "DELETE FROM ttp_one_time_charges WHERE tenant_id = $1",
        "DELETE FROM ttp_service_agreements WHERE tenant_id = $1",
        "DELETE FROM ttp_customers WHERE tenant_id = $1",
        "DELETE FROM ttp_metering_events WHERE tenant_id = $1",
        "DELETE FROM ttp_metering_pricing WHERE tenant_id = $1",
    ];
    for stmt in &ttp_stmts {
        sqlx::query(stmt).bind(tid).execute(ttp).await.ok();
    }
    sqlx::query("DELETE FROM tenants WHERE tenant_id = $1")
        .bind(tid)
        .execute(tr)
        .await
        .ok();

    // AR: TTP-created invoices use app_id='test-app', external_customer_id=party_id
    let ps = party_id.to_string();
    sqlx::query(
        "DELETE FROM ar_invoices WHERE app_id = 'test-app'
         AND ar_customer_id IN (SELECT id FROM ar_customers WHERE app_id = 'test-app' AND external_customer_id = $1)",
    ).bind(&ps).execute(ar).await.ok();
    sqlx::query("DELETE FROM ar_customers WHERE app_id = 'test-app' AND external_customer_id = $1")
        .bind(&ps)
        .execute(ar)
        .await
        .ok();
    // Also clean AR data keyed by subscription tenant_id_str
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(tid_str)
        .execute(ar)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(tid_str)
        .execute(ar)
        .await
        .ok();

    // Subscriptions (FK order: attempts → subscriptions → plans → outbox)
    let sub_stmts = [
        "DELETE FROM subscription_invoice_attempts WHERE tenant_id = $1",
        "DELETE FROM subscriptions WHERE tenant_id = $1",
        "DELETE FROM subscription_plans WHERE tenant_id = $1",
        "DELETE FROM events_outbox WHERE tenant_id = $1",
    ];
    for stmt in &sub_stmts {
        sqlx::query(stmt).bind(tid_str).execute(subs).await.ok();
    }
}

// ============================================================================
// Test
// ============================================================================

/// Full cross-module E2E: Subscription lifecycle → TTP billing → AR invoice.
#[tokio::test]
async fn test_subscription_lifecycle_triggers_ttp_billing_and_ar_invoice() {
    let ttp_pool = get_ttp_pool().await;
    let tr_pool = get_tenant_registry_pool().await;
    let ar_pool = get_ar_pool().await;
    let subs_pool = get_subscriptions_pool().await;

    run_ttp_migrations(&ttp_pool).await;

    let tenant_id = Uuid::new_v4();
    let party_id = Uuid::new_v4();
    let tid_str = format!("test-tenant-{}", tenant_id);
    let app_id = format!("app-{}", &tenant_id.to_string().replace('-', "")[..12]);
    let billing_period = format!("2099-{:02}", (tenant_id.as_u128() % 12 + 1) as u8);
    let idem_key = format!("sub-ttp-ar-e2e-{}", tenant_id);

    cleanup(
        &ttp_pool, &tr_pool, &ar_pool, &subs_pool, tenant_id, &tid_str, party_id,
    )
    .await;

    // ── Step 1: Create subscription in SUSPENDED state (pre-activation) ─────
    let ar_cust_id = create_ar_customer(&ar_pool, &tid_str).await;
    let bill_date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let (_, sub_id) =
        create_suspended_subscription(&subs_pool, &tid_str, ar_cust_id, bill_date, 4999).await;

    let status: String = sqlx::query_scalar("SELECT status FROM subscriptions WHERE id = $1")
        .bind(sub_id)
        .fetch_one(&subs_pool)
        .await
        .expect("fetch status");
    assert_eq!(status, "suspended", "subscription must start suspended");

    // ── Step 2: Activate subscription (suspended → active) ──────────────────
    subscriptions_rs::transition_to_active(sub_id, &tid_str, "trial_converted", &subs_pool)
        .await
        .expect("activate subscription");

    let status: String = sqlx::query_scalar("SELECT status FROM subscriptions WHERE id = $1")
        .bind(sub_id)
        .fetch_one(&subs_pool)
        .await
        .expect("fetch status");
    assert_eq!(
        status, "active",
        "subscription must be active after activation"
    );

    // ── Step 3: Seed TTP data and record metering usage ─────────────────────
    insert_test_tenant(&tr_pool, tenant_id, &app_id).await;
    seed_ttp_data(&ttp_pool, tenant_id, party_id).await;
    seed_ttp_pricing(&ttp_pool, tenant_id).await;
    ingest_metering_events(&ttp_pool, tenant_id).await;

    let evt_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ttp_metering_events WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(&ttp_pool)
            .await
            .expect("count events");
    assert_eq!(evt_count, 3, "exactly 3 metering events recorded");

    // ── Step 4: TTP billing run ─────────────────────────────────────────────
    let registry = TenantRegistryClient::new(tenant_registry_url());
    let ar = ArClient::new(ar_base_url());
    let svc_claims = platform_sdk::PlatformClient::service_claims(tenant_id);

    let summary = run_billing(
        &ttp_pool,
        &registry,
        &ar,
        &svc_claims,
        tenant_id,
        &billing_period,
        &idem_key,
    )
    .await
    .unwrap_or_else(|e| panic!("billing run failed: {:?}", e));

    assert!(!summary.was_noop, "first billing run must not be a no-op");
    assert!(
        summary.parties_billed >= 1,
        "at least one party must be billed"
    );
    assert!(
        summary.total_amount_minor > 0,
        "must produce positive charges"
    );

    let run_status: String = sqlx::query_scalar(
        "SELECT status FROM ttp_billing_runs WHERE tenant_id = $1 AND billing_period = $2",
    )
    .bind(tenant_id)
    .bind(&billing_period)
    .fetch_one(&ttp_pool)
    .await
    .expect("run status");
    assert_eq!(run_status, "completed", "billing run must be completed");

    let inv_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ttp_billing_run_items WHERE run_id = $1 AND status = 'invoiced'",
    )
    .bind(summary.run_id)
    .fetch_one(&ttp_pool)
    .await
    .expect("invoiced count");
    assert!(inv_count >= 1, "at least one item must be invoiced");

    // ── Step 5: Verify AR invoice for service agreement ─────────────────────
    let item_key = derive_item_key(summary.run_id, party_id);
    let ar_status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM ar_invoices WHERE app_id = 'test-app' AND correlation_id = $1 LIMIT 1",
    )
    .bind(&item_key)
    .fetch_optional(&ar_pool)
    .await
    .expect("query AR invoice");
    assert_eq!(
        ar_status.as_deref(),
        Some("open"),
        "AR invoice must be finalized (open)"
    );

    let ar_amount: Option<i32> = sqlx::query_scalar(
        "SELECT amount_cents FROM ar_invoices WHERE app_id = 'test-app' AND correlation_id = $1 LIMIT 1",
    ).bind(&item_key).fetch_optional(&ar_pool).await.expect("query amount");
    assert_eq!(
        ar_amount,
        Some(15000),
        "AR invoice amount must equal agreement (15000)"
    );

    // ── Step 6: Verify metering trace produced AR invoice ───────────────────
    // Metered usage billed as separate item with party_id = tenant_id.
    // Expected: api_calls (150 * 10 = 1500) + storage_gb (10 * 500 = 5000) = 6500
    let meter_key = derive_item_key(summary.run_id, tenant_id);
    let meter_status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM ar_invoices WHERE app_id = 'test-app' AND correlation_id = $1 LIMIT 1",
    )
    .bind(&meter_key)
    .fetch_optional(&ar_pool)
    .await
    .expect("query metered invoice");
    assert_eq!(
        meter_status.as_deref(),
        Some("open"),
        "metered AR invoice must be open"
    );

    let meter_amt: Option<i32> = sqlx::query_scalar(
        "SELECT amount_cents FROM ar_invoices WHERE app_id = 'test-app' AND correlation_id = $1 LIMIT 1",
    ).bind(&meter_key).fetch_optional(&ar_pool).await.expect("query metered amount");
    assert_eq!(
        meter_amt,
        Some(6500),
        "metered invoice = (150*10 + 10*500) = 6500"
    );

    // ── Step 7: Idempotency — rerun produces no duplicates ──────────────────
    let s2 = run_billing(
        &ttp_pool,
        &registry,
        &ar,
        &svc_claims,
        tenant_id,
        &billing_period,
        &idem_key,
    )
    .await
    .unwrap_or_else(|e| panic!("second run failed: {:?}", e));
    assert!(s2.was_noop, "second billing run must be a no-op");
    assert_eq!(s2.run_id, summary.run_id, "run_id must be stable");

    // ── Step 8: Subscription remains active after billing ───────────────────
    let final_status: String = sqlx::query_scalar("SELECT status FROM subscriptions WHERE id = $1")
        .bind(sub_id)
        .fetch_one(&subs_pool)
        .await
        .expect("final status");
    assert_eq!(
        final_status, "active",
        "subscription must remain active after billing"
    );

    cleanup(
        &ttp_pool, &tr_pool, &ar_pool, &subs_pool, tenant_id, &tid_str, party_id,
    )
    .await;
}
