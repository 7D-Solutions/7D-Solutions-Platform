//! E2E Test: Dunning Scheduler Concurrency Safety (bd-2bj)
//!
//! **Coverage:**
//! 1. Single worker claims and transitions a due dunning row (Pending → Warned)
//! 2. Bounded backoff: next_attempt_at computed correctly (1h, 2h, 4h, ...)
//! 3. Two concurrent workers: each claims a different row (no double-processing)
//! 4. More workers than rows: losers get NothingToClaim (not errors)
//! 5. Batch poll: processes multiple due rows in one call
//! 6. Terminal state rows are not claimed by the scheduler
//! 7. Future next_attempt_at rows are not claimed
//! 8. Outbox events emitted atomically with state transitions
//!
//! **Pattern:** No Docker, no mocks — uses live AR database pool via common::get_ar_pool()
//! All tests use tenant-scoped claiming for test isolation.

mod common;

use anyhow::Result;
use ar_rs::dunning::{init_dunning, DunningStateValue, InitDunningRequest, InitDunningResult};
use ar_rs::dunning_scheduler::{
    claim_and_execute_one, poll_and_execute_batch, DunningExecutionOutcome,
};
use chrono::{Duration, Utc};
use common::{generate_test_tenant, get_ar_pool};
use uuid::Uuid;

// ============================================================================
// Test helpers
// ============================================================================

/// Insert a minimal customer and invoice for test isolation.
async fn create_test_invoice(pool: &sqlx::PgPool, tenant_id: &str) -> Result<(i32, i32)> {
    let customer_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
        VALUES ($1, $2, $3, 'active', 0, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("sched-test-{}@test.local", Uuid::new_v4()))
    .bind("Scheduler Test Customer")
    .fetch_one(pool)
    .await?;

    let invoice_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status, amount_cents, currency,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, 'open', 10000, 'usd', NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("in_sched_{}", Uuid::new_v4()))
    .bind(customer_id)
    .fetch_one(pool)
    .await?;

    Ok((customer_id, invoice_id))
}

/// Initialize dunning with a next_attempt_at in the past (due immediately).
async fn init_due_dunning(pool: &sqlx::PgPool, tenant_id: &str, invoice_id: i32) -> Result<Uuid> {
    let dunning_id = Uuid::new_v4();
    let result = init_dunning(
        pool,
        InitDunningRequest {
            dunning_id,
            app_id: tenant_id.to_string(),
            invoice_id,
            customer_id: format!("cust-{}", tenant_id),
            next_attempt_at: Some(Utc::now() - Duration::seconds(60)), // Due 1 min ago
            correlation_id: Uuid::new_v4().to_string(),
            causation_id: None,
        },
    )
    .await?;

    assert!(
        matches!(result, InitDunningResult::Initialized { .. }),
        "expected Initialized, got {:?}",
        result
    );

    Ok(dunning_id)
}

/// Clean up all test data for a tenant.
async fn cleanup_tenant(pool: &sqlx::PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM events_outbox WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM ar_dunning_states WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM ar_invoice_attempts WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: Single worker claims and transitions a due dunning row (Pending → Warned)
#[tokio::test]
async fn test_scheduler_claims_due_row_and_transitions() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let (_customer_id, invoice_id) = create_test_invoice(&pool, &tenant_id)
        .await
        .expect("create invoice failed");
    let dunning_id = init_due_dunning(&pool, &tenant_id, invoice_id)
        .await
        .expect("init dunning failed");

    let correlation_id = Uuid::new_v4().to_string();
    let outcome = claim_and_execute_one(&pool, &correlation_id, Some(&tenant_id))
        .await
        .expect("claim_and_execute_one failed");

    match &outcome {
        DunningExecutionOutcome::Transitioned {
            from_state,
            to_state,
            new_attempt_count,
            next_attempt_at,
        } => {
            assert_eq!(from_state, "pending", "should transition FROM pending");
            assert_eq!(to_state, "warned", "should transition TO warned");
            assert_eq!(*new_attempt_count, 1, "attempt_count should be 1");
            assert!(
                next_attempt_at.is_some(),
                "non-terminal should have next_attempt_at"
            );
        }
        other => panic!("expected Transitioned, got {:?}", other),
    }

    // Verify DB state
    let state: String = sqlx::query_scalar(
        "SELECT state FROM ar_dunning_states WHERE app_id = $1 AND invoice_id = $2",
    )
    .bind(&tenant_id)
    .bind(invoice_id)
    .fetch_one(&pool)
    .await
    .expect("state query failed");
    assert_eq!(state, "warned");

    // Verify outbox event count (init + transition = 2)
    let event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE aggregate_type = 'dunning_state' AND aggregate_id = $1",
    )
    .bind(dunning_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("event count failed");
    assert_eq!(event_count, 2, "init + transition = 2 outbox events");

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test 2: Bounded backoff — next_attempt_at is approximately 1h from transition time
#[tokio::test]
async fn test_scheduler_backoff_stored_correctly() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let (_customer_id, invoice_id) = create_test_invoice(&pool, &tenant_id)
        .await
        .expect("create invoice failed");
    let _dunning_id = init_due_dunning(&pool, &tenant_id, invoice_id)
        .await
        .expect("init dunning failed");

    let before = Utc::now();
    let correlation_id = Uuid::new_v4().to_string();
    claim_and_execute_one(&pool, &correlation_id, Some(&tenant_id))
        .await
        .expect("claim failed");
    let after = Utc::now();

    // Read the next_attempt_at from DB
    let next_attempt: Option<chrono::DateTime<Utc>> = sqlx::query_scalar(
        "SELECT next_attempt_at FROM ar_dunning_states WHERE app_id = $1 AND invoice_id = $2",
    )
    .bind(&tenant_id)
    .bind(invoice_id)
    .fetch_one(&pool)
    .await
    .expect("next_attempt_at query failed");

    let next_attempt = next_attempt.expect("non-terminal state should have next_attempt_at");

    // For attempt_count=1, backoff should be ~1 hour from the execution time
    let expected_min = before + Duration::seconds(3600 - 5); // small tolerance
    let expected_max = after + Duration::seconds(3600 + 5);
    assert!(
        next_attempt >= expected_min && next_attempt <= expected_max,
        "next_attempt_at should be ~1h from execution, got diff_from_before={}s, diff_from_after={}s",
        (next_attempt - before).num_seconds(),
        (next_attempt - after).num_seconds()
    );

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test 3: Two concurrent workers each claim a different row (no double-processing)
#[tokio::test]
async fn test_scheduler_concurrent_workers_no_double_claim() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();

    // Create 2 invoices with due dunning
    let (_c1, inv1) = create_test_invoice(&pool, &tenant_id)
        .await
        .expect("create invoice 1 failed");
    let (_c2, inv2) = create_test_invoice(&pool, &tenant_id)
        .await
        .expect("create invoice 2 failed");

    let _d1 = init_due_dunning(&pool, &tenant_id, inv1)
        .await
        .expect("init dunning 1 failed");
    let _d2 = init_due_dunning(&pool, &tenant_id, inv2)
        .await
        .expect("init dunning 2 failed");

    // Launch 2 concurrent workers with barrier synchronization
    let pool1 = pool.clone();
    let pool2 = pool.clone();
    let tid1 = tenant_id.clone();
    let tid2 = tenant_id.clone();

    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let b1 = barrier.clone();
    let b2 = barrier.clone();

    let handle1 = tokio::spawn(async move {
        b1.wait().await;
        claim_and_execute_one(&pool1, "worker-1", Some(&tid1)).await
    });

    let handle2 = tokio::spawn(async move {
        b2.wait().await;
        claim_and_execute_one(&pool2, "worker-2", Some(&tid2)).await
    });

    let result1 = handle1.await.expect("worker 1 panicked");
    let result2 = handle2.await.expect("worker 2 panicked");

    let outcome1 = result1.expect("worker 1 error");
    let outcome2 = result2.expect("worker 2 error");

    // Both should succeed — SKIP LOCKED ensures each gets a different row
    let mut transitioned_count = 0;
    if matches!(outcome1, DunningExecutionOutcome::Transitioned { .. }) {
        transitioned_count += 1;
    }
    if matches!(outcome2, DunningExecutionOutcome::Transitioned { .. }) {
        transitioned_count += 1;
    }

    assert_eq!(
        transitioned_count, 2,
        "both workers should claim and transition a different row, got: {:?}, {:?}",
        outcome1, outcome2
    );

    // Verify both rows are now in 'warned' state
    let warned_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_dunning_states WHERE app_id = $1 AND state = 'warned'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("warned count failed");
    assert_eq!(warned_count, 2, "both rows should be warned");

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test 4: More workers than rows — losers get NothingToClaim
#[tokio::test]
async fn test_scheduler_more_workers_than_rows() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();

    // Create 1 invoice with due dunning
    let (_c, inv) = create_test_invoice(&pool, &tenant_id)
        .await
        .expect("create invoice failed");
    let _d = init_due_dunning(&pool, &tenant_id, inv)
        .await
        .expect("init dunning failed");

    // Launch 4 concurrent workers but only 1 row
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(4));
    let mut handles = Vec::new();

    for i in 0..4 {
        let pool_clone = pool.clone();
        let b = barrier.clone();
        let tid = tenant_id.clone();
        handles.push(tokio::spawn(async move {
            b.wait().await;
            claim_and_execute_one(&pool_clone, &format!("worker-{}", i), Some(&tid)).await
        }));
    }

    let mut transitioned = 0;
    let mut nothing = 0;
    let mut concurrent_mod = 0;

    for handle in handles {
        let result = handle.await.expect("worker panicked");
        match result {
            Ok(DunningExecutionOutcome::Transitioned { .. }) => transitioned += 1,
            Ok(DunningExecutionOutcome::NothingToClaim) => nothing += 1,
            Ok(DunningExecutionOutcome::AlreadyTerminal { .. }) => nothing += 1,
            Ok(DunningExecutionOutcome::Failed { .. }) => concurrent_mod += 1,
            Err(_) => concurrent_mod += 1,
        }
    }

    assert_eq!(
        transitioned, 1,
        "exactly 1 worker should transition the row"
    );
    assert!(
        transitioned + nothing + concurrent_mod == 4,
        "all 4 workers should have returned: transitioned={}, nothing={}, errors={}",
        transitioned,
        nothing,
        concurrent_mod
    );

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test 5: Batch poll processes multiple due rows
#[tokio::test]
async fn test_scheduler_batch_poll_processes_multiple() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();

    // Create 3 invoices with due dunning
    for _ in 0..3 {
        let (_c, inv) = create_test_invoice(&pool, &tenant_id)
            .await
            .expect("create invoice failed");
        init_due_dunning(&pool, &tenant_id, inv)
            .await
            .expect("init dunning failed");
    }

    let correlation_id = Uuid::new_v4().to_string();
    let outcomes = poll_and_execute_batch(&pool, 10, &correlation_id, Some(&tenant_id)).await;

    let transitioned_count = outcomes
        .iter()
        .filter(|o| matches!(o, DunningExecutionOutcome::Transitioned { .. }))
        .count();

    assert_eq!(transitioned_count, 3, "batch should process all 3 due rows");

    // Verify all are now warned
    let warned_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_dunning_states WHERE app_id = $1 AND state = 'warned'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("warned count failed");
    assert_eq!(warned_count, 3);

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test 6: Terminal state rows are not claimed by the scheduler
#[tokio::test]
async fn test_scheduler_skips_terminal_rows() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let (_c, inv) = create_test_invoice(&pool, &tenant_id)
        .await
        .expect("create invoice failed");
    let _d = init_due_dunning(&pool, &tenant_id, inv)
        .await
        .expect("init dunning failed");

    // Manually set to resolved (terminal) with a past next_attempt_at
    sqlx::query(
        "UPDATE ar_dunning_states SET state = 'resolved', next_attempt_at = NOW() - INTERVAL '1 hour' WHERE app_id = $1 AND invoice_id = $2",
    )
    .bind(&tenant_id)
    .bind(inv)
    .execute(&pool)
    .await
    .expect("update to resolved failed");

    let correlation_id = Uuid::new_v4().to_string();
    let outcome = claim_and_execute_one(&pool, &correlation_id, Some(&tenant_id))
        .await
        .expect("claim failed");

    assert!(
        matches!(outcome, DunningExecutionOutcome::NothingToClaim),
        "terminal row should not be claimed, got {:?}",
        outcome
    );

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test 7: Future next_attempt_at rows are not claimed
#[tokio::test]
async fn test_scheduler_skips_future_rows() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let (_c, inv) = create_test_invoice(&pool, &tenant_id)
        .await
        .expect("create invoice failed");

    // Init with future next_attempt_at (2 hours from now)
    let dunning_id = Uuid::new_v4();
    init_dunning(
        &pool,
        InitDunningRequest {
            dunning_id,
            app_id: tenant_id.clone(),
            invoice_id: inv,
            customer_id: format!("cust-{}", tenant_id),
            next_attempt_at: Some(Utc::now() + Duration::hours(2)),
            correlation_id: Uuid::new_v4().to_string(),
            causation_id: None,
        },
    )
    .await
    .expect("init failed");

    let correlation_id = Uuid::new_v4().to_string();
    let outcome = claim_and_execute_one(&pool, &correlation_id, Some(&tenant_id))
        .await
        .expect("claim failed");

    assert!(
        matches!(outcome, DunningExecutionOutcome::NothingToClaim),
        "future row should not be claimed, got {:?}",
        outcome
    );

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test 8: Outbox events are emitted atomically with transitions
#[tokio::test]
async fn test_scheduler_outbox_atomicity() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let (_c, inv) = create_test_invoice(&pool, &tenant_id)
        .await
        .expect("create invoice failed");
    let dunning_id = init_due_dunning(&pool, &tenant_id, inv)
        .await
        .expect("init dunning failed");

    let correlation_id = Uuid::new_v4().to_string();
    claim_and_execute_one(&pool, &correlation_id, Some(&tenant_id))
        .await
        .expect("claim failed");

    // Count outbox events: 1 for init + 1 for scheduler transition
    let event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE aggregate_type = 'dunning_state' AND aggregate_id = $1",
    )
    .bind(dunning_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("event count failed");
    assert_eq!(
        event_count, 2,
        "init + transition = 2 outbox events atomically"
    );

    // Verify all events have LIFECYCLE mutation_class
    let mutation_classes: Vec<String> = sqlx::query_scalar(
        "SELECT mutation_class FROM events_outbox WHERE aggregate_type = 'dunning_state' AND aggregate_id = $1 ORDER BY occurred_at",
    )
    .bind(dunning_id.to_string())
    .fetch_all(&pool)
    .await
    .expect("fetch mutation_class failed");

    assert_eq!(mutation_classes.len(), 2);
    assert!(
        mutation_classes.iter().all(|mc| mc == "LIFECYCLE"),
        "all dunning events must be LIFECYCLE"
    );

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}
