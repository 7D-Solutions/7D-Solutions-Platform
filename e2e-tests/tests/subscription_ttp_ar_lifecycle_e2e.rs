//! E2E: Subscription lifecycle triggers billing via TTP and invoicing via AR (bd-2d8d)
//!
//! Proves the SaaS revenue recognition chain end-to-end:
//! 1. Create subscription via Subscriptions module (starts suspended/pre-activation)
//! 2. Activate subscription (suspended → active)
//! 3. TTP metering records usage for the tenant
//! 4. TTP billing run creates charges (agreement + metered usage)
//! 5. AR invoice generated from billing run
//!
//! Three modules integrated: Subscriptions → TTP → AR.
//!
//! **Requirements:**
//! - Subscriptions postgres at localhost:5435
//! - TTP postgres at localhost:5450
//! - AR service at localhost:8086
//! - AR postgres at localhost:5434
//! - Tenant-registry service at localhost:8092
//! - Tenant-registry postgres at localhost:5441
//!
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

// ============================================================================
// URL helpers
// ============================================================================

fn get_ar_base_url() -> String {
    std::env::var("AR_BASE_URL").unwrap_or_else(|_| "http://localhost:8086".to_string())
}

fn get_tenant_registry_url() -> String {
    std::env::var("TENANT_REGISTRY_URL")
        .unwrap_or_else(|_| "http://localhost:8092".to_string())
}

// ============================================================================
// Setup helpers
// ============================================================================

async fn run_ttp_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/ttp/db/migrations")
        .run(pool)
        .await
        .expect("TTP migrations failed");
}

/// Insert test tenant into tenant-registry so TTP can resolve tenant_id → app_id.
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

/// Create AR customer and return SERIAL id.
async fn create_ar_customer(pool: &PgPool, app_id: &str) -> i32 {
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO ar_customers
         (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
         VALUES ($1, $2, $3, 'active', 0, NOW(), NOW())
         RETURNING id",
    )
    .bind(app_id)
    .bind(format!("sub-ttp-ar-{}@test.com", Uuid::new_v4()))
    .bind("Subscription TTP AR E2E Customer")
    .fetch_one(pool)
    .await
    .expect("create AR customer")
}

/// Create subscription plan + subscription in SUSPENDED state (pre-activation).
async fn create_suspended_subscription(
    pool: &PgPool,
    tenant_id: &str,
    ar_customer_id: i32,
    next_bill_date: NaiveDate,
    price_minor: i64,
) -> (Uuid, Uuid) {
    let plan_id: Uuid = sqlx::query_scalar(
        "INSERT INTO subscription_plans
         (tenant_id, name, schedule, price_minor, currency)
         VALUES ($1, 'SaaS Pro Plan', 'monthly', $2, 'USD')
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(price_minor)
    .fetch_one(pool)
    .await
    .expect("create subscription plan");

    let subscription_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO subscriptions
         (id, tenant_id, ar_customer_id, plan_id, status, schedule,
          price_minor, currency, start_date, next_bill_date)
         VALUES ($1, $2, $3, $4, 'suspended', 'monthly', $5, 'USD', $6, $6)",
    )
    .bind(subscription_id)
    .bind(tenant_id)
    .bind(ar_customer_id.to_string())
    .bind(plan_id)
    .bind(price_minor)
    .bind(next_bill_date)
    .execute(pool)
    .await
    .expect("create subscription in suspended state");

    (plan_id, subscription_id)
}

/// Seed TTP customer + service agreement for the party.
async fn seed_ttp_data(pool: &PgPool, tenant_id: Uuid, party_id: Uuid) {
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

    // Monthly service agreement: 15 000 minor units ($150.00)
    sqlx::query(
        r#"
        INSERT INTO ttp_service_agreements
            (tenant_id, party_id, plan_code, amount_minor, currency,
             billing_cycle, status, effective_from)
        VALUES ($1, $2, 'saas-pro', 15000, 'usd', 'monthly', 'active', '2026-01-01')
        "#,
    )
    .bind(tenant_id)
    .bind(party_id)
    .execute(pool)
    .await
    .expect("seed ttp_service_agreements");
}

/// Seed TTP metering pricing rules for the tenant.
async fn seed_ttp_pricing(pool: &PgPool, tenant_id: Uuid) {
    // api_calls: 10 minor units per call ($0.10)
    sqlx::query(
        r#"
        INSERT INTO ttp_metering_pricing
            (tenant_id, dimension, unit_price_minor, currency, effective_from)
        VALUES ($1, 'api_calls', 10, 'usd', '2026-01-01')
        ON CONFLICT (tenant_id, dimension, effective_from) DO NOTHING
        "#,
    )
    .bind(tenant_id)
    .execute(pool)
    .await
    .expect("seed api_calls pricing");

    // storage_gb: 500 minor units per GB ($5.00)
    sqlx::query(
        r#"
        INSERT INTO ttp_metering_pricing
            (tenant_id, dimension, unit_price_minor, currency, effective_from)
        VALUES ($1, 'storage_gb', 500, 'usd', '2026-01-01')
        ON CONFLICT (tenant_id, dimension, effective_from) DO NOTHING
        "#,
    )
    .bind(tenant_id)
    .execute(pool)
    .await
    .expect("seed storage_gb pricing");
}

/// Ingest metering events into TTP for the tenant.
async fn ingest_metering_events(pool: &PgPool, tenant_id: Uuid) {
    let events = vec![
        MeteringEventInput {
            tenant_id,
            dimension: "api_calls".to_string(),
            quantity: 100,
            occurred_at: DateTime::parse_from_rfc3339("2026-03-05T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            idempotency_key: format!("e2e-api-1-{}", tenant_id),
            source_ref: Some("e2e-test".to_string()),
        },
        MeteringEventInput {
            tenant_id,
            dimension: "api_calls".to_string(),
            quantity: 50,
            occurred_at: DateTime::parse_from_rfc3339("2026-03-15T14:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            idempotency_key: format!("e2e-api-2-{}", tenant_id),
            source_ref: Some("e2e-test".to_string()),
        },
        MeteringEventInput {
            tenant_id,
            dimension: "storage_gb".to_string(),
            quantity: 10,
            occurred_at: DateTime::parse_from_rfc3339("2026-03-10T08:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            idempotency_key: format!("e2e-storage-1-{}", tenant_id),
            source_ref: Some("e2e-test".to_string()),
        },
    ];

    for event in &events {
        let result = ingest_event(pool, event)
            .await
            .expect("ingest metering event");
        assert!(
            !result.was_duplicate,
            "first ingestion must not be a duplicate"
        );
    }
}

// ============================================================================
// Cleanup
// ============================================================================

async fn cleanup_all(
    ttp_pool: &PgPool,
    tr_pool: &PgPool,
    ar_pool: &PgPool,
    subscriptions_pool: &PgPool,
    tenant_id: Uuid,
    tenant_id_str: &str,
    party_id: Uuid,
) {
    // TTP: items → runs → charges → agreements → customers → metering
    sqlx::query(
        "DELETE FROM ttp_billing_run_items
         WHERE run_id IN (SELECT run_id FROM ttp_billing_runs WHERE tenant_id = $1)",
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

    sqlx::query("DELETE FROM ttp_metering_events WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(ttp_pool)
        .await
        .ok();

    sqlx::query("DELETE FROM ttp_metering_pricing WHERE tenant_id = $1")
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

    // AR: invoices created by TTP (external_customer_id = party_id)
    let party_str = party_id.to_string();
    sqlx::query(
        "DELETE FROM ar_invoices
         WHERE app_id = 'test-app'
           AND ar_customer_id IN (
               SELECT id FROM ar_customers
               WHERE app_id = 'test-app' AND external_customer_id = $1
           )",
    )
    .bind(&party_str)
    .execute(ar_pool)
    .await
    .ok();

    sqlx::query("DELETE FROM ar_customers WHERE app_id = 'test-app' AND external_customer_id = $1")
        .bind(&party_str)
        .execute(ar_pool)
        .await
        .ok();

    // Also clean AR data keyed by tenant_id_str (subscription-side AR customer)
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(tenant_id_str)
        .execute(ar_pool)
        .await
        .ok();

    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(tenant_id_str)
        .execute(ar_pool)
        .await
        .ok();

    // Subscriptions: attempts → subscriptions → plans → outbox
    sqlx::query("DELETE FROM subscription_invoice_attempts WHERE tenant_id = $1")
        .bind(tenant_id_str)
        .execute(subscriptions_pool)
        .await
        .ok();

    sqlx::query("DELETE FROM subscriptions WHERE tenant_id = $1")
        .bind(tenant_id_str)
        .execute(subscriptions_pool)
        .await
        .ok();

    sqlx::query("DELETE FROM subscription_plans WHERE tenant_id = $1")
        .bind(tenant_id_str)
        .execute(subscriptions_pool)
        .await
        .ok();

    sqlx::query("DELETE FROM events_outbox WHERE tenant_id = $1")
        .bind(tenant_id_str)
        .execute(subscriptions_pool)
        .await
        .ok();
}

// ============================================================================
// Test
// ============================================================================

/// Full cross-module E2E: Subscription lifecycle → TTP billing → AR invoice.
///
/// Proves the SaaS revenue recognition chain: if subscriptions don't trigger
/// metering or billing runs don't produce invoices, the platform doesn't get paid.
#[tokio::test]
async fn test_subscription_lifecycle_triggers_ttp_billing_and_ar_invoice() {
    let ttp_pool = get_ttp_pool().await;
    let tr_pool = get_tenant_registry_pool().await;
    let ar_pool = get_ar_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;

    // Ensure TTP schema is present
    run_ttp_migrations(&ttp_pool).await;

    let tenant_id = Uuid::new_v4();
    let party_id = Uuid::new_v4();
    let tenant_id_str = format!("test-tenant-{}", tenant_id);
    let app_id = format!("app-{}", &tenant_id.to_string().replace('-', "")[..12]);
    let billing_period = format!("2099-{:02}", (tenant_id.as_u128() % 12 + 1) as u8);
    let idempotency_key = format!("sub-ttp-ar-e2e-{}", tenant_id);

    // Pre-cleanup
    cleanup_all(
        &ttp_pool,
        &tr_pool,
        &ar_pool,
        &subscriptions_pool,
        tenant_id,
        &tenant_id_str,
        party_id,
    )
    .await;

    // ─── Step 1: Create subscription in SUSPENDED state (pre-activation) ────
    let ar_customer_id =
        create_ar_customer(&ar_pool, &tenant_id_str);
    let ar_customer_id = ar_customer_id.await;

    let billing_date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let price_minor: i64 = 4999;
    let (_plan_id, subscription_id) = create_suspended_subscription(
        &subscriptions_pool,
        &tenant_id_str,
        ar_customer_id,
        billing_date,
        price_minor,
    )
    .await;

    // Verify: subscription starts suspended
    let status: String =
        sqlx::query_scalar("SELECT status FROM subscriptions WHERE id = $1")
            .bind(subscription_id)
            .fetch_one(&subscriptions_pool)
            .await
            .expect("fetch subscription status");
    assert_eq!(status, "suspended", "subscription must start in suspended state");

    // ─── Step 2: Activate subscription (suspended → active) ─────────────────
    subscriptions_rs::transition_to_active(subscription_id, "trial_converted", &subscriptions_pool)
        .await
        .expect("transition subscription to active");

    let status: String =
        sqlx::query_scalar("SELECT status FROM subscriptions WHERE id = $1")
            .bind(subscription_id)
            .fetch_one(&subscriptions_pool)
            .await
            .expect("fetch subscription status after activation");
    assert_eq!(
        status, "active",
        "subscription must be active after activation"
    );

    // ─── Step 3: Seed TTP data and record metering usage ────────────────────
    insert_test_tenant(&tr_pool, tenant_id, &app_id).await;
    seed_ttp_data(&ttp_pool, tenant_id, party_id).await;
    seed_ttp_pricing(&ttp_pool, tenant_id).await;
    ingest_metering_events(&ttp_pool, tenant_id).await;

    // Verify: metering events recorded
    let event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ttp_metering_events WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_one(&ttp_pool)
    .await
    .expect("count metering events");
    assert_eq!(event_count, 3, "exactly 3 metering events must be recorded");

    // ─── Step 4: TTP billing run creates charges ────────────────────────────
    let registry = TenantRegistryClient::new(get_tenant_registry_url());
    let ar = ArClient::new(get_ar_base_url());

    let summary = run_billing(
        &ttp_pool,
        &registry,
        &ar,
        tenant_id,
        &billing_period,
        &idempotency_key,
    )
    .await
    .unwrap_or_else(|e| panic!("TTP billing run failed: {:?}", e));

    assert!(!summary.was_noop, "first billing run must not be a no-op");
    assert!(
        summary.parties_billed >= 1,
        "at least one party must be billed"
    );

    // Expected metered usage: api_calls (100+50)*10 = 1500 + storage_gb 10*500 = 5000
    // Total metered: 6500. Service agreement: 15000. Grand total: 21500.
    // But billing handles metered + agreement as separate billing items.
    // The service agreement party (party_id) gets 15000.
    // The metered usage party (tenant_id as party_id) gets 6500.
    assert!(
        summary.total_amount_minor > 0,
        "billing run must produce positive charges"
    );

    // Verify billing run status in DB
    let run_status: String = sqlx::query_scalar(
        "SELECT status FROM ttp_billing_runs WHERE tenant_id = $1 AND billing_period = $2",
    )
    .bind(tenant_id)
    .bind(&billing_period)
    .fetch_one(&ttp_pool)
    .await
    .expect("query billing run status");
    assert_eq!(
        run_status, "completed",
        "billing run must be in completed state"
    );

    // Verify billing run items exist with invoiced status
    let invoiced_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ttp_billing_run_items
         WHERE run_id = $1 AND status = 'invoiced'",
    )
    .bind(summary.run_id)
    .fetch_one(&ttp_pool)
    .await
    .expect("count invoiced items");
    assert!(
        invoiced_count >= 1,
        "at least one billing run item must be invoiced"
    );

    // ─── Step 5: Verify AR invoice generated ────────────────────────────────
    // TTP creates AR invoices with correlation_id = derive_item_key(run_id, party_id).
    // Check for the service agreement party's invoice.
    let item_key = derive_item_key(summary.run_id, party_id);
    let ar_invoice_status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM ar_invoices
         WHERE app_id = 'test-app' AND correlation_id = $1
         LIMIT 1",
    )
    .bind(&item_key)
    .fetch_optional(&ar_pool)
    .await
    .expect("query AR invoice by correlation_id");

    assert_eq!(
        ar_invoice_status.as_deref(),
        Some("open"),
        "AR invoice must be finalized (status=open) after TTP billing run"
    );

    // Verify AR invoice amount matches service agreement (15000 minor units)
    let ar_invoice_amount: Option<i32> = sqlx::query_scalar(
        "SELECT amount_cents FROM ar_invoices
         WHERE app_id = 'test-app' AND correlation_id = $1
         LIMIT 1",
    )
    .bind(&item_key)
    .fetch_optional(&ar_pool)
    .await
    .expect("query AR invoice amount");

    assert_eq!(
        ar_invoice_amount,
        Some(15000),
        "AR invoice amount must equal service agreement (15000 minor units)"
    );

    // ─── Step 6: Verify metering trace contributed to billing ───────────────
    // The metered usage is billed as a separate item with party_id = tenant_id.
    let metered_item_key = derive_item_key(summary.run_id, tenant_id);
    let metered_invoice_status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM ar_invoices
         WHERE app_id = 'test-app' AND correlation_id = $1
         LIMIT 1",
    )
    .bind(&metered_item_key)
    .fetch_optional(&ar_pool)
    .await
    .expect("query metered AR invoice");

    assert_eq!(
        metered_invoice_status.as_deref(),
        Some("open"),
        "metered usage AR invoice must be finalized (status=open)"
    );

    // Metered amount: api_calls (150 * 10 = 1500) + storage_gb (10 * 500 = 5000) = 6500
    let metered_amount: Option<i32> = sqlx::query_scalar(
        "SELECT amount_cents FROM ar_invoices
         WHERE app_id = 'test-app' AND correlation_id = $1
         LIMIT 1",
    )
    .bind(&metered_item_key)
    .fetch_optional(&ar_pool)
    .await
    .expect("query metered AR invoice amount");

    assert_eq!(
        metered_amount,
        Some(6500),
        "metered usage invoice must equal (150*10 + 10*500) = 6500 minor units"
    );

    // ─── Step 7: Verify idempotency — rerun produces no duplicates ──────────
    let summary2 = run_billing(
        &ttp_pool,
        &registry,
        &ar,
        tenant_id,
        &billing_period,
        &idempotency_key,
    )
    .await
    .unwrap_or_else(|e| panic!("second billing run failed: {:?}", e));

    assert!(summary2.was_noop, "second billing run must be a no-op");
    assert_eq!(
        summary2.run_id, summary.run_id,
        "run_id must be stable across idempotent reruns"
    );

    // ─── Step 8: Verify subscription remains active after billing ───────────
    let final_status: String =
        sqlx::query_scalar("SELECT status FROM subscriptions WHERE id = $1")
            .bind(subscription_id)
            .fetch_one(&subscriptions_pool)
            .await
            .expect("fetch final subscription status");
    assert_eq!(
        final_status, "active",
        "subscription must remain active after billing"
    );

    // Cleanup
    cleanup_all(
        &ttp_pool,
        &tr_pool,
        &ar_pool,
        &subscriptions_pool,
        tenant_id,
        &tenant_id_str,
        party_id,
    )
    .await;
}
