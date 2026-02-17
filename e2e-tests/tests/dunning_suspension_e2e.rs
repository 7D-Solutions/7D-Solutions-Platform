//! E2E Test: Dunning Suspension Triggers (bd-1i7)
//!
//! **Coverage:**
//! 1. Dunning escalation to Suspended emits ar.invoice_suspended outbox event
//! 2. Subscription consumer processes ar.invoice_suspended and suspends subscription
//! 3. Idempotent reprocessing: duplicate event_id is a no-op
//! 4. Scheduler auto-escalation to Suspended also emits ar.invoice_suspended
//! 5. Multiple subscriptions for same customer all get suspended
//! 6. No subscription found — consumer handles gracefully (no error)
//! 7. Already-suspended subscription — consumer is idempotent
//!
//! **Pattern:** No Docker, no mocks — uses live AR + Subscriptions database pools

mod common;

use anyhow::Result;
use ar_rs::dunning::{
    init_dunning, transition_dunning, DunningStateValue, InitDunningRequest,
    TransitionDunningRequest,
};
use ar_rs::dunning_scheduler::claim_and_execute_one;
use chrono::Utc;
use common::{generate_test_tenant, get_ar_pool, get_subscriptions_pool};
use subscriptions_rs::consumer::{handle_invoice_suspended, InvoiceSuspendedEvent};
use uuid::Uuid;

// ============================================================================
// Test helpers
// ============================================================================

/// Insert a minimal AR customer and invoice for test isolation.
async fn create_test_invoice(pool: &sqlx::PgPool, tenant_id: &str) -> Result<(i32, i32, String)> {
    let customer_id_str = format!("cust-{}", Uuid::new_v4());

    let customer_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
        VALUES ($1, $2, $3, 'active', 0, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("dunn-susp-{}@test.local", Uuid::new_v4()))
    .bind("Dunning Suspension Test Customer")
    .fetch_one(pool)
    .await?;

    let invoice_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status, amount_cents, currency,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, 'open', 50000, 'usd', NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("in_susp_{}", Uuid::new_v4()))
    .bind(customer_id)
    .fetch_one(pool)
    .await?;

    Ok((customer_id, invoice_id, customer_id_str))
}

/// Create a subscription plan and an active subscription in the subscriptions DB.
async fn create_test_subscription(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    ar_customer_id: &str,
) -> Result<Uuid> {
    let plan_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO subscription_plans (tenant_id, name, schedule, price_minor, currency, created_at, updated_at)
        VALUES ($1, 'Test Plan', 'monthly', 5000, 'usd', NOW(), NOW())
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

/// Create a past_due subscription in the subscriptions DB.
async fn create_past_due_subscription(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    ar_customer_id: &str,
) -> Result<Uuid> {
    let plan_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO subscription_plans (tenant_id, name, schedule, price_minor, currency, created_at, updated_at)
        VALUES ($1, 'Past Due Plan', 'monthly', 7000, 'usd', NOW(), NOW())
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
        VALUES ($1, $2, $3, 'past_due', 'monthly', 7000, 'usd', CURRENT_DATE, CURRENT_DATE + 30, NOW(), NOW())
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

/// Clean up all test data for a tenant in AR DB (reverse FK order).
async fn cleanup_ar_tenant(pool: &sqlx::PgPool, tenant_id: &str) -> Result<()> {
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

/// Clean up all test data for a tenant in Subscriptions DB.
async fn cleanup_subs_tenant(pool: &sqlx::PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM processed_events WHERE event_id LIKE $1")
        .bind(format!("{}%", tenant_id))
        .execute(pool)
        .await?;
    // Delete processed_events by looking up events we might have inserted
    sqlx::query("DELETE FROM events_outbox WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM subscription_invoice_attempts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM subscriptions WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM subscription_plans WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Get subscription status from DB.
async fn get_subscription_status(pool: &sqlx::PgPool, sub_id: Uuid) -> Result<String> {
    let status: String = sqlx::query_scalar(
        "SELECT status FROM subscriptions WHERE id = $1",
    )
    .bind(sub_id)
    .fetch_one(pool)
    .await?;
    Ok(status)
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: Dunning transition to Suspended emits ar.invoice_suspended outbox event
#[tokio::test]
async fn test_dunning_suspension_emits_invoice_suspended_event() {
    let ar_pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let dunning_id = Uuid::new_v4();

    let (_customer_id, invoice_id, customer_id_str) =
        create_test_invoice(&ar_pool, &tenant_id).await.expect("create invoice");

    // Init dunning → Pending
    init_dunning(&ar_pool, InitDunningRequest {
        dunning_id,
        app_id: tenant_id.clone(),
        invoice_id,
        customer_id: customer_id_str.clone(),
        next_attempt_at: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    }).await.expect("init dunning");

    // Pending → Warned
    transition_dunning(&ar_pool, TransitionDunningRequest {
        app_id: tenant_id.clone(),
        invoice_id,
        to_state: DunningStateValue::Warned,
        reason: "first_attempt_failed".to_string(),
        next_attempt_at: None,
        last_error: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    }).await.expect("Pending → Warned");

    // Warned → Escalated
    transition_dunning(&ar_pool, TransitionDunningRequest {
        app_id: tenant_id.clone(),
        invoice_id,
        to_state: DunningStateValue::Escalated,
        reason: "second_attempt_failed".to_string(),
        next_attempt_at: None,
        last_error: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    }).await.expect("Warned → Escalated");

    // Escalated → Suspended (should emit ar.invoice_suspended)
    transition_dunning(&ar_pool, TransitionDunningRequest {
        app_id: tenant_id.clone(),
        invoice_id,
        to_state: DunningStateValue::Suspended,
        reason: "max_attempts_exceeded".to_string(),
        next_attempt_at: None,
        last_error: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    }).await.expect("Escalated → Suspended");

    // Verify ar.invoice_suspended event exists in outbox
    let suspended_event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE event_type = 'ar.invoice_suspended' AND tenant_id = $1",
    )
    .bind(&tenant_id)
    .fetch_one(&ar_pool)
    .await
    .expect("count query");

    assert_eq!(suspended_event_count, 1, "exactly one ar.invoice_suspended event must be emitted");

    // Verify the event payload contains correct customer_id
    let event_payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM events_outbox WHERE event_type = 'ar.invoice_suspended' AND tenant_id = $1",
    )
    .bind(&tenant_id)
    .fetch_one(&ar_pool)
    .await
    .expect("payload query");

    let payload_data = &event_payload["payload"];
    assert_eq!(payload_data["customer_id"].as_str().unwrap(), &customer_id_str);
    assert_eq!(payload_data["invoice_id"].as_str().unwrap(), &invoice_id.to_string());
    assert_eq!(payload_data["tenant_id"].as_str().unwrap(), &tenant_id);

    // Verify the mutation_class is LIFECYCLE
    let mutation_class: String = sqlx::query_scalar(
        "SELECT mutation_class FROM events_outbox WHERE event_type = 'ar.invoice_suspended' AND tenant_id = $1",
    )
    .bind(&tenant_id)
    .fetch_one(&ar_pool)
    .await
    .expect("mutation_class query");
    assert_eq!(mutation_class, "LIFECYCLE");

    cleanup_ar_tenant(&ar_pool, &tenant_id).await.unwrap();
}

/// Test 2: Subscription consumer processes ar.invoice_suspended and suspends subscription
#[tokio::test]
async fn test_subscription_suspended_on_dunning_event() {
    let ar_pool = get_ar_pool().await;
    let subs_pool = get_subscriptions_pool().await;
    let tenant_id = generate_test_tenant();

    let (_customer_id, invoice_id, customer_id_str) =
        create_test_invoice(&ar_pool, &tenant_id).await.expect("create invoice");

    // Create an active subscription for this customer
    let sub_id = create_test_subscription(&subs_pool, &tenant_id, &customer_id_str)
        .await.expect("create subscription");

    // Verify subscription is active
    let status = get_subscription_status(&subs_pool, sub_id).await.expect("get status");
    assert_eq!(status, "active", "subscription must start as active");

    // Simulate receiving ar.invoice_suspended event
    let event_id = Uuid::new_v4().to_string();
    let event = InvoiceSuspendedEvent {
        tenant_id: tenant_id.clone(),
        invoice_id: invoice_id.to_string(),
        customer_id: customer_id_str.clone(),
        dunning_attempt: 3,
        reason: "max_attempts_exceeded".to_string(),
    };

    let processed = handle_invoice_suspended(&subs_pool, &event_id, &event)
        .await
        .expect("handle_invoice_suspended failed");

    assert!(processed, "event should be processed (not a duplicate)");

    // Verify subscription is now suspended
    let status = get_subscription_status(&subs_pool, sub_id).await.expect("get status");
    assert_eq!(status, "suspended", "subscription must be suspended after dunning event");

    // Verify subscriptions outbox has a status.changed event
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE event_type = 'subscriptions.status.changed' AND tenant_id = $1",
    )
    .bind(&tenant_id)
    .fetch_one(&subs_pool)
    .await
    .expect("outbox count");
    assert_eq!(outbox_count, 1, "subscriptions.status.changed event must be emitted");

    cleanup_ar_tenant(&ar_pool, &tenant_id).await.unwrap();
    cleanup_subs_tenant(&subs_pool, &tenant_id).await.unwrap();
}

/// Test 3: Idempotent reprocessing — duplicate event_id is a no-op
#[tokio::test]
async fn test_suspension_consumer_idempotent() {
    let subs_pool = get_subscriptions_pool().await;
    let tenant_id = generate_test_tenant();

    let customer_id_str = format!("cust-{}", Uuid::new_v4());
    let sub_id = create_test_subscription(&subs_pool, &tenant_id, &customer_id_str)
        .await.expect("create subscription");

    let event_id = Uuid::new_v4().to_string();
    let event = InvoiceSuspendedEvent {
        tenant_id: tenant_id.clone(),
        invoice_id: "inv-99".to_string(),
        customer_id: customer_id_str.clone(),
        dunning_attempt: 3,
        reason: "max_attempts_exceeded".to_string(),
    };

    // First processing — should suspend
    let first = handle_invoice_suspended(&subs_pool, &event_id, &event)
        .await.expect("first handle");
    assert!(first, "first processing should return true");

    let status = get_subscription_status(&subs_pool, sub_id).await.expect("get status");
    assert_eq!(status, "suspended");

    // Second processing with same event_id — should be skipped
    let second = handle_invoice_suspended(&subs_pool, &event_id, &event)
        .await.expect("second handle");
    assert!(!second, "second processing should return false (idempotent skip)");

    cleanup_subs_tenant(&subs_pool, &tenant_id).await.unwrap();
}

/// Test 4: Scheduler auto-escalation to Suspended also emits ar.invoice_suspended
#[tokio::test]
async fn test_scheduler_escalation_emits_invoice_suspended() {
    let ar_pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let dunning_id = Uuid::new_v4();

    let (_customer_id, invoice_id, customer_id_str) =
        create_test_invoice(&ar_pool, &tenant_id).await.expect("create invoice");

    // Init dunning with a past next_attempt_at so scheduler can claim it
    let past = Utc::now() - chrono::Duration::hours(1);

    init_dunning(&ar_pool, InitDunningRequest {
        dunning_id,
        app_id: tenant_id.clone(),
        invoice_id,
        customer_id: customer_id_str.clone(),
        next_attempt_at: Some(past),
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    }).await.expect("init dunning");

    // Manually advance to Escalated state (one step before Suspended)
    transition_dunning(&ar_pool, TransitionDunningRequest {
        app_id: tenant_id.clone(),
        invoice_id,
        to_state: DunningStateValue::Warned,
        reason: "test".to_string(),
        next_attempt_at: Some(past),
        last_error: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    }).await.expect("→ Warned");

    transition_dunning(&ar_pool, TransitionDunningRequest {
        app_id: tenant_id.clone(),
        invoice_id,
        to_state: DunningStateValue::Escalated,
        reason: "test".to_string(),
        next_attempt_at: Some(past),
        last_error: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    }).await.expect("→ Escalated");

    // Now let the scheduler auto-escalate to Suspended
    let corr_id = Uuid::new_v4().to_string();
    let outcome = claim_and_execute_one(&ar_pool, &corr_id, Some(&tenant_id))
        .await
        .expect("scheduler claim_and_execute_one");

    match outcome {
        ar_rs::dunning_scheduler::DunningExecutionOutcome::Transitioned { to_state, .. } => {
            assert_eq!(to_state, "suspended", "scheduler must escalate to suspended");
        }
        other => panic!("expected Transitioned outcome, got {:?}", other),
    }

    // Verify ar.invoice_suspended event in outbox
    let suspended_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE event_type = 'ar.invoice_suspended' AND tenant_id = $1",
    )
    .bind(&tenant_id)
    .fetch_one(&ar_pool)
    .await
    .expect("count query");

    assert_eq!(suspended_count, 1, "scheduler must emit ar.invoice_suspended on suspension");

    cleanup_ar_tenant(&ar_pool, &tenant_id).await.unwrap();
}

/// Test 5: Multiple subscriptions for same customer all get suspended
#[tokio::test]
async fn test_multiple_subscriptions_all_suspended() {
    let subs_pool = get_subscriptions_pool().await;
    let tenant_id = generate_test_tenant();

    let customer_id_str = format!("cust-{}", Uuid::new_v4());

    // Create two active subscriptions for the same customer
    let sub_id_1 = create_test_subscription(&subs_pool, &tenant_id, &customer_id_str)
        .await.expect("create sub 1");
    let sub_id_2 = create_test_subscription(&subs_pool, &tenant_id, &customer_id_str)
        .await.expect("create sub 2");

    let event_id = Uuid::new_v4().to_string();
    let event = InvoiceSuspendedEvent {
        tenant_id: tenant_id.clone(),
        invoice_id: "inv-multi".to_string(),
        customer_id: customer_id_str.clone(),
        dunning_attempt: 3,
        reason: "max_attempts_exceeded".to_string(),
    };

    handle_invoice_suspended(&subs_pool, &event_id, &event)
        .await.expect("handle");

    let status_1 = get_subscription_status(&subs_pool, sub_id_1).await.expect("s1");
    let status_2 = get_subscription_status(&subs_pool, sub_id_2).await.expect("s2");

    assert_eq!(status_1, "suspended", "first subscription must be suspended");
    assert_eq!(status_2, "suspended", "second subscription must be suspended");

    cleanup_subs_tenant(&subs_pool, &tenant_id).await.unwrap();
}

/// Test 6: No subscription found — consumer handles gracefully
#[tokio::test]
async fn test_no_subscription_found_no_error() {
    let subs_pool = get_subscriptions_pool().await;
    let tenant_id = generate_test_tenant();

    let event_id = Uuid::new_v4().to_string();
    let event = InvoiceSuspendedEvent {
        tenant_id: tenant_id.clone(),
        invoice_id: "inv-none".to_string(),
        customer_id: format!("nonexistent-{}", Uuid::new_v4()),
        dunning_attempt: 3,
        reason: "max_attempts_exceeded".to_string(),
    };

    // Should not error — just logs a warning
    let processed = handle_invoice_suspended(&subs_pool, &event_id, &event)
        .await.expect("should not error");

    assert!(processed, "event was processed (even though no subs found)");

    // Clean up processed_events
    sqlx::query("DELETE FROM processed_events WHERE event_id = $1")
        .bind(&event_id)
        .execute(&subs_pool)
        .await
        .unwrap();
}

/// Test 7: Already-suspended subscription — consumer is idempotent (no error)
#[tokio::test]
async fn test_already_suspended_subscription_no_error() {
    let subs_pool = get_subscriptions_pool().await;
    let tenant_id = generate_test_tenant();

    let customer_id_str = format!("cust-{}", Uuid::new_v4());

    // Create a past_due subscription, then suspend it via the consumer
    let sub_id = create_past_due_subscription(&subs_pool, &tenant_id, &customer_id_str)
        .await.expect("create past_due sub");

    // First event suspends the subscription
    let event_id_1 = Uuid::new_v4().to_string();
    let event = InvoiceSuspendedEvent {
        tenant_id: tenant_id.clone(),
        invoice_id: "inv-already".to_string(),
        customer_id: customer_id_str.clone(),
        dunning_attempt: 3,
        reason: "max_attempts_exceeded".to_string(),
    };

    handle_invoice_suspended(&subs_pool, &event_id_1, &event)
        .await.expect("first suspend");

    let status = get_subscription_status(&subs_pool, sub_id).await.expect("status");
    assert_eq!(status, "suspended");

    // Second event with DIFFERENT event_id but subscription already suspended
    let event_id_2 = Uuid::new_v4().to_string();
    let processed = handle_invoice_suspended(&subs_pool, &event_id_2, &event)
        .await.expect("second suspend should not error");

    // This was processed (new event_id) but the subscription was already suspended
    // so the IllegalTransition was handled gracefully
    assert!(processed, "event was processed (idempotent transition handled)");

    // Subscription should still be suspended
    let status = get_subscription_status(&subs_pool, sub_id).await.expect("status");
    assert_eq!(status, "suspended");

    cleanup_subs_tenant(&subs_pool, &tenant_id).await.unwrap();
}
