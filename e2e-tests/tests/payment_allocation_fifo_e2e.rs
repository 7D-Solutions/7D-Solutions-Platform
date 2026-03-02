//! E2E Test: Partial Payment Allocation — FIFO Strategy (bd-14f)
//!
//! **Coverage:**
//! 1. Single invoice, exact payment — fully allocated, zero remainder
//! 2. Multiple invoices — FIFO ordering by due_at ASC, partial allocation
//! 3. Overpayment — allocates available invoices, returns unallocated remainder
//! 4. Idempotency — duplicate idempotency_key returns cached result
//! 5. Outbox atomicity — allocation rows + ar.payment_allocated event in one tx
//! 6. Zero open invoices — no allocations, full unallocated
//! 7. Already-allocated invoices excluded — respects prior allocation totals
//!
//! **Pattern:** No Docker, no mocks — uses live AR database via common::get_ar_pool()

mod common;

use ar_rs::aging::refresh_aging;
use ar_rs::payment_allocation::{allocate_payment_fifo, AllocatePaymentRequest};
use common::{generate_test_tenant, get_ar_pool};
use serial_test::serial;
use uuid::Uuid;

// ============================================================================
// Test helpers
// ============================================================================

/// Insert a test customer and return its ID.
async fn create_customer(pool: &sqlx::PgPool, tenant_id: &str) -> i32 {
    sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
        VALUES ($1, $2, 'Alloc Test Customer', 'active', 0, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("alloc-test-{}@test.local", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("failed to create customer")
}

/// Insert a test invoice with optional due_at and return its ID.
async fn create_invoice(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    customer_id: i32,
    amount_cents: i32,
    currency: &str,
    due_at: Option<&str>,
) -> i32 {
    let due = due_at.map(|d| {
        chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .expect("bad date")
            .and_hms_opt(0, 0, 0)
            .unwrap()
    });

    sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status, amount_cents, currency,
            due_at, created_at, updated_at
        )
        VALUES ($1, $2, $3, 'open', $4, $5, $6, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("in_alloc_{}", Uuid::new_v4()))
    .bind(customer_id)
    .bind(amount_cents)
    .bind(currency)
    .bind(due)
    .fetch_one(pool)
    .await
    .expect("failed to create invoice")
}

/// Run the payment allocations migration (idempotent).
async fn run_alloc_migration(pool: &sqlx::PgPool) {
    let sql = include_str!(
        "../../modules/ar/db/migrations/20260217000008_create_payment_allocations.sql"
    );
    sqlx::raw_sql(sql)
        .execute(pool)
        .await
        .expect("failed to run payment_allocations migration");
}

/// Clean up all test data for a tenant (reverse FK order).
async fn cleanup_tenant(pool: &sqlx::PgPool, tenant_id: &str) {
    // Payment allocations
    sqlx::query("DELETE FROM ar_payment_allocations WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    // Outbox
    sqlx::query("DELETE FROM events_outbox WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    // Invoices
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    // Customers
    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: Single invoice, exact payment amount — fully allocated.
#[tokio::test]
#[serial]
async fn test_alloc_single_invoice_exact_amount() {
    let pool = get_ar_pool().await;
    run_alloc_migration(&pool).await;
    let tenant_id = generate_test_tenant();

    let customer = create_customer(&pool, &tenant_id).await;
    let inv_id = create_invoice(
        &pool,
        &tenant_id,
        customer,
        10000,
        "usd",
        Some("2026-01-15"),
    )
    .await;

    let req = AllocatePaymentRequest {
        payment_id: format!("pay_{}", Uuid::new_v4()),
        customer_id: customer,
        amount_cents: 10000,
        currency: "usd".to_string(),
        idempotency_key: Uuid::new_v4().to_string(),
    };

    let result = allocate_payment_fifo(&pool, &tenant_id, &req)
        .await
        .expect("allocation failed");

    assert_eq!(
        result.allocated_amount_cents, 10000,
        "full allocation expected"
    );
    assert_eq!(result.unallocated_amount_cents, 0, "zero remainder");
    assert_eq!(result.strategy, "fifo");
    assert_eq!(result.allocations.len(), 1);
    assert_eq!(result.allocations[0].invoice_id, inv_id);
    assert_eq!(result.allocations[0].amount_cents, 10000);

    // Verify DB row
    let db_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_payment_allocations WHERE app_id = $1")
            .bind(&tenant_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(db_count, 1);

    // Verify outbox event
    let event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1 AND event_type = 'ar.payment_allocated'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(event_count, 1, "exactly one ar.payment_allocated event");

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 2: Multiple invoices — FIFO ordering by due_at, partial allocation.
#[tokio::test]
#[serial]
async fn test_alloc_fifo_ordering_multiple_invoices() {
    let pool = get_ar_pool().await;
    run_alloc_migration(&pool).await;
    let tenant_id = generate_test_tenant();

    let customer = create_customer(&pool, &tenant_id).await;

    // Create invoices with different due dates — FIFO should allocate oldest first
    let inv_old =
        create_invoice(&pool, &tenant_id, customer, 3000, "usd", Some("2026-01-01")).await;
    let inv_mid =
        create_invoice(&pool, &tenant_id, customer, 5000, "usd", Some("2026-02-01")).await;
    let inv_new =
        create_invoice(&pool, &tenant_id, customer, 4000, "usd", Some("2026-03-01")).await;

    // Payment of $70 = $30 (inv_old) + $40 partial (inv_mid)
    let req = AllocatePaymentRequest {
        payment_id: format!("pay_{}", Uuid::new_v4()),
        customer_id: customer,
        amount_cents: 7000,
        currency: "usd".to_string(),
        idempotency_key: Uuid::new_v4().to_string(),
    };

    let result = allocate_payment_fifo(&pool, &tenant_id, &req)
        .await
        .expect("allocation failed");

    assert_eq!(result.allocated_amount_cents, 7000);
    assert_eq!(result.unallocated_amount_cents, 0);
    assert_eq!(result.allocations.len(), 2, "should allocate to 2 invoices");

    // First allocation: oldest invoice, fully allocated
    assert_eq!(result.allocations[0].invoice_id, inv_old);
    assert_eq!(result.allocations[0].amount_cents, 3000);

    // Second allocation: mid invoice, partially allocated ($40 of $50)
    assert_eq!(result.allocations[1].invoice_id, inv_mid);
    assert_eq!(result.allocations[1].amount_cents, 4000);

    // Third invoice (newest) untouched
    let alloc_for_new: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount_cents), 0)::BIGINT FROM ar_payment_allocations WHERE invoice_id = $1",
    )
    .bind(inv_new)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        alloc_for_new, 0,
        "newest invoice should have no allocations"
    );

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 3: Overpayment — allocates all open invoices, returns unallocated remainder.
#[tokio::test]
#[serial]
async fn test_alloc_overpayment_returns_remainder() {
    let pool = get_ar_pool().await;
    run_alloc_migration(&pool).await;
    let tenant_id = generate_test_tenant();

    let customer = create_customer(&pool, &tenant_id).await;
    create_invoice(&pool, &tenant_id, customer, 2000, "usd", Some("2026-01-01")).await;
    create_invoice(&pool, &tenant_id, customer, 3000, "usd", Some("2026-02-01")).await;

    // Payment of $80 > total open of $50
    let req = AllocatePaymentRequest {
        payment_id: format!("pay_{}", Uuid::new_v4()),
        customer_id: customer,
        amount_cents: 8000,
        currency: "usd".to_string(),
        idempotency_key: Uuid::new_v4().to_string(),
    };

    let result = allocate_payment_fifo(&pool, &tenant_id, &req)
        .await
        .expect("allocation failed");

    assert_eq!(
        result.allocated_amount_cents, 5000,
        "total of both invoices"
    );
    assert_eq!(result.unallocated_amount_cents, 3000, "$30 unallocated");
    assert_eq!(result.allocations.len(), 2);

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 4: Idempotency — duplicate idempotency_key returns cached result.
#[tokio::test]
#[serial]
async fn test_alloc_idempotency() {
    let pool = get_ar_pool().await;
    run_alloc_migration(&pool).await;
    let tenant_id = generate_test_tenant();

    let customer = create_customer(&pool, &tenant_id).await;
    create_invoice(&pool, &tenant_id, customer, 5000, "usd", Some("2026-01-15")).await;

    let idem_key = Uuid::new_v4().to_string();
    let req = AllocatePaymentRequest {
        payment_id: format!("pay_{}", Uuid::new_v4()),
        customer_id: customer,
        amount_cents: 5000,
        currency: "usd".to_string(),
        idempotency_key: idem_key.clone(),
    };

    // First call
    let result1 = allocate_payment_fifo(&pool, &tenant_id, &req)
        .await
        .expect("first allocation failed");
    assert_eq!(result1.allocated_amount_cents, 5000);
    assert_eq!(result1.allocations.len(), 1);

    // Second call with same idempotency_key
    let result2 = allocate_payment_fifo(&pool, &tenant_id, &req)
        .await
        .expect("second allocation failed");
    assert_eq!(result2.allocated_amount_cents, 5000, "cached result");
    assert_eq!(result2.allocations.len(), 1, "same allocation count");

    // Only one allocation row in DB (not duplicated)
    let db_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_payment_allocations WHERE app_id = $1")
            .bind(&tenant_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(db_count, 1, "idempotency prevented duplicate row");

    // Only one outbox event (not duplicated)
    let event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1 AND event_type = 'ar.payment_allocated'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(event_count, 1, "idempotency prevented duplicate event");

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 5: Outbox atomicity — allocation rows and event committed together.
#[tokio::test]
#[serial]
async fn test_alloc_outbox_atomicity() {
    let pool = get_ar_pool().await;
    run_alloc_migration(&pool).await;
    let tenant_id = generate_test_tenant();

    let customer = create_customer(&pool, &tenant_id).await;
    create_invoice(&pool, &tenant_id, customer, 7500, "usd", Some("2026-01-15")).await;

    let payment_id = format!("pay_{}", Uuid::new_v4());
    let req = AllocatePaymentRequest {
        payment_id: payment_id.clone(),
        customer_id: customer,
        amount_cents: 7500,
        currency: "usd".to_string(),
        idempotency_key: Uuid::new_v4().to_string(),
    };

    allocate_payment_fifo(&pool, &tenant_id, &req)
        .await
        .expect("allocation failed");

    // Verify allocation row exists
    let alloc_exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_payment_allocations WHERE app_id = $1 AND payment_id = $2",
    )
    .bind(&tenant_id)
    .bind(&payment_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(alloc_exists, 1);

    // Verify outbox event exists with correct metadata
    let event_type: String = sqlx::query_scalar(
        "SELECT event_type FROM events_outbox WHERE tenant_id = $1 AND aggregate_id = $2",
    )
    .bind(&tenant_id)
    .bind(&payment_id)
    .fetch_one(&pool)
    .await
    .expect("outbox event not found");
    assert_eq!(event_type, "ar.payment_allocated");

    // Verify DATA_MUTATION class on outbox event
    let mutation_class: String = sqlx::query_scalar(
        "SELECT mutation_class FROM events_outbox WHERE tenant_id = $1 AND aggregate_id = $2",
    )
    .bind(&tenant_id)
    .bind(&payment_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(mutation_class, "DATA_MUTATION");

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 6: Zero open invoices — full amount unallocated.
#[tokio::test]
#[serial]
async fn test_alloc_no_open_invoices() {
    let pool = get_ar_pool().await;
    run_alloc_migration(&pool).await;
    let tenant_id = generate_test_tenant();

    let customer = create_customer(&pool, &tenant_id).await;
    // No invoices created

    let req = AllocatePaymentRequest {
        payment_id: format!("pay_{}", Uuid::new_v4()),
        customer_id: customer,
        amount_cents: 5000,
        currency: "usd".to_string(),
        idempotency_key: Uuid::new_v4().to_string(),
    };

    let result = allocate_payment_fifo(&pool, &tenant_id, &req)
        .await
        .expect("allocation failed");

    assert_eq!(result.allocated_amount_cents, 0);
    assert_eq!(result.unallocated_amount_cents, 5000);
    assert_eq!(result.allocations.len(), 0);

    // No outbox event when nothing allocated
    let event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1 AND event_type = 'ar.payment_allocated'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(event_count, 0, "no event when nothing allocated");

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 7: Already-allocated invoices respected — second payment sees reduced balance.
#[tokio::test]
#[serial]
async fn test_alloc_respects_prior_allocations() {
    let pool = get_ar_pool().await;
    run_alloc_migration(&pool).await;
    let tenant_id = generate_test_tenant();

    let customer = create_customer(&pool, &tenant_id).await;
    let inv_id = create_invoice(
        &pool,
        &tenant_id,
        customer,
        10000,
        "usd",
        Some("2026-01-15"),
    )
    .await;

    // First payment: $60 against a $100 invoice → $40 remaining
    let req1 = AllocatePaymentRequest {
        payment_id: format!("pay_1_{}", Uuid::new_v4()),
        customer_id: customer,
        amount_cents: 6000,
        currency: "usd".to_string(),
        idempotency_key: Uuid::new_v4().to_string(),
    };

    let result1 = allocate_payment_fifo(&pool, &tenant_id, &req1)
        .await
        .expect("first allocation failed");
    assert_eq!(result1.allocated_amount_cents, 6000);

    // Second payment: $70 — but only $40 remains on the invoice
    let req2 = AllocatePaymentRequest {
        payment_id: format!("pay_2_{}", Uuid::new_v4()),
        customer_id: customer,
        amount_cents: 7000,
        currency: "usd".to_string(),
        idempotency_key: Uuid::new_v4().to_string(),
    };

    let result2 = allocate_payment_fifo(&pool, &tenant_id, &req2)
        .await
        .expect("second allocation failed");
    assert_eq!(
        result2.allocated_amount_cents, 4000,
        "only $40 remaining on invoice"
    );
    assert_eq!(result2.unallocated_amount_cents, 3000, "$30 unallocated");
    assert_eq!(result2.allocations.len(), 1);
    assert_eq!(result2.allocations[0].invoice_id, inv_id);
    assert_eq!(result2.allocations[0].amount_cents, 4000);

    // Total allocations for the invoice should equal invoice amount
    let total_allocated: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount_cents), 0)::BIGINT FROM ar_payment_allocations WHERE invoice_id = $1",
    )
    .bind(inv_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        total_allocated, 10000,
        "invoice fully allocated across two payments"
    );

    cleanup_tenant(&pool, &tenant_id).await;
}

// ============================================================================
// Integrated cross-domain test (bd-b6k)
// ============================================================================

/// Run the aging buckets migration (idempotent).
async fn run_aging_migration(pool: &sqlx::PgPool) {
    let sql =
        include_str!("../../modules/ar/db/migrations/20260217000005_create_ar_aging_buckets.sql");
    sqlx::raw_sql(sql)
        .execute(pool)
        .await
        .expect("failed to run aging migration");
}

/// Clean up aging + allocation data for a tenant.
async fn cleanup_tenant_with_aging(pool: &sqlx::PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM ar_aging_buckets WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    cleanup_tenant(pool, tenant_id).await;
}

/// Test 8 (bd-b6k): Integrated allocation → aging multi-cycle proof.
///
/// Proves the full chain across two payment cycles:
///   Cycle 1: Create 3 invoices in different aging buckets → allocate partial payment
///            via FIFO → refresh aging → verify buckets reflect reduced balances.
///   Cycle 2: Second payment covers remaining balance → refresh aging →
///            total_outstanding drops to zero.
#[tokio::test]
#[serial]
async fn test_integrated_allocation_aging_multi_cycle() {
    let pool = get_ar_pool().await;
    run_alloc_migration(&pool).await;
    run_aging_migration(&pool).await;
    let tenant_id = generate_test_tenant();

    let customer = create_customer(&pool, &tenant_id).await;

    // Create 3 invoices in different aging buckets using dynamic due dates:
    //   Invoice A: $100, due 15 days ago → days_1_30 bucket
    //   Invoice B: $200, due 45 days ago → days_31_60 bucket
    //   Invoice C: $150, due in the future → current bucket
    let due_15_ago = chrono::Utc::now().naive_utc().date() - chrono::Duration::days(15);
    let due_45_ago = chrono::Utc::now().naive_utc().date() - chrono::Duration::days(45);
    let due_future = chrono::Utc::now().naive_utc().date() + chrono::Duration::days(10);

    let inv_a = create_invoice(
        &pool,
        &tenant_id,
        customer,
        10000,
        "usd",
        Some(&due_15_ago.format("%Y-%m-%d").to_string()),
    )
    .await;
    let inv_b = create_invoice(
        &pool,
        &tenant_id,
        customer,
        20000,
        "usd",
        Some(&due_45_ago.format("%Y-%m-%d").to_string()),
    )
    .await;
    let _inv_c = create_invoice(
        &pool,
        &tenant_id,
        customer,
        15000,
        "usd",
        Some(&due_future.format("%Y-%m-%d").to_string()),
    )
    .await;

    // --- Pre-allocation aging baseline ---
    let snapshot_before = refresh_aging(&pool, &tenant_id, customer)
        .await
        .expect("refresh_aging before allocation failed");
    assert_eq!(
        snapshot_before.total_outstanding_minor, 45000,
        "total = $100 + $200 + $150"
    );
    assert_eq!(snapshot_before.invoice_count, 3);

    // --- Cycle 1: Partial payment of $250 via FIFO ---
    // FIFO orders by due_at ASC, so: inv_b ($200, oldest) → inv_a ($100) → inv_c ($150)
    // $250 covers inv_b fully ($200) + $50 of inv_a
    let req1 = AllocatePaymentRequest {
        payment_id: format!("pay_cycle1_{}", Uuid::new_v4()),
        customer_id: customer,
        amount_cents: 25000,
        currency: "usd".to_string(),
        idempotency_key: Uuid::new_v4().to_string(),
    };

    let alloc_result1 = allocate_payment_fifo(&pool, &tenant_id, &req1)
        .await
        .expect("cycle 1 allocation failed");
    assert_eq!(alloc_result1.allocated_amount_cents, 25000);
    assert_eq!(alloc_result1.unallocated_amount_cents, 0);
    assert_eq!(
        alloc_result1.allocations.len(),
        2,
        "covers inv_b fully + inv_a partially"
    );

    // Refresh aging after allocation
    let snapshot_after_cycle1 = refresh_aging(&pool, &tenant_id, customer)
        .await
        .expect("refresh_aging after cycle 1 failed");

    // Total outstanding should be $450 - $250 = $200 remaining
    assert_eq!(
        snapshot_after_cycle1.total_outstanding_minor, 20000,
        "total outstanding after $250 allocation = $200"
    );

    // inv_b ($200, days_31_60) fully covered → 0 in that bucket
    assert_eq!(
        snapshot_after_cycle1.days_31_60_minor, 0,
        "days_31_60 bucket should be 0 (inv_b fully allocated)"
    );

    // inv_a ($100, days_1_30) has $50 remaining after partial allocation
    assert_eq!(
        snapshot_after_cycle1.days_1_30_minor, 5000,
        "days_1_30 bucket should be $50 (inv_a partially allocated)"
    );

    // inv_c ($150, current) untouched
    assert_eq!(
        snapshot_after_cycle1.current_minor, 15000,
        "current bucket should be $150 (inv_c untouched)"
    );

    // --- Cycle 2: Second payment covers remaining $200 ---
    let req2 = AllocatePaymentRequest {
        payment_id: format!("pay_cycle2_{}", Uuid::new_v4()),
        customer_id: customer,
        amount_cents: 20000,
        currency: "usd".to_string(),
        idempotency_key: Uuid::new_v4().to_string(),
    };

    let alloc_result2 = allocate_payment_fifo(&pool, &tenant_id, &req2)
        .await
        .expect("cycle 2 allocation failed");
    assert_eq!(alloc_result2.allocated_amount_cents, 20000);
    assert_eq!(alloc_result2.unallocated_amount_cents, 0);

    // Refresh aging after second allocation
    let snapshot_after_cycle2 = refresh_aging(&pool, &tenant_id, customer)
        .await
        .expect("refresh_aging after cycle 2 failed");

    assert_eq!(
        snapshot_after_cycle2.total_outstanding_minor, 0,
        "all invoices fully covered after two payment cycles"
    );
    assert_eq!(snapshot_after_cycle2.current_minor, 0);
    assert_eq!(snapshot_after_cycle2.days_1_30_minor, 0);
    assert_eq!(snapshot_after_cycle2.days_31_60_minor, 0);
    assert_eq!(snapshot_after_cycle2.days_61_90_minor, 0);
    assert_eq!(snapshot_after_cycle2.days_over_90_minor, 0);

    // Verify outbox: should have ar.payment_allocated events for both cycles
    let alloc_event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1 AND event_type = 'ar.payment_allocated'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        alloc_event_count, 2,
        "two payment_allocated events for two cycles"
    );

    // Verify aging outbox events
    let aging_event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1 AND event_type = 'ar.ar_aging_updated'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        aging_event_count >= 2,
        "at least 2 aging events (one per refresh)"
    );

    cleanup_tenant_with_aging(&pool, &tenant_id).await;
}
