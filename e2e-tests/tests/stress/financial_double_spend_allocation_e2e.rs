//! Stress test: Financial double-spend — 50 concurrent allocations cannot exceed invoice balance
//!
//! Proves that under 50 concurrent payment allocation attempts against a single
//! invoice, the conservation invariant holds: sum(allocated) <= invoice_balance.
//! No allocation request should produce a 500/panic — rejected attempts return
//! cleanly with zero allocation (SKIP LOCKED skips the locked invoice row).
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- financial_double_spend_allocation_e2e --nocapture
//! ```

use ar_rs::payment_allocation::{allocate_payment_fifo, AllocatePaymentRequest};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

fn get_ar_db_url() -> String {
    std::env::var("AR_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://ar_user:ar_pass@localhost:5434/ar_db".to_string())
}

async fn get_ar_pool() -> PgPool {
    PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&get_ar_db_url())
        .await
        .expect("failed to connect to AR database")
}

fn generate_test_tenant() -> String {
    format!("test-tenant-{}", Uuid::new_v4())
}

const CONCURRENCY: usize = 50;
const INVOICE_AMOUNT_CENTS: i64 = 100_000; // $1,000.00
const ALLOCATION_AMOUNT_CENTS: i64 = 5_000; // $50.00 per attempt

/// Insert a test customer and return its ID.
async fn create_customer(pool: &PgPool, tenant_id: &str) -> i32 {
    sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
        VALUES ($1, $2, 'Stress Test Customer', 'active', 0, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("stress-alloc-{}@test.local", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("failed to create customer")
}

/// Insert a test invoice and return its ID.
async fn create_invoice(
    pool: &PgPool,
    tenant_id: &str,
    customer_id: i32,
    amount_cents: i64,
) -> i32 {
    sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status, amount_cents, currency,
            due_at, created_at, updated_at
        )
        VALUES ($1, $2, $3, 'open', $4, 'usd', NOW(), NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("stress_inv_{}", Uuid::new_v4()))
    .bind(customer_id)
    .bind(amount_cents)
    .fetch_one(pool)
    .await
    .expect("failed to create invoice")
}

/// Run the payment allocations migration (idempotent).
async fn run_alloc_migration(pool: &PgPool) {
    let sql = include_str!(
        "../../../modules/ar/db/migrations/20260217000008_create_payment_allocations.sql"
    );
    sqlx::raw_sql(sql)
        .execute(pool)
        .await
        .expect("failed to run payment_allocations migration");
}

/// Clean up all test data for a tenant (reverse FK order).
async fn cleanup_tenant(pool: &PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM ar_payment_allocations WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM events_outbox WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

#[derive(Debug)]
struct AllocationOutcome {
    allocated_cents: i64,
    is_error: bool,
    error_msg: Option<String>,
}

#[tokio::test]
async fn financial_double_spend_allocation_e2e() {
    let pool = get_ar_pool().await;
    run_alloc_migration(&pool).await;
    let tenant_id = generate_test_tenant();

    // --- Seed: customer + single invoice for $1,000 ---
    let customer_id = create_customer(&pool, &tenant_id).await;
    let invoice_id = create_invoice(&pool, &tenant_id, customer_id, INVOICE_AMOUNT_CENTS).await;
    println!(
        "seeded: tenant={}, customer={}, invoice={} (${:.2})",
        tenant_id,
        customer_id,
        invoice_id,
        INVOICE_AMOUNT_CENTS as f64 / 100.0
    );

    // --- Fire 50 concurrent allocation attempts ---
    // Each tries to allocate $50 against the same invoice.
    // Total attempted = $2,500 > $1,000 invoice balance.
    println!(
        "\n--- {} concurrent allocations of ${:.2} (total attempted: ${:.2}) ---",
        CONCURRENCY,
        ALLOCATION_AMOUNT_CENTS as f64 / 100.0,
        (CONCURRENCY as i64 * ALLOCATION_AMOUNT_CENTS) as f64 / 100.0
    );

    let pool = Arc::new(pool);
    let tenant_id = Arc::new(tenant_id);
    let start = Instant::now();

    let handles: Vec<_> = (0..CONCURRENCY)
        .map(|i| {
            let pool = Arc::clone(&pool);
            let tenant_id = Arc::clone(&tenant_id);
            tokio::spawn(async move {
                let req = AllocatePaymentRequest {
                    payment_id: format!("stress_pay_{}_{}", i, Uuid::new_v4()),
                    customer_id,
                    amount_cents: ALLOCATION_AMOUNT_CENTS,
                    currency: "usd".to_string(),
                    idempotency_key: Uuid::new_v4().to_string(),
                };

                match allocate_payment_fifo(&pool, &tenant_id, &req).await {
                    Ok(result) => AllocationOutcome {
                        allocated_cents: result.allocated_amount_cents,
                        is_error: false,
                        error_msg: None,
                    },
                    Err(e) => AllocationOutcome {
                        allocated_cents: 0,
                        is_error: true,
                        error_msg: Some(format!("{}", e)),
                    },
                }
            })
        })
        .collect();

    let mut outcomes = Vec::with_capacity(CONCURRENCY);
    for h in handles {
        outcomes.push(h.await.expect("task panicked"));
    }
    let elapsed = start.elapsed();

    // --- Analyze results ---
    let total_allocated_from_responses: i64 =
        outcomes.iter().map(|o| o.allocated_cents as i64).sum();
    let successful_count = outcomes.iter().filter(|o| o.allocated_cents > 0).count();
    let zero_count = outcomes
        .iter()
        .filter(|o| o.allocated_cents == 0 && !o.is_error)
        .count();
    let error_count = outcomes.iter().filter(|o| o.is_error).count();

    println!("completed in {:?}", elapsed);
    println!(
        "  successful allocations: {} (allocated > 0)",
        successful_count
    );
    println!("  zero allocations (SKIP LOCKED): {}", zero_count);
    println!("  errors: {}", error_count);
    println!(
        "  total allocated from responses: ${:.2}",
        total_allocated_from_responses as f64 / 100.0
    );

    for (i, o) in outcomes.iter().enumerate() {
        if o.is_error {
            println!(
                "  request {}: ERROR: {}",
                i,
                o.error_msg.as_deref().unwrap_or("unknown")
            );
        }
    }

    // --- Assertion 1: No database errors (500-class) ---
    let db_errors = outcomes
        .iter()
        .filter(|o| {
            o.is_error
                && o.error_msg
                    .as_deref()
                    .map_or(false, |m| m.starts_with("database error"))
        })
        .count();
    assert_eq!(
        db_errors, 0,
        "no database errors expected — all rejections should be clean"
    );

    // --- Assertion 2: Response-level conservation invariant ---
    assert!(
        total_allocated_from_responses <= INVOICE_AMOUNT_CENTS as i64,
        "CONSERVATION VIOLATION (responses): total allocated ${:.2} exceeds invoice ${:.2}",
        total_allocated_from_responses as f64 / 100.0,
        INVOICE_AMOUNT_CENTS as f64 / 100.0
    );

    // --- Assertion 3: DB-level conservation invariant (canonical truth) ---
    let db_total_allocated: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount_cents), 0)::BIGINT FROM ar_payment_allocations WHERE invoice_id = $1",
    )
    .bind(invoice_id)
    .fetch_one(pool.as_ref())
    .await
    .expect("failed to query allocation total");

    println!(
        "\n  DB total allocated for invoice {}: ${:.2}",
        invoice_id,
        db_total_allocated as f64 / 100.0
    );

    assert!(
        db_total_allocated <= INVOICE_AMOUNT_CENTS as i64,
        "CONSERVATION VIOLATION (DB): total allocated ${:.2} exceeds invoice ${:.2}",
        db_total_allocated as f64 / 100.0,
        INVOICE_AMOUNT_CENTS as f64 / 100.0
    );

    assert!(
        db_total_allocated >= 0,
        "total allocated must be non-negative, got {}",
        db_total_allocated
    );

    // --- Assertion 4: At least one allocation succeeded ---
    assert!(
        successful_count > 0,
        "at least one allocation must succeed (got 0 out of {})",
        CONCURRENCY
    );

    // --- Assertion 5: Response totals match DB totals ---
    assert_eq!(
        total_allocated_from_responses,
        db_total_allocated,
        "response total (${:.2}) must match DB total (${:.2})",
        total_allocated_from_responses as f64 / 100.0,
        db_total_allocated as f64 / 100.0
    );

    // --- Assertion 6: On-hand balance is non-negative ---
    let remaining = INVOICE_AMOUNT_CENTS as i64 - db_total_allocated;
    assert!(
        remaining >= 0,
        "invoice remaining balance must be >= 0, got ${:.2}",
        remaining as f64 / 100.0
    );

    println!(
        "\n  invoice remaining balance: ${:.2}",
        remaining as f64 / 100.0
    );
    println!("  conservation invariant: PASSED");

    cleanup_tenant(pool.as_ref(), &tenant_id).await;
}
