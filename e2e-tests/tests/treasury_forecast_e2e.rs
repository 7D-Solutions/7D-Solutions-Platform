//! E2E: Treasury cash forecast — AR/AP aging inputs, compute projection (bd-1hiv)
//!
//! Proves the treasury cash forecast computation works end-to-end:
//!   1. Seed AR invoices (current + 31-60 overdue) → refresh aging projection
//!   2. Seed AP bill (current, approved) and a pending payment run
//!   3. Call read_ar_aging / read_ap_aging / read_scheduled_payments from treasury domain
//!   4. Compute forecast and assert time-bucket grouping, currency grouping, and assumptions
//!
//! No mocks. Real AR Postgres (port 5434) and AP Postgres (port 5443).
//!
//! Run: ./scripts/cargo-slot.sh test -p e2e-tests -- treasury_forecast_e2e --nocapture

mod common;

use chrono::Utc;
use common::{generate_test_tenant, get_ap_pool, get_ar_pool};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

use ar_rs::aging::refresh_aging;
use treasury::domain::reports::forecast::{
    compute_forecast, read_ap_aging, read_ar_aging, read_scheduled_payments,
};
use treasury::domain::reports::assumptions::ForecastAssumptions;

// ============================================================================
// AR seed helpers
// ============================================================================

async fn ar_make_customer(pool: &PgPool, tenant_id: &str) -> i32 {
    sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
        VALUES ($1, $2, $3, 'active', 0, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("forecast-{}@test.local", Uuid::new_v4()))
    .bind(format!("Forecast Test {}", tenant_id))
    .fetch_one(pool)
    .await
    .expect("create AR customer")
}

/// Insert an open invoice. positive `due_offset_days` = future; negative = overdue.
async fn ar_make_invoice(
    pool: &PgPool,
    tenant_id: &str,
    customer_id: i32,
    amount_cents: i64,
    due_offset_days: i64,
) -> i32 {
    let due_expr = if due_offset_days >= 0 {
        format!("NOW() + INTERVAL '{} days'", due_offset_days)
    } else {
        format!("NOW() - INTERVAL '{} days'", due_offset_days.unsigned_abs())
    };
    sqlx::query_scalar::<_, i32>(&format!(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status,
            amount_cents, currency, due_at, created_at, updated_at
        )
        VALUES ($1, $2, $3, 'open', $4, 'USD', {}, NOW(), NOW())
        RETURNING id
        "#,
        due_expr
    ))
    .bind(tenant_id)
    .bind(format!("inv_{}", Uuid::new_v4()))
    .bind(customer_id)
    .bind(amount_cents)
    .fetch_one(pool)
    .await
    .expect("create AR invoice")
}

async fn ar_cleanup(pool: &PgPool, tenant_id: &str) {
    for q in [
        "DELETE FROM ar_aging_buckets WHERE app_id = $1",
        "DELETE FROM events_outbox WHERE tenant_id = $1",
        "DELETE FROM ar_invoice_line_items WHERE app_id = $1",
        "DELETE FROM ar_charges WHERE app_id = $1",
        "DELETE FROM ar_payment_allocations WHERE app_id = $1",
        "DELETE FROM ar_invoices WHERE app_id = $1",
        "DELETE FROM ar_customers WHERE app_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(pool).await.ok();
    }
}

// ============================================================================
// AP seed helpers
// ============================================================================

async fn ap_make_vendor(pool: &PgPool, tenant_id: &str) -> Uuid {
    let vendor_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO vendors (vendor_id, tenant_id, name, currency, payment_terms_days, \
         is_active, created_at, updated_at) \
         VALUES ($1, $2, $3, 'USD', 30, TRUE, NOW(), NOW())",
    )
    .bind(vendor_id)
    .bind(tenant_id)
    .bind(format!("Forecast-Vendor-{}", &vendor_id.to_string()[..8]))
    .execute(pool)
    .await
    .expect("create AP vendor");
    vendor_id
}

/// Insert an approved vendor bill with a specific due date offset.
async fn ap_make_approved_bill(
    pool: &PgPool,
    tenant_id: &str,
    vendor_id: Uuid,
    total_minor: i64,
    due_offset_days: i64,
) -> Uuid {
    let bill_id = Uuid::new_v4();
    let due_expr = if due_offset_days >= 0 {
        format!("NOW() + INTERVAL '{} days'", due_offset_days)
    } else {
        format!("NOW() - INTERVAL '{} days'", due_offset_days.unsigned_abs())
    };
    sqlx::query(&format!(
        r#"
        INSERT INTO vendor_bills (
            bill_id, tenant_id, vendor_id, vendor_invoice_ref,
            currency, total_minor, invoice_date, due_date,
            status, entered_by
        )
        VALUES ($1, $2, $3, $4, 'USD', $5, NOW(), {}, 'approved', 'e2e-forecast')
        "#,
        due_expr
    ))
    .bind(bill_id)
    .bind(tenant_id)
    .bind(vendor_id)
    .bind(format!("INV-FCST-{}", Uuid::new_v4()))
    .bind(total_minor)
    .execute(pool)
    .await
    .expect("create AP vendor bill");
    bill_id
}

/// Insert a pending payment run directly.
async fn ap_make_pending_payment_run(
    pool: &PgPool,
    tenant_id: &str,
    total_minor: i64,
) -> Uuid {
    let run_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO payment_runs (
            run_id, tenant_id, total_minor, currency,
            scheduled_date, payment_method, status, created_by
        )
        VALUES ($1, $2, $3, 'USD', NOW() + INTERVAL '1 day', 'ach', 'pending', 'e2e-forecast')
        "#,
    )
    .bind(run_id)
    .bind(tenant_id)
    .bind(total_minor)
    .execute(pool)
    .await
    .expect("create pending payment run");
    run_id
}

async fn ap_cleanup(pool: &PgPool, tenant_id: &str) {
    for q in [
        "DELETE FROM payment_run_executions WHERE run_id IN \
         (SELECT run_id FROM payment_runs WHERE tenant_id = $1)",
        "DELETE FROM payment_run_items WHERE run_id IN \
         (SELECT run_id FROM payment_runs WHERE tenant_id = $1)",
        "DELETE FROM payment_runs WHERE tenant_id = $1",
        "DELETE FROM ap_allocations WHERE bill_id IN \
         (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM bill_lines WHERE bill_id IN \
         (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM vendor_bills WHERE tenant_id = $1",
        "DELETE FROM vendors WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(pool).await.ok();
    }
}

// ============================================================================
// Tests
// ============================================================================

/// Full forecast: AR aging inflows + AP bill outflows + scheduled payment run.
///
/// Seed:
///   AR: $10,000 USD current (due +5 days) + $5,000 USD 40-day overdue (days_31_60)
///   AP: $3,000 USD approved bill, current (due +10 days)
///   AP: $2,000 USD pending payment run
///
/// Expected (with default assumption rates):
///   inflows.current_minor   = 10_000 * 0.95 = 9_500
///   inflows.days_31_60_minor = 5_000 * 0.70 = 3_500
///   inflows.total_minor     = 13_000
///   outflows.current_minor  = 3_000 * 1.0  = 3_000
///   outflows.total_minor    = 3_000
///   scheduled_outflows      = 2_000 * 1.0  = 2_000
///   total_net_minor         = 13_000 - 3_000 - 2_000 = 8_000
///   data_sources            ⊇ ["ar_aging_buckets", "ap_vendor_bills", "ap_payment_runs"]
#[tokio::test]
#[serial]
async fn test_treasury_forecast_e2e() {
    let ar_pool = get_ar_pool().await;
    let ap_pool = get_ap_pool().await;
    let tenant = generate_test_tenant();

    // Cleanup before seeding (idempotent)
    ar_cleanup(&ar_pool, &tenant).await;
    ap_cleanup(&ap_pool, &tenant).await;

    // --- Step 1: Seed AR aging ---

    let customer_id = ar_make_customer(&ar_pool, &tenant).await;

    // Invoice 1: $10,000, due in 5 days → "current" bucket
    ar_make_invoice(&ar_pool, &tenant, customer_id, 10_000, 5).await;
    // Invoice 2: $5,000, 40 days overdue → "days_31_60" bucket
    ar_make_invoice(&ar_pool, &tenant, customer_id, 5_000, -40).await;

    // Refresh the AR aging projection → populates ar_aging_buckets
    let snapshot = refresh_aging(&ar_pool, &tenant, customer_id)
        .await
        .expect("refresh_aging failed");

    assert_eq!(snapshot.current_minor, 10_000, "AR current bucket");
    assert_eq!(snapshot.days_31_60_minor, 5_000, "AR 31-60 bucket");
    assert_eq!(snapshot.total_outstanding_minor, 15_000, "AR total outstanding");

    // --- Step 2: Seed AP bill + payment run ---

    let vendor_id = ap_make_vendor(&ap_pool, &tenant).await;

    // Approved bill: $3,000 USD, due in 10 days → "current" outflow
    ap_make_approved_bill(&ap_pool, &tenant, vendor_id, 3_000, 10).await;

    // Pending payment run: $2,000 USD → scheduled outflow
    ap_make_pending_payment_run(&ap_pool, &tenant, 2_000).await;

    // --- Step 3: Read aging cross-module (treasury domain) ---

    let ar_aging = read_ar_aging(&ar_pool, &tenant)
        .await
        .expect("read_ar_aging failed");
    let ap_aging = read_ap_aging(&ap_pool, &tenant)
        .await
        .expect("read_ap_aging failed");
    let scheduled = read_scheduled_payments(&ap_pool, &tenant)
        .await
        .expect("read_scheduled_payments failed");

    assert!(!ar_aging.is_empty(), "AR aging should have data");
    assert!(!ap_aging.is_empty(), "AP aging should have data");
    assert!(!scheduled.is_empty(), "Scheduled payments should have data");

    // Verify raw read values before rates are applied
    let ar_usd = ar_aging.iter().find(|a| a.currency == "USD").expect("USD AR aging");
    assert_eq!(ar_usd.current_minor, 10_000, "AR read: current bucket");
    assert_eq!(ar_usd.days_31_60_minor, 5_000, "AR read: 31-60 bucket");

    let ap_usd = ap_aging.iter().find(|a| a.currency == "USD").expect("USD AP aging");
    assert_eq!(ap_usd.current_minor, 3_000, "AP read: current bucket");

    let sched_usd = scheduled.iter().find(|s| s.currency == "USD").expect("USD scheduled");
    assert_eq!(sched_usd.total_minor, 2_000, "Scheduled: total_minor");

    // --- Step 4: Compute forecast ---

    let assumptions = ForecastAssumptions::default();
    let data_sources = vec![
        "ar_aging_buckets".to_string(),
        "ap_vendor_bills".to_string(),
        "ap_payment_runs".to_string(),
    ];
    let response = compute_forecast(&ar_aging, &ap_aging, &scheduled, &assumptions, data_sources);

    // Criterion 1: Forecast returns inflow/outflow grouped by time bucket and currency
    assert_eq!(response.forecasts.len(), 1, "One currency (USD) in forecast");
    let usd = response.forecasts.iter().find(|f| f.currency == "USD")
        .expect("USD forecast");

    // Criterion 2: AR invoice amounts appear in correct inflow buckets
    assert_eq!(usd.inflows.current_minor, 9_500,
        "Inflow current: 10_000 * 0.95 = 9_500");
    assert_eq!(usd.inflows.days_31_60_minor, 3_500,
        "Inflow 31-60: 5_000 * 0.70 = 3_500");
    assert_eq!(usd.inflows.total_minor, 13_000,
        "Inflow total: 9_500 + 3_500 = 13_000");

    // Criterion 3: AP bill amounts appear in correct outflow bucket
    assert_eq!(usd.outflows.current_minor, 3_000,
        "Outflow current: 3_000 * 1.0 = 3_000");
    assert_eq!(usd.outflows.total_minor, 3_000,
        "Outflow total: 3_000");

    // Scheduled outflow from payment run
    assert_eq!(usd.scheduled_outflows_minor, 2_000,
        "Scheduled outflows: 2_000 * 1.0 = 2_000");

    // Net check
    assert_eq!(usd.total_net_minor, 8_000,
        "Net: 13_000 - 3_000 - 2_000 = 8_000");

    // Criterion 4: Assumptions field is populated and data_sources declared
    assert_eq!(response.assumptions.ar_current_rate, 0.95,
        "Assumptions: AR current rate declared");
    assert!(response.data_sources.contains(&"ar_aging_buckets".to_string()),
        "data_sources includes ar_aging_buckets");
    assert!(response.data_sources.contains(&"ap_vendor_bills".to_string()),
        "data_sources includes ap_vendor_bills");
    assert!(response.data_sources.contains(&"ap_payment_runs".to_string()),
        "data_sources includes ap_payment_runs");
    assert!(!response.methodology.is_empty(), "Methodology note populated");

    println!("✅ Treasury forecast E2E:");
    println!("   USD inflows:  current={}, 31-60={}, total={}",
        usd.inflows.current_minor,
        usd.inflows.days_31_60_minor,
        usd.inflows.total_minor);
    println!("   USD outflows: current={}, total={}",
        usd.outflows.current_minor, usd.outflows.total_minor);
    println!("   Scheduled:    {}", usd.scheduled_outflows_minor);
    println!("   Net:          {}", usd.total_net_minor);
    println!("   Data sources: {:?}", response.data_sources);

    // Cleanup
    ar_cleanup(&ar_pool, &tenant).await;
    ap_cleanup(&ap_pool, &tenant).await;
}

/// Currency grouping: multiple AR currencies produce separate forecast buckets.
#[tokio::test]
#[serial]
async fn test_treasury_forecast_currency_grouping() {
    let ar_pool = get_ar_pool().await;
    let ap_pool = get_ap_pool().await;
    let tenant = generate_test_tenant();

    ar_cleanup(&ar_pool, &tenant).await;
    ap_cleanup(&ap_pool, &tenant).await;

    // NOTE: AR aging buckets are per (app_id, customer_id, currency).
    // The treasury read_ar_aging query aggregates by currency.
    // Since ar_invoices uses a fixed currency column, we seed two customers
    // with different currencies (one USD, one EUR) to force two buckets.
    //
    // However, ar_aging_buckets stores the currency from the invoice.
    // Let's seed: USD customer with 20_000 current, EUR with 10_000 current.

    // Customer A: USD
    let cust_usd = ar_make_customer(&ar_pool, &tenant).await;
    ar_make_invoice(&ar_pool, &tenant, cust_usd, 20_000, 5).await;
    refresh_aging(&ar_pool, &tenant, cust_usd)
        .await
        .expect("refresh USD aging");

    // Customer B: EUR — AR invoices store currency; seed EUR via direct insert
    let cust_eur: i32 = sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
        VALUES ($1, $2, 'EUR Customer', 'active', 0, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(&tenant)
    .bind(format!("forecast-eur-{}@test.local", Uuid::new_v4()))
    .fetch_one(&ar_pool)
    .await
    .expect("create EUR customer");

    // Insert EUR invoice directly to test multi-currency
    sqlx::query(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status,
            amount_cents, currency, due_at, created_at, updated_at
        )
        VALUES ($1, $2, $3, 'open', 10000, 'EUR', NOW() + INTERVAL '5 days', NOW(), NOW())
        "#,
    )
    .bind(&tenant)
    .bind(format!("inv_eur_{}", Uuid::new_v4()))
    .bind(cust_eur)
    .execute(&ar_pool)
    .await
    .expect("create EUR invoice");

    refresh_aging(&ar_pool, &tenant, cust_eur)
        .await
        .expect("refresh EUR aging");

    // Compute forecast with no AP data
    let ar_aging = read_ar_aging(&ar_pool, &tenant)
        .await
        .expect("read_ar_aging");

    let assumptions = ForecastAssumptions::default();
    let response = compute_forecast(&ar_aging, &[], &[], &assumptions, vec!["ar_aging_buckets".to_string()]);

    // Should have 2 currency forecasts (EUR and USD), sorted alphabetically
    assert_eq!(response.forecasts.len(), 2, "Two currencies in forecast");
    assert_eq!(response.forecasts[0].currency, "EUR", "First: EUR");
    assert_eq!(response.forecasts[1].currency, "USD", "Second: USD");

    // EUR: 10_000 * 0.95 = 9_500
    assert_eq!(response.forecasts[0].inflows.current_minor, 9_500, "EUR inflow");
    // USD: 20_000 * 0.95 = 19_000
    assert_eq!(response.forecasts[1].inflows.current_minor, 19_000, "USD inflow");

    println!("✅ Currency grouping: EUR={}, USD={}",
        response.forecasts[0].inflows.current_minor,
        response.forecasts[1].inflows.current_minor);

    ar_cleanup(&ar_pool, &tenant).await;
    ap_cleanup(&ap_pool, &tenant).await;
}

/// AR only: no AP data → forecast has inflows only, zero outflows, assumptions declares AR.
#[tokio::test]
#[serial]
async fn test_treasury_forecast_ar_only() {
    let ar_pool = get_ar_pool().await;
    let tenant = generate_test_tenant();

    ar_cleanup(&ar_pool, &tenant).await;

    let customer_id = ar_make_customer(&ar_pool, &tenant).await;
    ar_make_invoice(&ar_pool, &tenant, customer_id, 8_000, 3).await;
    refresh_aging(&ar_pool, &tenant, customer_id)
        .await
        .expect("refresh aging");

    let ar_aging = read_ar_aging(&ar_pool, &tenant)
        .await
        .expect("read_ar_aging");

    let assumptions = ForecastAssumptions::default();
    let data_sources = vec!["ar_aging_buckets".to_string()];
    let response = compute_forecast(&ar_aging, &[], &[], &assumptions, data_sources);

    let usd = response.forecasts.iter().find(|f| f.currency == "USD")
        .expect("USD forecast");

    // 8_000 * 0.95 = 7_600
    assert_eq!(usd.inflows.current_minor, 7_600, "Inflow current: 8_000 * 0.95");
    assert_eq!(usd.outflows.total_minor, 0, "No outflows");
    assert_eq!(usd.total_net_minor, 7_600, "Net = inflow only");
    assert!(response.data_sources.contains(&"ar_aging_buckets".to_string()),
        "AR data source declared");

    println!("✅ AR-only forecast: inflow={}", usd.inflows.current_minor);

    ar_cleanup(&ar_pool, &tenant).await;
}
