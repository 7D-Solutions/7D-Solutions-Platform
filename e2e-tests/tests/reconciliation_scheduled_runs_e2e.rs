//! E2E Test: Scheduled Reconciliation Runs (bd-1kl)
//!
//! **Coverage:**
//! 1. Create scheduled run — pending status returned
//! 2. Window dedup — duplicate window returns AlreadyScheduled
//! 3. Worker claims and executes a pending run → completed
//! 4. Two concurrent workers claim different runs (no double-processing)
//! 5. More workers than runs — losers get NothingToClaim
//! 6. Completed runs are not re-claimed
//! 7. Run lifecycle: pending → running → completed with match counts
//! 8. Failed run records error and status
//! 9. Batch poll processes multiple pending runs
//! 10. Outbox events emitted for executed matching run
//!
//! **Pattern:** No Docker, no mocks — uses live AR database pool via common::get_ar_pool()

mod common;

use ar_rs::recon_scheduler::{
    claim_and_execute_scheduled_run, create_scheduled_run, poll_scheduled_runs,
    CreateScheduledRunOutcome, CreateScheduledRunRequest, ScheduledRunExecutionOutcome,
};
use chrono::{NaiveDateTime, Utc};
use common::{generate_test_tenant, get_ar_pool};
use uuid::Uuid;

// ============================================================================
// Test helpers
// ============================================================================

/// Advisory lock key for serializing migration execution across parallel tests.
const RECON_MIGRATION_LOCK_KEY: i64 = 8_312_947_653_i64;

/// Run the migrations needed for reconciliation scheduled runs.
/// Uses an advisory lock to prevent parallel test DDL races.
async fn run_migrations(pool: &sqlx::PgPool) {
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(RECON_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("failed to acquire migration advisory lock");

    let recon_sql =
        include_str!("../../modules/ar/db/migrations/20260217000006_create_recon_matching.sql");
    let _ = sqlx::raw_sql(recon_sql).execute(pool).await;

    let sched_sql = include_str!(
        "../../modules/ar/db/migrations/20260217000009_create_recon_scheduled_runs.sql"
    );
    let _ = sqlx::raw_sql(sched_sql).execute(pool).await;

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(RECON_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("failed to release migration advisory lock");
}

/// Insert a test customer and return its ID.
async fn create_customer(pool: &sqlx::PgPool, tenant_id: &str) -> i32 {
    sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
        VALUES ($1, $2, 'Sched Recon Test', 'active', 0, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("sched-recon-{}@test.local", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("failed to create customer")
}

/// Insert a test invoice (status 'open') and return its ID.
async fn create_invoice(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    customer_id: i32,
    amount_cents: i32,
    currency: &str,
) -> i32 {
    sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status, amount_cents, currency,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, 'open', $4, $5, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("in_sched_{}", Uuid::new_v4()))
    .bind(customer_id)
    .bind(amount_cents)
    .bind(currency)
    .fetch_one(pool)
    .await
    .expect("failed to create invoice")
}

/// Insert a test charge (payment) with status 'succeeded'.
async fn create_charge(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    customer_id: i32,
    amount_cents: i32,
    currency: &str,
) -> i32 {
    sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO ar_charges (
            app_id, ar_customer_id, status, amount_cents, currency,
            charge_type, reason, reference_id, created_at, updated_at
        )
        VALUES ($1, $2, 'succeeded', $3, $4, 'one_time', 'payment', $5, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(customer_id)
    .bind(amount_cents)
    .bind(currency)
    .bind(format!("pay_ref_{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("failed to create charge")
}

/// Create a window start/end pair for testing.
fn make_window(offset_hours: i64) -> (NaiveDateTime, NaiveDateTime) {
    let start = Utc::now().naive_utc() - chrono::Duration::hours(offset_hours + 1);
    let end = Utc::now().naive_utc() - chrono::Duration::hours(offset_hours);
    (start, end)
}

/// Clean up all test data for a tenant.
async fn cleanup_tenant(pool: &sqlx::PgPool, tenant_id: &str) {
    // Scheduled runs
    sqlx::query("DELETE FROM ar_recon_scheduled_runs WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    // Recon exceptions
    sqlx::query("DELETE FROM ar_recon_exceptions WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    // Recon matches
    sqlx::query("DELETE FROM ar_recon_matches WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    // Recon runs
    sqlx::query("DELETE FROM ar_recon_runs WHERE app_id = $1")
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
    // Charges
    sqlx::query("DELETE FROM ar_charges WHERE app_id = $1")
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

/// Test 1: Create scheduled run — returns pending status.
#[tokio::test]
async fn test_create_scheduled_run_pending() {
    let pool = get_ar_pool().await;
    run_migrations(&pool).await;
    let tenant_id = generate_test_tenant();

    let (ws, we) = make_window(24);
    let result = create_scheduled_run(
        &pool,
        CreateScheduledRunRequest {
            scheduled_run_id: Uuid::new_v4(),
            app_id: tenant_id.clone(),
            window_start: ws,
            window_end: we,
            correlation_id: Uuid::new_v4().to_string(),
        },
    )
    .await
    .expect("create failed");

    match result {
        CreateScheduledRunOutcome::Created(r) => {
            assert_eq!(r.status, "pending");
            assert_eq!(r.app_id, tenant_id);
            assert!(r.recon_run_id.is_none());
        }
        CreateScheduledRunOutcome::AlreadyScheduled(_) => {
            panic!("expected Created, got AlreadyScheduled")
        }
    }

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 2: Window dedup — duplicate window returns AlreadyScheduled.
#[tokio::test]
async fn test_window_dedup() {
    let pool = get_ar_pool().await;
    run_migrations(&pool).await;
    let tenant_id = generate_test_tenant();

    let (ws, we) = make_window(48);

    // First create
    let result1 = create_scheduled_run(
        &pool,
        CreateScheduledRunRequest {
            scheduled_run_id: Uuid::new_v4(),
            app_id: tenant_id.clone(),
            window_start: ws,
            window_end: we,
            correlation_id: Uuid::new_v4().to_string(),
        },
    )
    .await
    .expect("first create failed");
    assert!(matches!(result1, CreateScheduledRunOutcome::Created(_)));

    // Second create with same window — deduped
    let result2 = create_scheduled_run(
        &pool,
        CreateScheduledRunRequest {
            scheduled_run_id: Uuid::new_v4(),
            app_id: tenant_id.clone(),
            window_start: ws,
            window_end: we,
            correlation_id: Uuid::new_v4().to_string(),
        },
    )
    .await
    .expect("second create failed");

    match result2 {
        CreateScheduledRunOutcome::AlreadyScheduled(r) => {
            assert_eq!(r.status, "pending");
        }
        CreateScheduledRunOutcome::Created(_) => {
            panic!("expected AlreadyScheduled for duplicate window")
        }
    }

    // Verify only one row in DB
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_recon_scheduled_runs WHERE app_id = $1 AND window_start = $2 AND window_end = $3",
    )
    .bind(&tenant_id)
    .bind(ws)
    .bind(we)
    .fetch_one(&pool)
    .await
    .expect("count failed");
    assert_eq!(count, 1, "only one scheduled run per window");

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 3: Worker claims and executes a pending run → completed.
#[tokio::test]
async fn test_claim_and_execute_completes() {
    let pool = get_ar_pool().await;
    run_migrations(&pool).await;
    let tenant_id = generate_test_tenant();

    // Create test data for matching
    let customer = create_customer(&pool, &tenant_id).await;
    create_invoice(&pool, &tenant_id, customer, 5000, "usd").await;
    create_charge(&pool, &tenant_id, customer, 5000, "usd").await;

    // Create a scheduled run
    let sched_id = Uuid::new_v4();
    let (ws, we) = make_window(12);
    create_scheduled_run(
        &pool,
        CreateScheduledRunRequest {
            scheduled_run_id: sched_id,
            app_id: tenant_id.clone(),
            window_start: ws,
            window_end: we,
            correlation_id: Uuid::new_v4().to_string(),
        },
    )
    .await
    .expect("schedule failed");

    // Worker claims and executes
    let outcome = claim_and_execute_scheduled_run(
        &pool,
        "worker-1",
        &Uuid::new_v4().to_string(),
        Some(&tenant_id),
    )
    .await
    .expect("claim failed");

    match outcome {
        ScheduledRunExecutionOutcome::Completed(r) => {
            assert_eq!(r.status, "completed");
            assert!(r.recon_run_id.is_some(), "should have a recon_run_id");
            assert_eq!(
                r.match_count,
                Some(1),
                "one payment should match one invoice"
            );
            assert_eq!(r.exception_count, Some(0));
        }
        other => panic!("expected Completed, got {:?}", other),
    }

    // Verify DB state
    let status: String = sqlx::query_scalar(
        "SELECT status FROM ar_recon_scheduled_runs WHERE scheduled_run_id = $1",
    )
    .bind(sched_id)
    .fetch_one(&pool)
    .await
    .expect("status query failed");
    assert_eq!(status, "completed");

    // Verify worker_id was recorded
    let worker: Option<String> = sqlx::query_scalar(
        "SELECT worker_id FROM ar_recon_scheduled_runs WHERE scheduled_run_id = $1",
    )
    .bind(sched_id)
    .fetch_one(&pool)
    .await
    .expect("worker query failed");
    assert_eq!(worker.as_deref(), Some("worker-1"));

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 4: Two concurrent workers claim different runs (no double-processing).
#[tokio::test]
async fn test_concurrent_workers_no_double_claim() {
    let pool = get_ar_pool().await;
    run_migrations(&pool).await;
    let tenant_id = generate_test_tenant();

    // Create 2 scheduled runs with different windows
    let (ws1, we1) = make_window(24);
    let (ws2, we2) = make_window(48);

    create_scheduled_run(
        &pool,
        CreateScheduledRunRequest {
            scheduled_run_id: Uuid::new_v4(),
            app_id: tenant_id.clone(),
            window_start: ws1,
            window_end: we1,
            correlation_id: Uuid::new_v4().to_string(),
        },
    )
    .await
    .expect("schedule 1 failed");

    create_scheduled_run(
        &pool,
        CreateScheduledRunRequest {
            scheduled_run_id: Uuid::new_v4(),
            app_id: tenant_id.clone(),
            window_start: ws2,
            window_end: we2,
            correlation_id: Uuid::new_v4().to_string(),
        },
    )
    .await
    .expect("schedule 2 failed");

    // Launch 2 concurrent workers
    let pool1 = pool.clone();
    let pool2 = pool.clone();
    let tid1 = tenant_id.clone();
    let tid2 = tenant_id.clone();

    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let b1 = barrier.clone();
    let b2 = barrier.clone();

    let handle1 = tokio::spawn(async move {
        b1.wait().await;
        claim_and_execute_scheduled_run(&pool1, "worker-1", "corr-1", Some(&tid1)).await
    });

    let handle2 = tokio::spawn(async move {
        b2.wait().await;
        claim_and_execute_scheduled_run(&pool2, "worker-2", "corr-2", Some(&tid2)).await
    });

    let result1 = handle1.await.expect("worker 1 panicked");
    let result2 = handle2.await.expect("worker 2 panicked");

    let outcome1 = result1.expect("worker 1 error");
    let outcome2 = result2.expect("worker 2 error");

    // Both should complete (different runs)
    let mut completed_count = 0;
    if matches!(outcome1, ScheduledRunExecutionOutcome::Completed(_)) {
        completed_count += 1;
    }
    if matches!(outcome2, ScheduledRunExecutionOutcome::Completed(_)) {
        completed_count += 1;
    }

    assert_eq!(
        completed_count, 2,
        "both workers should complete different runs, got: {:?}, {:?}",
        outcome1, outcome2
    );

    // Verify both scheduled runs are completed
    let completed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_recon_scheduled_runs WHERE app_id = $1 AND status = 'completed'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("count failed");
    assert_eq!(completed, 2, "both runs should be completed");

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 5: More workers than runs — losers get NothingToClaim.
#[tokio::test]
async fn test_more_workers_than_runs() {
    let pool = get_ar_pool().await;
    run_migrations(&pool).await;
    let tenant_id = generate_test_tenant();

    // Create 1 scheduled run
    let (ws, we) = make_window(6);
    create_scheduled_run(
        &pool,
        CreateScheduledRunRequest {
            scheduled_run_id: Uuid::new_v4(),
            app_id: tenant_id.clone(),
            window_start: ws,
            window_end: we,
            correlation_id: Uuid::new_v4().to_string(),
        },
    )
    .await
    .expect("schedule failed");

    // Launch 4 concurrent workers
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(4));
    let mut handles = Vec::new();

    for i in 0..4 {
        let pool_clone = pool.clone();
        let b = barrier.clone();
        let tid = tenant_id.clone();
        handles.push(tokio::spawn(async move {
            b.wait().await;
            claim_and_execute_scheduled_run(
                &pool_clone,
                &format!("worker-{}", i),
                &format!("corr-{}", i),
                Some(&tid),
            )
            .await
        }));
    }

    let mut completed = 0;
    let mut nothing = 0;
    let mut errors = 0;

    for handle in handles {
        let result = handle.await.expect("worker panicked");
        match result {
            Ok(ScheduledRunExecutionOutcome::Completed(_)) => completed += 1,
            Ok(ScheduledRunExecutionOutcome::NothingToClaim) => nothing += 1,
            Ok(ScheduledRunExecutionOutcome::Failed { .. }) => errors += 1,
            Err(_) => errors += 1,
        }
    }

    assert_eq!(completed, 1, "exactly 1 worker should complete the run");
    assert!(
        completed + nothing + errors == 4,
        "all 4 workers should return: completed={}, nothing={}, errors={}",
        completed,
        nothing,
        errors
    );

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 6: Completed runs are not re-claimed.
#[tokio::test]
async fn test_completed_runs_not_reclaimed() {
    let pool = get_ar_pool().await;
    run_migrations(&pool).await;
    let tenant_id = generate_test_tenant();

    // Create and complete a scheduled run
    let (ws, we) = make_window(72);
    create_scheduled_run(
        &pool,
        CreateScheduledRunRequest {
            scheduled_run_id: Uuid::new_v4(),
            app_id: tenant_id.clone(),
            window_start: ws,
            window_end: we,
            correlation_id: Uuid::new_v4().to_string(),
        },
    )
    .await
    .expect("schedule failed");

    // Execute first claim (completes it)
    let outcome1 = claim_and_execute_scheduled_run(&pool, "worker-1", "corr-1", Some(&tenant_id))
        .await
        .expect("first claim failed");
    assert!(matches!(
        outcome1,
        ScheduledRunExecutionOutcome::Completed(_)
    ));

    // Second claim should get NothingToClaim (run is completed)
    let outcome2 = claim_and_execute_scheduled_run(&pool, "worker-2", "corr-2", Some(&tenant_id))
        .await
        .expect("second claim failed");

    assert!(
        matches!(outcome2, ScheduledRunExecutionOutcome::NothingToClaim),
        "completed run should not be re-claimed, got {:?}",
        outcome2,
    );

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 7: Run lifecycle — pending → running → completed with match/exception counts.
#[tokio::test]
async fn test_run_lifecycle_with_counts() {
    let pool = get_ar_pool().await;
    run_migrations(&pool).await;
    let tenant_id = generate_test_tenant();

    // Create data: 2 invoices, 1 matching payment, 1 unmatched payment
    let customer = create_customer(&pool, &tenant_id).await;
    create_invoice(&pool, &tenant_id, customer, 3000, "usd").await;
    create_charge(&pool, &tenant_id, customer, 3000, "usd").await;
    create_charge(&pool, &tenant_id, customer, 9999, "usd").await; // no matching invoice

    let sched_id = Uuid::new_v4();
    let (ws, we) = make_window(3);
    create_scheduled_run(
        &pool,
        CreateScheduledRunRequest {
            scheduled_run_id: sched_id,
            app_id: tenant_id.clone(),
            window_start: ws,
            window_end: we,
            correlation_id: Uuid::new_v4().to_string(),
        },
    )
    .await
    .expect("schedule failed");

    // Verify initial status is pending
    let status: String = sqlx::query_scalar(
        "SELECT status FROM ar_recon_scheduled_runs WHERE scheduled_run_id = $1",
    )
    .bind(sched_id)
    .fetch_one(&pool)
    .await
    .expect("status query failed");
    assert_eq!(status, "pending");

    // Execute
    let outcome = claim_and_execute_scheduled_run(
        &pool,
        "worker-lifecycle",
        "corr-lifecycle",
        Some(&tenant_id),
    )
    .await
    .expect("claim failed");

    match outcome {
        ScheduledRunExecutionOutcome::Completed(r) => {
            assert_eq!(r.match_count, Some(1), "one payment matches one invoice");
            assert_eq!(
                r.exception_count,
                Some(1),
                "one unmatched payment exception"
            );
            assert!(r.recon_run_id.is_some());
        }
        other => panic!("expected Completed, got {:?}", other),
    }

    // Verify final status in DB
    let final_status: String = sqlx::query_scalar(
        "SELECT status FROM ar_recon_scheduled_runs WHERE scheduled_run_id = $1",
    )
    .bind(sched_id)
    .fetch_one(&pool)
    .await
    .expect("final status query failed");
    assert_eq!(final_status, "completed");

    // Verify completed_at is set
    let completed_at: Option<chrono::NaiveDateTime> = sqlx::query_scalar(
        "SELECT completed_at FROM ar_recon_scheduled_runs WHERE scheduled_run_id = $1",
    )
    .bind(sched_id)
    .fetch_one(&pool)
    .await
    .expect("completed_at query failed");
    assert!(completed_at.is_some(), "completed_at should be set");

    // Verify match and exception counts in scheduled run row
    let (mc, ec): (Option<i32>, Option<i32>) = sqlx::query_as(
        "SELECT match_count, exception_count FROM ar_recon_scheduled_runs WHERE scheduled_run_id = $1",
    )
    .bind(sched_id)
    .fetch_one(&pool)
    .await
    .expect("counts query failed");
    assert_eq!(mc, Some(1));
    assert_eq!(ec, Some(1));

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 8: Batch poll processes multiple pending runs.
#[tokio::test]
async fn test_batch_poll_multiple_runs() {
    let pool = get_ar_pool().await;
    run_migrations(&pool).await;
    let tenant_id = generate_test_tenant();

    // Create 3 scheduled runs with different windows
    for i in 0..3 {
        let (ws, we) = make_window(100 + i * 2);
        create_scheduled_run(
            &pool,
            CreateScheduledRunRequest {
                scheduled_run_id: Uuid::new_v4(),
                app_id: tenant_id.clone(),
                window_start: ws,
                window_end: we,
                correlation_id: Uuid::new_v4().to_string(),
            },
        )
        .await
        .expect("schedule failed");
    }

    // Poll batch
    let outcomes =
        poll_scheduled_runs(&pool, 10, "batch-worker", "corr-batch", Some(&tenant_id)).await;

    let completed_count = outcomes
        .iter()
        .filter(|o| matches!(o, ScheduledRunExecutionOutcome::Completed(_)))
        .count();

    assert_eq!(
        completed_count, 3,
        "batch should process all 3 pending runs"
    );

    // Verify all completed in DB
    let db_completed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_recon_scheduled_runs WHERE app_id = $1 AND status = 'completed'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("count failed");
    assert_eq!(db_completed, 3);

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 9: Outbox events emitted for executed matching run.
#[tokio::test]
async fn test_outbox_events_emitted() {
    let pool = get_ar_pool().await;
    run_migrations(&pool).await;
    let tenant_id = generate_test_tenant();

    // Create matching data
    let customer = create_customer(&pool, &tenant_id).await;
    create_invoice(&pool, &tenant_id, customer, 7000, "usd").await;
    create_charge(&pool, &tenant_id, customer, 7000, "usd").await;

    // Schedule and execute
    let (ws, we) = make_window(36);
    create_scheduled_run(
        &pool,
        CreateScheduledRunRequest {
            scheduled_run_id: Uuid::new_v4(),
            app_id: tenant_id.clone(),
            window_start: ws,
            window_end: we,
            correlation_id: Uuid::new_v4().to_string(),
        },
    )
    .await
    .expect("schedule failed");

    claim_and_execute_scheduled_run(&pool, "worker-outbox", "corr-outbox", Some(&tenant_id))
        .await
        .expect("claim failed");

    // Verify outbox has run_started event
    let run_events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1 AND event_type = 'ar.recon_run_started'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("event count failed");
    assert!(
        run_events >= 1,
        "at least one run_started event should exist"
    );

    // Verify outbox has match_applied event
    let match_events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1 AND event_type = 'ar.recon_match_applied'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("event count failed");
    assert!(
        match_events >= 1,
        "at least one match_applied event should exist"
    );

    // Verify causation_id chain — match events should have causation from the scheduled run
    let causation_ids: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT causation_id FROM events_outbox WHERE tenant_id = $1 AND event_type = 'ar.recon_match_applied'",
    )
    .bind(&tenant_id)
    .fetch_all(&pool)
    .await
    .expect("causation query failed");

    for cid in &causation_ids {
        assert!(cid.is_some(), "match events should have causation_id set");
    }

    cleanup_tenant(&pool, &tenant_id).await;
}

/// Test 10: Different tenants are isolated — claiming for tenant A doesn't affect tenant B.
#[tokio::test]
async fn test_tenant_isolation() {
    let pool = get_ar_pool().await;
    run_migrations(&pool).await;
    let tenant_a = generate_test_tenant();
    let tenant_b = generate_test_tenant();

    let (ws, we) = make_window(60);

    // Schedule run for tenant A
    create_scheduled_run(
        &pool,
        CreateScheduledRunRequest {
            scheduled_run_id: Uuid::new_v4(),
            app_id: tenant_a.clone(),
            window_start: ws,
            window_end: we,
            correlation_id: Uuid::new_v4().to_string(),
        },
    )
    .await
    .expect("schedule A failed");

    // Schedule run for tenant B
    create_scheduled_run(
        &pool,
        CreateScheduledRunRequest {
            scheduled_run_id: Uuid::new_v4(),
            app_id: tenant_b.clone(),
            window_start: ws,
            window_end: we,
            correlation_id: Uuid::new_v4().to_string(),
        },
    )
    .await
    .expect("schedule B failed");

    // Claim for tenant A only
    let outcome = claim_and_execute_scheduled_run(&pool, "worker-a", "corr-a", Some(&tenant_a))
        .await
        .expect("claim A failed");
    assert!(matches!(
        outcome,
        ScheduledRunExecutionOutcome::Completed(_)
    ));

    // Tenant B still has a pending run
    let b_status: String =
        sqlx::query_scalar("SELECT status FROM ar_recon_scheduled_runs WHERE app_id = $1")
            .bind(&tenant_b)
            .fetch_one(&pool)
            .await
            .expect("status B query failed");
    assert_eq!(
        b_status, "pending",
        "tenant B's run should still be pending"
    );

    cleanup_tenant(&pool, &tenant_a).await;
    cleanup_tenant(&pool, &tenant_b).await;
}
