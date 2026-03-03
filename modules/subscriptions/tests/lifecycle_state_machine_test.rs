//! Integration tests for subscription lifecycle state machine
//!
//! Tests cover:
//! 1. Full lifecycle chains (multi-step transition sequences)
//! 2. Invalid transition rejection at the database level
//! 3. Outbox event emission after lifecycle transitions
//! 4. Consumer event handling (handle_invoice_suspended)
//! 5. Consumer idempotency (duplicate event processing)

use sqlx::PgPool;
use subscriptions_rs::consumer::{handle_invoice_suspended, InvoiceSuspendedEvent};
use subscriptions_rs::lifecycle::{
    transition_to_active, transition_to_past_due, transition_to_suspended, TransitionError,
};
use uuid::Uuid;

// ============================================================================
// Test Helpers
// ============================================================================

async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://subscriptions_user:subscriptions_pass@localhost:5435/subscriptions_db"
            .to_string()
    });

    let pool = sqlx::PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

async fn create_test_subscription(pool: &PgPool, status: &str) -> (Uuid, String, String) {
    let tenant_id = format!("test-tenant-{}", Uuid::new_v4());
    let ar_customer_id = format!("customer-{}", Uuid::new_v4());

    let plan_id: Uuid = sqlx::query_scalar(
        "INSERT INTO subscription_plans (tenant_id, name, schedule, price_minor, currency)
         VALUES ($1, 'Test Plan', 'monthly', 2999, 'USD')
         RETURNING id",
    )
    .bind(&tenant_id)
    .fetch_one(pool)
    .await
    .expect("Failed to create test plan");

    let subscription_id: Uuid = sqlx::query_scalar(
        "INSERT INTO subscriptions (tenant_id, ar_customer_id, plan_id, status, schedule, \
         price_minor, currency, start_date, next_bill_date)
         VALUES ($1, $2, $3, $4, 'monthly', 2999, 'USD', CURRENT_DATE, \
         CURRENT_DATE + INTERVAL '1 month')
         RETURNING id",
    )
    .bind(&tenant_id)
    .bind(&ar_customer_id)
    .bind(plan_id)
    .bind(status)
    .fetch_one(pool)
    .await
    .expect("Failed to create test subscription");

    (subscription_id, tenant_id, ar_customer_id)
}

async fn get_status(pool: &PgPool, id: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM subscriptions WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("Failed to fetch status")
}

async fn cleanup(pool: &PgPool, id: Uuid) {
    sqlx::query("DELETE FROM subscriptions WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// Full Lifecycle Chain: Active → PastDue → Suspended → Active
// ============================================================================

#[tokio::test]
async fn test_full_dunning_lifecycle_and_recovery() {
    let pool = setup_test_pool().await;
    let (sub_id, _, _) = create_test_subscription(&pool, "active").await;

    // Step 1: Payment fails → PastDue
    transition_to_past_due(sub_id, "payment_failed", &pool)
        .await
        .expect("Active → PastDue should succeed");
    assert_eq!(get_status(&pool, sub_id).await, "past_due");

    // Step 2: Grace period expires → Suspended
    transition_to_suspended(sub_id, "grace_period_expired", &pool)
        .await
        .expect("PastDue → Suspended should succeed");
    assert_eq!(get_status(&pool, sub_id).await, "suspended");

    // Step 3: Payment recovered → Active (reactivation)
    transition_to_active(sub_id, "payment_recovered", &pool)
        .await
        .expect("Suspended → Active should succeed");
    assert_eq!(get_status(&pool, sub_id).await, "active");

    cleanup(&pool, sub_id).await;
}

#[tokio::test]
async fn test_direct_active_to_suspended_dunning_escalation() {
    let pool = setup_test_pool().await;
    let (sub_id, _, _) = create_test_subscription(&pool, "active").await;

    // Direct Active → Suspended (dunning terminal escalation)
    transition_to_suspended(sub_id, "dunning_terminal_escalation", &pool)
        .await
        .expect("Active → Suspended (direct) should succeed");
    assert_eq!(get_status(&pool, sub_id).await, "suspended");

    cleanup(&pool, sub_id).await;
}

#[tokio::test]
async fn test_past_due_recovery_without_suspension() {
    let pool = setup_test_pool().await;
    let (sub_id, _, _) = create_test_subscription(&pool, "active").await;

    // Payment fails
    transition_to_past_due(sub_id, "payment_failed", &pool)
        .await
        .unwrap();
    assert_eq!(get_status(&pool, sub_id).await, "past_due");

    // Payment recovered before suspension
    transition_to_active(sub_id, "payment_recovered", &pool)
        .await
        .expect("PastDue → Active should succeed");
    assert_eq!(get_status(&pool, sub_id).await, "active");

    cleanup(&pool, sub_id).await;
}

// ============================================================================
// Invalid Transition Rejection (database-backed)
// ============================================================================

#[tokio::test]
async fn test_suspended_to_past_due_rejected_via_lifecycle_fn() {
    let pool = setup_test_pool().await;
    let (sub_id, _, _) = create_test_subscription(&pool, "suspended").await;

    let result = transition_to_past_due(sub_id, "backwards", &pool).await;

    assert!(result.is_err());
    match result {
        Err(TransitionError::IllegalTransition { from, to, .. }) => {
            assert_eq!(from, "suspended");
            assert_eq!(to, "past_due");
        }
        other => panic!("Expected IllegalTransition, got: {:?}", other),
    }

    // Status unchanged
    assert_eq!(get_status(&pool, sub_id).await, "suspended");
    cleanup(&pool, sub_id).await;
}

#[tokio::test]
async fn test_nonexistent_subscription_transition_fails() {
    let pool = setup_test_pool().await;
    let fake_id = Uuid::new_v4();

    let result = transition_to_past_due(fake_id, "test", &pool).await;
    assert!(result.is_err());
    match result {
        Err(TransitionError::SubscriptionNotFound { subscription_id }) => {
            assert_eq!(subscription_id, fake_id);
        }
        other => panic!("Expected SubscriptionNotFound, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_double_suspension_is_idempotent() {
    let pool = setup_test_pool().await;
    let (sub_id, _, _) = create_test_subscription(&pool, "suspended").await;

    // Suspending an already-suspended subscription should succeed (idempotent)
    transition_to_suspended(sub_id, "duplicate_suspension", &pool)
        .await
        .expect("Suspended → Suspended should be idempotent");
    assert_eq!(get_status(&pool, sub_id).await, "suspended");

    cleanup(&pool, sub_id).await;
}

// ============================================================================
// Outbox Event Verification
// ============================================================================

#[tokio::test]
async fn test_transition_to_past_due_emits_outbox_event() {
    let pool = setup_test_pool().await;
    let (sub_id, _, _) = create_test_subscription(&pool, "active").await;

    // Count outbox events before
    let before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE subject = 'subscriptions.status.changed'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    transition_to_past_due(sub_id, "payment_failed", &pool)
        .await
        .unwrap();

    // Count outbox events after
    let after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE subject = 'subscriptions.status.changed'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert!(after > before, "Should have emitted an outbox event");

    cleanup(&pool, sub_id).await;
}

#[tokio::test]
async fn test_transition_to_suspended_emits_outbox_event() {
    let pool = setup_test_pool().await;
    let (sub_id, _, _) = create_test_subscription(&pool, "past_due").await;

    let before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE subject = 'subscriptions.status.changed'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    transition_to_suspended(sub_id, "grace_expired", &pool)
        .await
        .unwrap();

    let after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE subject = 'subscriptions.status.changed'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert!(after > before, "Should have emitted an outbox event");

    cleanup(&pool, sub_id).await;
}

#[tokio::test]
async fn test_failed_transition_does_not_emit_outbox_event() {
    let pool = setup_test_pool().await;
    let (sub_id, _, _) = create_test_subscription(&pool, "suspended").await;

    let sub_id_str = sub_id.to_string();
    let before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE subject = 'subscriptions.status.changed' \
         AND payload::text LIKE '%' || $1 || '%'",
    )
    .bind(&sub_id_str)
    .fetch_one(&pool)
    .await
    .unwrap();

    // Illegal transition should fail
    let _ = transition_to_past_due(sub_id, "backwards", &pool).await;

    let after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE subject = 'subscriptions.status.changed' \
         AND payload::text LIKE '%' || $1 || '%'",
    )
    .bind(&sub_id_str)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(
        before, after,
        "Failed transition should not emit outbox event"
    );

    cleanup(&pool, sub_id).await;
}

// ============================================================================
// Consumer Event Handling: handle_invoice_suspended
// ============================================================================

#[tokio::test]
async fn test_consumer_suspends_active_subscription() {
    let pool = setup_test_pool().await;
    let (sub_id, tenant_id, ar_customer_id) = create_test_subscription(&pool, "active").await;

    let event = InvoiceSuspendedEvent {
        tenant_id: tenant_id.clone(),
        invoice_id: format!("inv-{}", Uuid::new_v4()),
        customer_id: ar_customer_id.clone(),
        dunning_attempt: 3,
        reason: "max_retries_exceeded".to_string(),
    };

    let event_id = Uuid::new_v4().to_string();
    let processed = handle_invoice_suspended(&pool, &event_id, &event)
        .await
        .expect("Consumer should handle event successfully");

    assert!(processed, "Event should be processed (not duplicate)");
    assert_eq!(get_status(&pool, sub_id).await, "suspended");

    cleanup(&pool, sub_id).await;
}

#[tokio::test]
async fn test_consumer_suspends_past_due_subscription() {
    let pool = setup_test_pool().await;
    let (sub_id, tenant_id, ar_customer_id) = create_test_subscription(&pool, "past_due").await;

    let event = InvoiceSuspendedEvent {
        tenant_id: tenant_id.clone(),
        invoice_id: format!("inv-{}", Uuid::new_v4()),
        customer_id: ar_customer_id.clone(),
        dunning_attempt: 3,
        reason: "max_retries_exceeded".to_string(),
    };

    let event_id = Uuid::new_v4().to_string();
    let processed = handle_invoice_suspended(&pool, &event_id, &event)
        .await
        .expect("Consumer should handle event successfully");

    assert!(processed);
    assert_eq!(get_status(&pool, sub_id).await, "suspended");

    cleanup(&pool, sub_id).await;
}

#[tokio::test]
async fn test_consumer_idempotent_duplicate_event() {
    let pool = setup_test_pool().await;
    let (sub_id, tenant_id, ar_customer_id) = create_test_subscription(&pool, "active").await;

    let event = InvoiceSuspendedEvent {
        tenant_id: tenant_id.clone(),
        invoice_id: format!("inv-{}", Uuid::new_v4()),
        customer_id: ar_customer_id.clone(),
        dunning_attempt: 3,
        reason: "max_retries_exceeded".to_string(),
    };

    let event_id = Uuid::new_v4().to_string();

    // First processing
    let first = handle_invoice_suspended(&pool, &event_id, &event)
        .await
        .expect("First call should succeed");
    assert!(first, "First call should process the event");

    // Duplicate processing (same event_id)
    let second = handle_invoice_suspended(&pool, &event_id, &event)
        .await
        .expect("Duplicate call should succeed");
    assert!(!second, "Duplicate call should be skipped (idempotent)");

    // Status should still be suspended from first call
    assert_eq!(get_status(&pool, sub_id).await, "suspended");

    cleanup(&pool, sub_id).await;
}

#[tokio::test]
async fn test_consumer_skips_already_suspended_subscription() {
    let pool = setup_test_pool().await;
    let (sub_id, tenant_id, ar_customer_id) = create_test_subscription(&pool, "suspended").await;

    let event = InvoiceSuspendedEvent {
        tenant_id: tenant_id.clone(),
        invoice_id: format!("inv-{}", Uuid::new_v4()),
        customer_id: ar_customer_id.clone(),
        dunning_attempt: 3,
        reason: "max_retries_exceeded".to_string(),
    };

    let event_id = Uuid::new_v4().to_string();

    // Should handle gracefully (suspended → suspended is idempotent)
    let processed = handle_invoice_suspended(&pool, &event_id, &event)
        .await
        .expect("Should handle already-suspended subscription");

    assert!(processed);
    assert_eq!(get_status(&pool, sub_id).await, "suspended");

    cleanup(&pool, sub_id).await;
}

#[tokio::test]
async fn test_consumer_no_matching_subscriptions() {
    let pool = setup_test_pool().await;

    let event = InvoiceSuspendedEvent {
        tenant_id: format!("nonexistent-tenant-{}", Uuid::new_v4()),
        invoice_id: format!("inv-{}", Uuid::new_v4()),
        customer_id: format!("nonexistent-customer-{}", Uuid::new_v4()),
        dunning_attempt: 3,
        reason: "max_retries_exceeded".to_string(),
    };

    let event_id = Uuid::new_v4().to_string();

    // Should succeed even with no matching subscriptions (logs warning)
    let processed = handle_invoice_suspended(&pool, &event_id, &event)
        .await
        .expect("Should handle no-match case gracefully");

    assert!(processed, "Event should still be marked as processed");
}
