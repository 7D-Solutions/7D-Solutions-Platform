//! E2E Test: AR Dunning Workflow (bd-3o56)
//!
//! Proves the full dunning lifecycle end-to-end:
//! 1. Past-due invoice triggers dunning initialization
//! 2. Scheduler auto-escalates: Pending → Warned → Escalated → Suspended
//! 3. Retry count increments and next_attempt_at advances with backoff at each step
//! 4. Suspended is the final automatic state — scheduler does not progress further
//! 5. Grace period boundary: future next_attempt_at prevents scheduler from claiming
//!
//! **Pattern:** No Docker, no mocks — uses live AR database pool via common::get_ar_pool()

mod common;

use anyhow::Result;
use ar_rs::dunning::{init_dunning, InitDunningRequest, InitDunningResult};
use ar_rs::dunning_scheduler::{claim_and_execute_one, DunningExecutionOutcome};
use chrono::{Duration, Utc};
use common::{generate_test_tenant, get_ar_pool};
use uuid::Uuid;

// ============================================================================
// Test helpers
// ============================================================================

/// Insert a minimal AR customer and open invoice for test isolation.
async fn create_test_invoice(pool: &sqlx::PgPool, tenant_id: &str) -> Result<(i32, i32)> {
    let customer_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
        VALUES ($1, $2, $3, 'active', 0, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("dunning-wf-{}@test.local", Uuid::new_v4()))
    .bind("Dunning Workflow Customer")
    .fetch_one(pool)
    .await?;

    let invoice_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status, amount_cents, currency,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, 'open', 25000, 'usd', NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("in_wf_{}", Uuid::new_v4()))
    .bind(customer_id)
    .fetch_one(pool)
    .await?;

    Ok((customer_id, invoice_id))
}

/// Clean up all test data for a tenant (reverse FK order).
async fn cleanup_tenant(pool: &sqlx::PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM events_outbox WHERE tenant_id = $1")
        .bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM ar_dunning_states WHERE app_id = $1")
        .bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM ar_invoice_attempts WHERE app_id = $1")
        .bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(tenant_id).execute(pool).await.ok();
}

/// Read dunning state from DB: (state, version, attempt_count, next_attempt_at).
async fn get_dunning_state(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    invoice_id: i32,
) -> Result<(String, i32, i32, Option<chrono::DateTime<Utc>>)> {
    let row: (String, i32, i32, Option<chrono::DateTime<Utc>>) = sqlx::query_as(
        r#"SELECT state, version, attempt_count, next_attempt_at
           FROM ar_dunning_states
           WHERE app_id = $1 AND invoice_id = $2"#,
    )
    .bind(tenant_id)
    .bind(invoice_id)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Force the next_attempt_at to the past so the scheduler can pick it up.
async fn make_dunning_due(pool: &sqlx::PgPool, tenant_id: &str, invoice_id: i32) -> Result<()> {
    sqlx::query(
        "UPDATE ar_dunning_states SET next_attempt_at = NOW() - INTERVAL '1 minute' \
         WHERE app_id = $1 AND invoice_id = $2",
    )
    .bind(tenant_id)
    .bind(invoice_id)
    .execute(pool)
    .await?;
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

/// Full dunning workflow: past-due invoice → scheduler auto-escalates through
/// Pending → Warned → Escalated → Suspended, verifying retry count and backoff
/// at each step. Then confirms scheduler does NOT progress past Suspended.
#[tokio::test]
async fn test_dunning_workflow_full_lifecycle() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let dunning_id = Uuid::new_v4();

    let (_customer_id, invoice_id) = create_test_invoice(&pool, &tenant_id)
        .await
        .expect("create test invoice");

    // ── Step 1: Initialize dunning with past next_attempt_at (past due) ──
    let past = Utc::now() - Duration::minutes(5);
    let result = init_dunning(
        &pool,
        InitDunningRequest {
            dunning_id,
            app_id: tenant_id.clone(),
            invoice_id,
            customer_id: format!("cust-{}", tenant_id),
            next_attempt_at: Some(past),
            correlation_id: Uuid::new_v4().to_string(),
            causation_id: None,
        },
    )
    .await
    .expect("init_dunning");
    assert!(matches!(result, InitDunningResult::Initialized { .. }));
    println!("[1/6] Dunning initialized: Pending (past due)");

    let (state, version, attempt_count, _next) =
        get_dunning_state(&pool, &tenant_id, invoice_id).await.unwrap();
    assert_eq!(state, "pending");
    assert_eq!(version, 1);
    assert_eq!(attempt_count, 0);

    // ── Step 2: Scheduler escalation → Pending to Warned ──
    let corr = Uuid::new_v4().to_string();
    let outcome = claim_and_execute_one(&pool, &corr, Some(&tenant_id))
        .await
        .expect("scheduler step 1");
    match &outcome {
        DunningExecutionOutcome::Transitioned {
            from_state, to_state, new_attempt_count, next_attempt_at,
        } => {
            assert_eq!(from_state, "pending");
            assert_eq!(to_state, "warned");
            assert_eq!(*new_attempt_count, 1);
            assert!(next_attempt_at.is_some(), "warned should have next_attempt_at");
        }
        other => panic!("expected Transitioned, got {:?}", other),
    }
    println!("[2/6] Scheduler: Pending → Warned (attempt=1)");

    // Verify backoff for attempt=1 is ~1h
    let (state, version, attempt_count, next_at) =
        get_dunning_state(&pool, &tenant_id, invoice_id).await.unwrap();
    assert_eq!(state, "warned");
    assert_eq!(version, 2);
    assert_eq!(attempt_count, 1);
    let next_at = next_at.expect("warned should have next_attempt_at");
    let backoff_secs = (next_at - Utc::now()).num_seconds();
    assert!(
        backoff_secs > 3500 && backoff_secs < 3700,
        "attempt 1 backoff should be ~1h (3600s), got {}s",
        backoff_secs
    );

    // ── Step 3: Make due again, scheduler → Warned to Escalated ──
    make_dunning_due(&pool, &tenant_id, invoice_id).await.unwrap();
    let corr = Uuid::new_v4().to_string();
    let outcome = claim_and_execute_one(&pool, &corr, Some(&tenant_id))
        .await
        .expect("scheduler step 2");
    match &outcome {
        DunningExecutionOutcome::Transitioned {
            from_state, to_state, new_attempt_count, next_attempt_at,
        } => {
            assert_eq!(from_state, "warned");
            assert_eq!(to_state, "escalated");
            assert_eq!(*new_attempt_count, 2);
            assert!(next_attempt_at.is_some(), "escalated should have next_attempt_at");
        }
        other => panic!("expected Transitioned, got {:?}", other),
    }
    println!("[3/6] Scheduler: Warned → Escalated (attempt=2)");

    // Verify backoff for attempt=2 is ~2h
    let (state, _version, attempt_count, next_at) =
        get_dunning_state(&pool, &tenant_id, invoice_id).await.unwrap();
    assert_eq!(state, "escalated");
    assert_eq!(attempt_count, 2);
    let next_at = next_at.expect("escalated should have next_attempt_at");
    let backoff_secs = (next_at - Utc::now()).num_seconds();
    assert!(
        backoff_secs > 7100 && backoff_secs < 7300,
        "attempt 2 backoff should be ~2h (7200s), got {}s",
        backoff_secs
    );

    // ── Step 4: Make due again, scheduler → Escalated to Suspended ──
    make_dunning_due(&pool, &tenant_id, invoice_id).await.unwrap();
    let corr = Uuid::new_v4().to_string();
    let outcome = claim_and_execute_one(&pool, &corr, Some(&tenant_id))
        .await
        .expect("scheduler step 3");
    match &outcome {
        DunningExecutionOutcome::Transitioned {
            from_state, to_state, new_attempt_count, ..
        } => {
            assert_eq!(from_state, "escalated");
            assert_eq!(to_state, "suspended");
            assert_eq!(*new_attempt_count, 3);
        }
        other => panic!("expected Transitioned, got {:?}", other),
    }
    println!("[4/6] Scheduler: Escalated → Suspended (attempt=3, final auto-state)");

    let (state, _version, attempt_count, _next_at) =
        get_dunning_state(&pool, &tenant_id, invoice_id).await.unwrap();
    assert_eq!(state, "suspended");
    assert_eq!(attempt_count, 3);

    // ── Step 5: Scheduler does NOT progress past Suspended ──
    make_dunning_due(&pool, &tenant_id, invoice_id).await.unwrap();
    let corr = Uuid::new_v4().to_string();
    let outcome = claim_and_execute_one(&pool, &corr, Some(&tenant_id))
        .await
        .expect("scheduler step 4");
    assert!(
        matches!(outcome, DunningExecutionOutcome::AlreadyTerminal { .. }),
        "scheduler must not progress Suspended further, got {:?}",
        outcome
    );
    println!("[5/6] Scheduler stops at Suspended (no further auto-escalation)");

    // ── Step 6: Verify ar.invoice_suspended event was emitted ──
    let suspended_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE event_type = 'ar.invoice_suspended' AND tenant_id = $1",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("suspended event count");
    assert_eq!(
        suspended_count, 1,
        "ar.invoice_suspended must be emitted when reaching Suspended"
    );

    // Total outbox: 1 (init) + 3 (transitions) + 1 (invoice_suspended) = 5
    let total_events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("total event count");
    assert_eq!(total_events, 5, "init + 3 transitions + 1 invoice_suspended = 5 events");
    println!("[6/6] ar.invoice_suspended outbox event confirmed (5 total events)");

    println!("\n=== Dunning Workflow Full Lifecycle: ALL PASSED ===");
    cleanup_tenant(&pool, &tenant_id).await;
}

/// Grace period boundary: dunning with future next_attempt_at is NOT claimed
/// by the scheduler — proves the scheduler respects the due date.
#[tokio::test]
async fn test_dunning_grace_period_boundary() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let dunning_id = Uuid::new_v4();

    let (_customer_id, invoice_id) = create_test_invoice(&pool, &tenant_id)
        .await
        .expect("create test invoice");

    // Initialize dunning with next_attempt_at 2 hours in the future
    let future = Utc::now() + Duration::hours(2);
    let result = init_dunning(
        &pool,
        InitDunningRequest {
            dunning_id,
            app_id: tenant_id.clone(),
            invoice_id,
            customer_id: format!("cust-{}", tenant_id),
            next_attempt_at: Some(future),
            correlation_id: Uuid::new_v4().to_string(),
            causation_id: None,
        },
    )
    .await
    .expect("init_dunning");
    assert!(matches!(result, InitDunningResult::Initialized { .. }));

    // Scheduler should find nothing to claim — still within grace period
    let corr = Uuid::new_v4().to_string();
    let outcome = claim_and_execute_one(&pool, &corr, Some(&tenant_id))
        .await
        .expect("scheduler claim");
    assert!(
        matches!(outcome, DunningExecutionOutcome::NothingToClaim),
        "scheduler must NOT claim rows with future next_attempt_at, got {:?}",
        outcome
    );

    // Verify state is still Pending (untouched)
    let (state, version, attempt_count, _) =
        get_dunning_state(&pool, &tenant_id, invoice_id).await.unwrap();
    assert_eq!(state, "pending", "state must remain pending");
    assert_eq!(version, 1, "version must be unchanged");
    assert_eq!(attempt_count, 0, "attempt_count must be unchanged");

    println!("=== Grace Period Boundary: PASSED ===");
    cleanup_tenant(&pool, &tenant_id).await;
}
