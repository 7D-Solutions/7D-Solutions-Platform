//! Stress test: Double-spend — 50 concurrent AP allocations prove budget isolation
//!
//! Proves that under 50 concurrent allocation attempts against a single vendor
//! bill (the "budget"), the conservation invariant holds: sum(allocated) <= total.
//! Exactly 1 request succeeds (allocating the full amount); the remaining 49 fail
//! with a clean business error (bill status is now 'paid', blocking further allocation).
//! No request should produce a 500/panic.
//!
//! The AP allocation service uses `SELECT … FOR UPDATE` on the vendor_bills row,
//! serializing concurrent transactions at the row lock. This test proves that
//! concurrency cannot overdraw the budget.
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- double_spend_budget_e2e --nocapture
//! ```

use ap::domain::allocations::{
    service::apply_allocation, AllocationError, AllocationRecord, CreateAllocationRequest,
};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

const CONCURRENCY: usize = 50;
const BUDGET_AMOUNT_MINOR: i64 = 100_000; // $1,000.00

fn get_ap_db_url() -> String {
    std::env::var("AP_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "postgresql://ap_user:ap_pass@localhost:5443/ap_db".to_string())
}

async fn get_ap_pool() -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(55) // > CONCURRENCY so all tasks get a connection
        .acquire_timeout(Duration::from_secs(30))
        .connect(&get_ap_db_url())
        .await
        .expect("failed to connect to AP database");

    sqlx::migrate!("../modules/ap/db/migrations")
        .run(&pool)
        .await
        .expect("failed to run AP migrations");

    pool
}

fn generate_test_tenant() -> String {
    format!("stress-budget-{}", Uuid::new_v4())
}

async fn create_vendor(pool: &PgPool, tenant_id: &str) -> Uuid {
    let vendor_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO vendors (vendor_id, tenant_id, name, currency, payment_terms_days,
           is_active, created_at, updated_at)
           VALUES ($1, $2, 'Stress Budget Vendor', 'USD', 30, TRUE, NOW(), NOW())"#,
    )
    .bind(vendor_id)
    .bind(tenant_id)
    .execute(pool)
    .await
    .expect("failed to create vendor");
    vendor_id
}

async fn create_approved_bill(
    pool: &PgPool,
    tenant_id: &str,
    vendor_id: Uuid,
    total_minor: i64,
) -> Uuid {
    let bill_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO vendor_bills (
            bill_id, tenant_id, vendor_id, vendor_invoice_ref, currency,
            total_minor, invoice_date, due_date, status, entered_by
        )
        VALUES ($1, $2, $3, $4, 'USD', $5, NOW(), NOW() + INTERVAL '30 days', 'approved', 'stress-test')"#,
    )
    .bind(bill_id)
    .bind(tenant_id)
    .bind(vendor_id)
    .bind(format!("STRESS-INV-{}", Uuid::new_v4()))
    .bind(total_minor)
    .execute(pool)
    .await
    .expect("failed to create approved bill");

    sqlx::query(
        r#"INSERT INTO bill_lines (line_id, bill_id, description, quantity, unit_price_minor, line_total_minor, gl_account_code)
        VALUES ($1, $2, 'Budget line', 1.0, $3, $3, '6100')"#,
    )
    .bind(Uuid::new_v4())
    .bind(bill_id)
    .bind(total_minor)
    .execute(pool)
    .await
    .expect("failed to create bill line");

    bill_id
}

async fn cleanup_tenant(pool: &PgPool, tenant_id: &str) {
    for q in [
        "DELETE FROM ap_allocations WHERE tenant_id = $1",
        "DELETE FROM bill_lines WHERE bill_id IN (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM vendor_bills WHERE tenant_id = $1",
        "DELETE FROM events_outbox WHERE tenant_id = $1",
        "DELETE FROM vendors WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(pool).await.ok();
    }
}

#[derive(Debug)]
struct AllocationOutcome {
    allocated_minor: i64,
    is_over_allocation: bool,
    is_invalid_status: bool,
    is_unexpected_error: bool,
    error_msg: Option<String>,
}

impl AllocationOutcome {
    fn from_result(result: Result<AllocationRecord, AllocationError>) -> Self {
        match result {
            Ok(record) => AllocationOutcome {
                allocated_minor: record.amount_minor,
                is_over_allocation: false,
                is_invalid_status: false,
                is_unexpected_error: false,
                error_msg: None,
            },
            Err(AllocationError::OverAllocation { .. }) => AllocationOutcome {
                allocated_minor: 0,
                is_over_allocation: true,
                is_invalid_status: false,
                is_unexpected_error: false,
                error_msg: None,
            },
            Err(AllocationError::InvalidBillStatus(_)) => AllocationOutcome {
                allocated_minor: 0,
                is_over_allocation: false,
                is_invalid_status: true,
                is_unexpected_error: false,
                error_msg: None,
            },
            Err(e) => AllocationOutcome {
                allocated_minor: 0,
                is_over_allocation: false,
                is_invalid_status: false,
                is_unexpected_error: true,
                error_msg: Some(format!("{}", e)),
            },
        }
    }
}

#[tokio::test]
async fn double_spend_budget_e2e() {
    let pool = Arc::new(get_ap_pool().await);
    let tenant_id = Arc::new(generate_test_tenant());

    // --- Seed: vendor + approved bill as the "budget" ---
    let vendor_id = create_vendor(&pool, &tenant_id).await;
    let bill_id = create_approved_bill(&pool, &tenant_id, vendor_id, BUDGET_AMOUNT_MINOR).await;

    println!(
        "seeded: tenant={}, vendor={}, bill={} (budget=${:.2})",
        tenant_id,
        vendor_id,
        bill_id,
        BUDGET_AMOUNT_MINOR as f64 / 100.0
    );

    // --- Fire 50 concurrent allocations, each requesting the full budget ---
    // Total attempted = 50 * $1,000 = $50,000 >> $1,000 budget.
    // SELECT FOR UPDATE serializes: exactly 1 succeeds, 49 rejected.
    println!(
        "\n--- {} concurrent allocations of ${:.2} against ${:.2} budget ---",
        CONCURRENCY,
        BUDGET_AMOUNT_MINOR as f64 / 100.0,
        BUDGET_AMOUNT_MINOR as f64 / 100.0,
    );

    let start = Instant::now();

    let handles: Vec<_> = (0..CONCURRENCY)
        .map(|_| {
            let pool = Arc::clone(&pool);
            let tenant_id = Arc::clone(&tenant_id);
            tokio::spawn(async move {
                let req = CreateAllocationRequest {
                    allocation_id: Uuid::new_v4(),
                    amount_minor: BUDGET_AMOUNT_MINOR,
                    currency: "USD".to_string(),
                    payment_run_id: None,
                };
                AllocationOutcome::from_result(
                    apply_allocation(&pool, &tenant_id, bill_id, &req).await,
                )
            })
        })
        .collect();

    let mut outcomes = Vec::with_capacity(CONCURRENCY);
    for h in handles {
        outcomes.push(h.await.expect("task panicked"));
    }
    let elapsed = start.elapsed();

    // --- Analyze results ---
    let success_count = outcomes.iter().filter(|o| o.allocated_minor > 0).count();
    let over_alloc_count = outcomes.iter().filter(|o| o.is_over_allocation).count();
    let invalid_status_count = outcomes.iter().filter(|o| o.is_invalid_status).count();
    let unexpected_error_count = outcomes.iter().filter(|o| o.is_unexpected_error).count();
    let total_allocated_from_responses: i64 =
        outcomes.iter().map(|o| o.allocated_minor).sum();
    let clean_rejection_count = over_alloc_count + invalid_status_count;

    println!("completed in {:?}", elapsed);
    println!("  successful allocations: {}", success_count);
    println!(
        "  clean rejections: {} (over_allocation={}, invalid_status={})",
        clean_rejection_count, over_alloc_count, invalid_status_count
    );
    println!("  unexpected errors: {}", unexpected_error_count);
    println!(
        "  total allocated from responses: ${:.2}",
        total_allocated_from_responses as f64 / 100.0
    );

    for (i, o) in outcomes.iter().enumerate() {
        if o.is_unexpected_error {
            println!(
                "  request {}: UNEXPECTED ERROR: {}",
                i,
                o.error_msg.as_deref().unwrap_or("unknown")
            );
        }
    }

    // --- Assertion 1: No unexpected errors ---
    assert_eq!(
        unexpected_error_count, 0,
        "no unexpected errors — all rejections should be OverAllocation or InvalidBillStatus"
    );

    // --- Assertion 2: Exactly 1 succeeded ---
    assert_eq!(
        success_count, 1,
        "exactly 1 allocation must succeed (got {})",
        success_count
    );

    // --- Assertion 3: All others rejected cleanly ---
    assert_eq!(
        clean_rejection_count,
        CONCURRENCY - 1,
        "expected {} clean rejections, got {}",
        CONCURRENCY - 1,
        clean_rejection_count
    );

    // --- Assertion 4: Response-level conservation ---
    assert_eq!(
        total_allocated_from_responses, BUDGET_AMOUNT_MINOR,
        "total allocated from responses (${:.2}) must equal budget (${:.2})",
        total_allocated_from_responses as f64 / 100.0,
        BUDGET_AMOUNT_MINOR as f64 / 100.0
    );

    // --- Assertion 5: DB-level conservation (canonical truth) ---
    let db_total_allocated: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount_minor), 0)::BIGINT FROM ap_allocations WHERE bill_id = $1 AND tenant_id = $2",
    )
    .bind(bill_id)
    .bind(tenant_id.as_ref())
    .fetch_one(pool.as_ref())
    .await
    .expect("failed to query allocation total");

    println!(
        "\n  DB total allocated: ${:.2}",
        db_total_allocated as f64 / 100.0
    );

    assert_eq!(
        db_total_allocated, BUDGET_AMOUNT_MINOR,
        "DB total (${:.2}) must equal budget (${:.2}) — no overdraw",
        db_total_allocated as f64 / 100.0,
        BUDGET_AMOUNT_MINOR as f64 / 100.0
    );

    // --- Assertion 6: No negative balance ---
    let remaining = BUDGET_AMOUNT_MINOR - db_total_allocated;
    assert!(
        remaining >= 0,
        "remaining balance must be >= 0, got ${:.2}",
        remaining as f64 / 100.0
    );
    assert_eq!(
        remaining, 0,
        "budget should be fully consumed, remaining=${:.2}",
        remaining as f64 / 100.0
    );

    // --- Assertion 7: Bill status is 'paid' (budget fully consumed) ---
    let (bill_status,): (String,) =
        sqlx::query_as("SELECT status FROM vendor_bills WHERE bill_id = $1 AND tenant_id = $2")
            .bind(bill_id)
            .bind(tenant_id.as_ref())
            .fetch_one(pool.as_ref())
            .await
            .expect("failed to query bill status");

    assert_eq!(
        bill_status, "paid",
        "bill must be 'paid' after full allocation, got '{}'",
        bill_status
    );

    // --- Assertion 8: Exactly 1 allocation row in DB ---
    let db_alloc_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::BIGINT FROM ap_allocations WHERE bill_id = $1 AND tenant_id = $2",
    )
    .bind(bill_id)
    .bind(tenant_id.as_ref())
    .fetch_one(pool.as_ref())
    .await
    .expect("failed to count allocations");

    assert_eq!(
        db_alloc_count, 1,
        "exactly 1 allocation row expected in DB, got {}",
        db_alloc_count
    );

    // --- Post-burst health check ---
    println!("\n  bill status: {}", bill_status);
    println!("  remaining balance: ${:.2}", remaining as f64 / 100.0);
    println!("  allocation rows: {}", db_alloc_count);
    println!("  conservation invariant: PASSED");
    println!("  budget isolation: PASSED");

    cleanup_tenant(pool.as_ref(), &tenant_id).await;
}
