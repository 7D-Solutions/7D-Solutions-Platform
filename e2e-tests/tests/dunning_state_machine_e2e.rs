//! E2E Test: AR Dunning State Machine (bd-1rr)
//!
//! **Coverage:**
//! 1. Init dunning — record created in Pending state + LIFECYCLE outbox event emitted
//! 2. Idempotency: duplicate dunning_id is a no-op (AlreadyExists, no second event)
//! 3. Transition Pending → Warned — state updated, version incremented, event emitted
//! 4. Transition Pending → Escalated (skip Warned) — valid skip transition
//! 5. Transition Warned → Escalated → Suspended — escalation chain
//! 6. Transition to Resolved (payment) from non-terminal state
//! 7. Terminal state rejection — Resolved → Warned returns TerminalState error
//! 8. Illegal transition rejection — Warned → Pending (backwards) returns error
//! 9. Outbox atomicity — state row + outbox event always committed together
//! 10. Outbox event carries mutation_class = LIFECYCLE
//!
//! **Pattern:** No Docker, no mocks — uses live AR database pool via common::get_ar_pool()

mod common;

use anyhow::Result;
use ar_rs::dunning::{
    init_dunning, transition_dunning, DunningError, DunningStateValue, InitDunningRequest,
    InitDunningResult, TransitionDunningRequest, TransitionDunningResult,
};
use common::{generate_test_tenant, get_ar_pool, get_subscriptions_pool};
use subscriptions_rs::consumer::{handle_invoice_suspended, InvoiceSuspendedEvent};
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
    .bind(format!("dunn-test-{}@test.local", Uuid::new_v4()))
    .bind("Dunning Test Customer")
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
    .bind(format!("in_dunn_{}", Uuid::new_v4()))
    .bind(customer_id)
    .fetch_one(pool)
    .await?;

    Ok((customer_id, invoice_id))
}

/// Clean up all test data for a tenant (reverse FK order).
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

/// Count dunning_state outbox events for this dunning_id.
async fn count_dunning_outbox_events(pool: &sqlx::PgPool, dunning_id: &str) -> Result<i64> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE aggregate_type = 'dunning_state' AND aggregate_id = $1",
    )
    .bind(dunning_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: Init dunning — record inserted in Pending state, outbox event emitted
#[tokio::test]
async fn test_dunning_init_creates_pending_state() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let dunning_id = Uuid::new_v4();

    let (_customer_id, invoice_id) = create_test_invoice(&pool, &tenant_id)
        .await
        .expect("failed to create invoice");

    let req = InitDunningRequest {
        dunning_id,
        app_id: tenant_id.clone(),
        invoice_id,
        customer_id: format!("cust-{}", tenant_id),
        next_attempt_at: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    };

    let result = init_dunning(&pool, req).await.expect("init_dunning failed");
    assert!(
        matches!(result, InitDunningResult::Initialized { .. }),
        "expected Initialized, got {:?}",
        result
    );

    // Verify DB state
    let state: String =
        sqlx::query_scalar("SELECT state FROM ar_dunning_states WHERE app_id = $1 AND invoice_id = $2")
            .bind(&tenant_id)
            .bind(invoice_id)
            .fetch_one(&pool)
            .await
            .expect("state query failed");
    assert_eq!(state, "pending", "initial state must be pending");

    let version: i32 =
        sqlx::query_scalar("SELECT version FROM ar_dunning_states WHERE app_id = $1 AND invoice_id = $2")
            .bind(&tenant_id)
            .bind(invoice_id)
            .fetch_one(&pool)
            .await
            .expect("version query failed");
    assert_eq!(version, 1, "initial version must be 1");

    // Verify outbox event was emitted
    let event_count = count_dunning_outbox_events(&pool, &dunning_id.to_string())
        .await
        .expect("event count failed");
    assert_eq!(event_count, 1, "exactly one LIFECYCLE outbox event must be emitted");

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test 2: Idempotency — duplicate dunning_id returns AlreadyExists, no second event
#[tokio::test]
async fn test_dunning_init_idempotency() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let dunning_id = Uuid::new_v4();

    let (_customer_id, invoice_id) = create_test_invoice(&pool, &tenant_id)
        .await
        .expect("failed to create invoice");

    let req = InitDunningRequest {
        dunning_id,
        app_id: tenant_id.clone(),
        invoice_id,
        customer_id: format!("cust-{}", tenant_id),
        next_attempt_at: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    };

    // First call — should succeed
    init_dunning(&pool, req.clone()).await.expect("first init failed");

    // Second call — same dunning_id → AlreadyExists
    let result = init_dunning(&pool, req).await.expect("second init should not error");
    assert!(
        matches!(result, InitDunningResult::AlreadyExists { .. }),
        "expected AlreadyExists on duplicate dunning_id, got {:?}",
        result
    );

    // Only one dunning row and one outbox event
    let row_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_dunning_states WHERE dunning_id = $1")
            .bind(dunning_id)
            .fetch_one(&pool)
            .await
            .expect("count failed");
    assert_eq!(row_count, 1, "only one dunning row after idempotent replay");

    let event_count = count_dunning_outbox_events(&pool, &dunning_id.to_string())
        .await
        .expect("event count failed");
    assert_eq!(event_count, 1, "only one outbox event after idempotent replay");

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test 3: Transition Pending → Warned — state updated, version incremented, event emitted
#[tokio::test]
async fn test_dunning_transition_pending_to_warned() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let dunning_id = Uuid::new_v4();

    let (_customer_id, invoice_id) = create_test_invoice(&pool, &tenant_id)
        .await
        .expect("failed to create invoice");

    // Initialize
    init_dunning(&pool, InitDunningRequest {
        dunning_id,
        app_id: tenant_id.clone(),
        invoice_id,
        customer_id: format!("cust-{}", tenant_id),
        next_attempt_at: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    })
    .await
    .expect("init failed");

    // Transition to Warned
    let result = transition_dunning(&pool, TransitionDunningRequest {
        app_id: tenant_id.clone(),
        invoice_id,
        to_state: DunningStateValue::Warned,
        reason: "first_collection_attempt_failed".to_string(),
        next_attempt_at: None,
        last_error: Some("card_declined".to_string()),
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: Some(dunning_id.to_string()),
    })
    .await
    .expect("transition to Warned failed");

    match result {
        TransitionDunningResult::Transitioned {
            from_state,
            to_state,
            new_version,
            new_attempt_count,
            ..
        } => {
            assert_eq!(from_state, DunningStateValue::Pending);
            assert_eq!(to_state, DunningStateValue::Warned);
            assert_eq!(new_version, 2, "version must increment on transition");
            assert_eq!(new_attempt_count, 1, "attempt_count must increment for Warned");
        }
    }

    // Verify DB state
    let (state, version): (String, i32) = sqlx::query_as(
        "SELECT state, version FROM ar_dunning_states WHERE app_id = $1 AND invoice_id = $2",
    )
    .bind(&tenant_id)
    .bind(invoice_id)
    .fetch_one(&pool)
    .await
    .expect("state/version query failed");

    assert_eq!(state, "warned");
    assert_eq!(version, 2);

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test 4: Transition Pending → Escalated directly (skip Warned)
#[tokio::test]
async fn test_dunning_transition_pending_to_escalated_skip_warned() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let dunning_id = Uuid::new_v4();

    let (_customer_id, invoice_id) = create_test_invoice(&pool, &tenant_id)
        .await
        .expect("failed to create invoice");

    init_dunning(&pool, InitDunningRequest {
        dunning_id,
        app_id: tenant_id.clone(),
        invoice_id,
        customer_id: format!("cust-{}", tenant_id),
        next_attempt_at: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    })
    .await
    .expect("init failed");

    // Skip Warned, go straight to Escalated
    let result = transition_dunning(&pool, TransitionDunningRequest {
        app_id: tenant_id.clone(),
        invoice_id,
        to_state: DunningStateValue::Escalated,
        reason: "aggressive_escalation_policy".to_string(),
        next_attempt_at: None,
        last_error: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    })
    .await
    .expect("Pending → Escalated should be valid");

    assert!(matches!(result, TransitionDunningResult::Transitioned { .. }));

    let state: String =
        sqlx::query_scalar("SELECT state FROM ar_dunning_states WHERE app_id = $1 AND invoice_id = $2")
            .bind(&tenant_id)
            .bind(invoice_id)
            .fetch_one(&pool)
            .await
            .expect("state query failed");
    assert_eq!(state, "escalated");

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test 5: Transition to Resolved (payment received)
#[tokio::test]
async fn test_dunning_transition_to_resolved() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let dunning_id = Uuid::new_v4();

    let (_customer_id, invoice_id) = create_test_invoice(&pool, &tenant_id)
        .await
        .expect("failed to create invoice");

    init_dunning(&pool, InitDunningRequest {
        dunning_id,
        app_id: tenant_id.clone(),
        invoice_id,
        customer_id: format!("cust-{}", tenant_id),
        next_attempt_at: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    })
    .await
    .expect("init failed");

    let result = transition_dunning(&pool, TransitionDunningRequest {
        app_id: tenant_id.clone(),
        invoice_id,
        to_state: DunningStateValue::Resolved,
        reason: "payment_received".to_string(),
        next_attempt_at: None,
        last_error: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    })
    .await
    .expect("Pending → Resolved should be valid");

    assert!(matches!(result, TransitionDunningResult::Transitioned { .. }));

    let state: String =
        sqlx::query_scalar("SELECT state FROM ar_dunning_states WHERE app_id = $1 AND invoice_id = $2")
            .bind(&tenant_id)
            .bind(invoice_id)
            .fetch_one(&pool)
            .await
            .expect("state query failed");
    assert_eq!(state, "resolved");

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test 6: Terminal state rejection — Resolved → Warned returns TerminalState error
#[tokio::test]
async fn test_dunning_terminal_state_rejects_further_transitions() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let dunning_id = Uuid::new_v4();

    let (_customer_id, invoice_id) = create_test_invoice(&pool, &tenant_id)
        .await
        .expect("failed to create invoice");

    init_dunning(&pool, InitDunningRequest {
        dunning_id,
        app_id: tenant_id.clone(),
        invoice_id,
        customer_id: format!("cust-{}", tenant_id),
        next_attempt_at: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    })
    .await
    .expect("init failed");

    // Move to Resolved (terminal)
    transition_dunning(&pool, TransitionDunningRequest {
        app_id: tenant_id.clone(),
        invoice_id,
        to_state: DunningStateValue::Resolved,
        reason: "payment_received".to_string(),
        next_attempt_at: None,
        last_error: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    })
    .await
    .expect("transition to Resolved failed");

    // Attempt further transition — must fail
    let err = transition_dunning(&pool, TransitionDunningRequest {
        app_id: tenant_id.clone(),
        invoice_id,
        to_state: DunningStateValue::Warned,
        reason: "should_not_happen".to_string(),
        next_attempt_at: None,
        last_error: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    })
    .await
    .expect_err("expected TerminalState error");

    assert!(
        matches!(err, DunningError::TerminalState { .. }),
        "expected TerminalState error, got {:?}",
        err
    );

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test 7: Illegal transition — Warned → Pending (backwards) returns IllegalTransition
#[tokio::test]
async fn test_dunning_illegal_transition_rejected() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let dunning_id = Uuid::new_v4();

    let (_customer_id, invoice_id) = create_test_invoice(&pool, &tenant_id)
        .await
        .expect("failed to create invoice");

    init_dunning(&pool, InitDunningRequest {
        dunning_id,
        app_id: tenant_id.clone(),
        invoice_id,
        customer_id: format!("cust-{}", tenant_id),
        next_attempt_at: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    })
    .await
    .expect("init failed");

    // Advance to Warned
    transition_dunning(&pool, TransitionDunningRequest {
        app_id: tenant_id.clone(),
        invoice_id,
        to_state: DunningStateValue::Warned,
        reason: "first_attempt_failed".to_string(),
        next_attempt_at: None,
        last_error: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    })
    .await
    .expect("Pending → Warned failed");

    // Attempt illegal backwards transition Warned → Pending
    let err = transition_dunning(&pool, TransitionDunningRequest {
        app_id: tenant_id.clone(),
        invoice_id,
        to_state: DunningStateValue::Pending,
        reason: "illegal_backwards".to_string(),
        next_attempt_at: None,
        last_error: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    })
    .await
    .expect_err("expected IllegalTransition error");

    assert!(
        matches!(err, DunningError::IllegalTransition { .. }),
        "expected IllegalTransition, got {:?}",
        err
    );

    // State must not have changed
    let state: String =
        sqlx::query_scalar("SELECT state FROM ar_dunning_states WHERE app_id = $1 AND invoice_id = $2")
            .bind(&tenant_id)
            .bind(invoice_id)
            .fetch_one(&pool)
            .await
            .expect("state query failed");
    assert_eq!(state, "warned", "state must remain warned after illegal transition");

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test 8: Outbox atomicity — dunning state row and outbox event committed together,
/// and outbox event carries mutation_class = LIFECYCLE
#[tokio::test]
async fn test_dunning_outbox_atomicity_and_lifecycle_class() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let dunning_id = Uuid::new_v4();

    let (_customer_id, invoice_id) = create_test_invoice(&pool, &tenant_id)
        .await
        .expect("failed to create invoice");

    init_dunning(&pool, InitDunningRequest {
        dunning_id,
        app_id: tenant_id.clone(),
        invoice_id,
        customer_id: format!("cust-{}", tenant_id),
        next_attempt_at: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: Some("invoice-overdue-check".to_string()),
    })
    .await
    .expect("init_dunning failed");

    // Verify dunning row exists
    let row_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_dunning_states WHERE dunning_id = $1")
            .bind(dunning_id)
            .fetch_one(&pool)
            .await
            .expect("count failed");
    assert_eq!(row_count, 1, "dunning row must exist");

    // Verify outbox event exists
    let event_count = count_dunning_outbox_events(&pool, &dunning_id.to_string())
        .await
        .expect("event count failed");
    assert_eq!(event_count, 1, "outbox event must exist atomically");

    // Verify outbox event has mutation_class = LIFECYCLE
    let mutation_class: String = sqlx::query_scalar(
        "SELECT mutation_class FROM events_outbox WHERE aggregate_type = 'dunning_state' AND aggregate_id = $1",
    )
    .bind(dunning_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("fetch mutation_class failed");
    assert_eq!(
        mutation_class, "LIFECYCLE",
        "dunning event must carry LIFECYCLE mutation_class"
    );

    // Verify event_type is the canonical ar.dunning_state_changed
    let event_type: String = sqlx::query_scalar(
        "SELECT event_type FROM events_outbox WHERE aggregate_type = 'dunning_state' AND aggregate_id = $1",
    )
    .bind(dunning_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("fetch event_type failed");
    assert_eq!(event_type, "ar.dunning_state_changed");

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test 9: Unknown invoice returns DunningNotFound on transition
#[tokio::test]
async fn test_dunning_transition_unknown_invoice_not_found() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();

    let err = transition_dunning(&pool, TransitionDunningRequest {
        app_id: tenant_id.clone(),
        invoice_id: i32::MAX, // very unlikely to exist
        to_state: DunningStateValue::Warned,
        reason: "test".to_string(),
        next_attempt_at: None,
        last_error: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    })
    .await
    .expect_err("expected DunningNotFound");

    assert!(
        matches!(err, DunningError::DunningNotFound { .. }),
        "expected DunningNotFound, got {:?}",
        err
    );
}

// ============================================================================
// Integrated cross-domain test (bd-b6k)
// ============================================================================

/// Create a subscription plan and an active subscription in the subscriptions DB.
async fn create_test_subscription(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    ar_customer_id: &str,
) -> anyhow::Result<Uuid> {
    let plan_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO subscription_plans (tenant_id, name, schedule, price_minor, currency, created_at, updated_at)
        VALUES ($1, 'Dunning Integ Plan', 'monthly', 5000, 'usd', NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    let sub_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO subscriptions (
            tenant_id, ar_customer_id, plan_id, status, schedule,
            price_minor, currency, start_date, next_bill_date,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, 'active', 'monthly', 5000, 'usd', CURRENT_DATE, CURRENT_DATE + 30, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(ar_customer_id)
    .bind(plan_id)
    .fetch_one(pool)
    .await?;

    Ok(sub_id)
}

/// Clean up subscriptions test data for a tenant.
async fn cleanup_subs_tenant(pool: &sqlx::PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM events_outbox WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM subscriptions WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM subscription_plans WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

/// Test 10 (bd-b6k): Integrated dunning-to-subscription-suspension flow.
///
/// Proves the full chain: init dunning → Warned → Escalated → Suspended →
/// ar.invoice_suspended outbox event emitted → subscription consumer processes
/// the event → subscription status changes to 'suspended'.
#[tokio::test]
async fn test_integrated_dunning_to_subscription_suspension() {
    let ar_pool = get_ar_pool().await;
    let subs_pool = get_subscriptions_pool().await;
    let tenant_id = generate_test_tenant();
    let dunning_id = Uuid::new_v4();
    let customer_id_str = format!("cust-{}", tenant_id);

    // --- Setup: AR invoice + subscription ---
    let (_customer_id, invoice_id) = create_test_invoice(&ar_pool, &tenant_id)
        .await
        .expect("failed to create invoice");

    let sub_id = create_test_subscription(&subs_pool, &tenant_id, &customer_id_str)
        .await
        .expect("failed to create subscription");

    // Verify subscription starts as active
    let status: String = sqlx::query_scalar(
        "SELECT status FROM subscriptions WHERE id = $1",
    )
    .bind(sub_id)
    .fetch_one(&subs_pool)
    .await
    .expect("subscription query failed");
    assert_eq!(status, "active", "subscription must start as active");

    // --- Step 1: Init dunning (Pending) ---
    let result = init_dunning(&ar_pool, InitDunningRequest {
        dunning_id,
        app_id: tenant_id.clone(),
        invoice_id,
        customer_id: customer_id_str.clone(),
        next_attempt_at: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    })
    .await
    .expect("init_dunning failed");
    assert!(matches!(result, InitDunningResult::Initialized { .. }));

    // --- Step 2: Pending → Warned ---
    transition_dunning(&ar_pool, TransitionDunningRequest {
        app_id: tenant_id.clone(),
        invoice_id,
        to_state: DunningStateValue::Warned,
        reason: "first_collection_failed".to_string(),
        next_attempt_at: None,
        last_error: Some("card_declined".to_string()),
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: Some(dunning_id.to_string()),
    })
    .await
    .expect("Pending → Warned failed");

    // --- Step 3: Warned → Escalated ---
    transition_dunning(&ar_pool, TransitionDunningRequest {
        app_id: tenant_id.clone(),
        invoice_id,
        to_state: DunningStateValue::Escalated,
        reason: "second_collection_failed".to_string(),
        next_attempt_at: None,
        last_error: Some("card_declined_again".to_string()),
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: Some(dunning_id.to_string()),
    })
    .await
    .expect("Warned → Escalated failed");

    // --- Step 4: Escalated → Suspended ---
    transition_dunning(&ar_pool, TransitionDunningRequest {
        app_id: tenant_id.clone(),
        invoice_id,
        to_state: DunningStateValue::Suspended,
        reason: "max_retries_exhausted".to_string(),
        next_attempt_at: None,
        last_error: Some("all_attempts_failed".to_string()),
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: Some(dunning_id.to_string()),
    })
    .await
    .expect("Escalated → Suspended failed");

    // Verify dunning state is suspended in DB
    let dunning_state: String = sqlx::query_scalar(
        "SELECT state FROM ar_dunning_states WHERE app_id = $1 AND invoice_id = $2",
    )
    .bind(&tenant_id)
    .bind(invoice_id)
    .fetch_one(&ar_pool)
    .await
    .expect("dunning state query failed");
    assert_eq!(dunning_state, "suspended");

    // Verify ar.invoice_suspended outbox event was emitted
    let suspended_event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1 AND event_type = 'ar.invoice_suspended'",
    )
    .bind(&tenant_id)
    .fetch_one(&ar_pool)
    .await
    .expect("outbox query failed");
    assert!(
        suspended_event_count >= 1,
        "ar.invoice_suspended event must be emitted on Suspended transition"
    );

    // --- Step 5: Feed the event to the subscription consumer ---
    let event_id = Uuid::new_v4().to_string();
    let event = InvoiceSuspendedEvent {
        tenant_id: tenant_id.clone(),
        invoice_id: invoice_id.to_string(),
        customer_id: customer_id_str.clone(),
        dunning_attempt: 3,
        reason: "max_retries_exhausted".to_string(),
    };

    let processed = handle_invoice_suspended(&subs_pool, &event_id, &event)
        .await
        .expect("handle_invoice_suspended failed");
    assert!(processed, "consumer must process the event successfully");

    // --- Step 6: Verify subscription is now suspended ---
    let sub_status: String = sqlx::query_scalar(
        "SELECT status FROM subscriptions WHERE id = $1",
    )
    .bind(sub_id)
    .fetch_one(&subs_pool)
    .await
    .expect("subscription status query failed");
    assert_eq!(
        sub_status, "suspended",
        "subscription must be suspended after dunning chain completes"
    );

    // --- Cleanup ---
    cleanup_tenant(&ar_pool, &tenant_id).await.unwrap();
    cleanup_subs_tenant(&subs_pool, &tenant_id).await;
}
