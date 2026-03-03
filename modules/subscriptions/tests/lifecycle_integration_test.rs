//! Integration tests for subscription lifecycle module
//!
//! Tests verify:
//! 1. Transition guards reject illegal transitions
//! 2. Lifecycle functions correctly update database status
//! 3. Routes cannot mutate status without calling lifecycle API (future)
//! 4. Concurrency safety (future)

use sqlx::PgPool;
use subscriptions_rs::lifecycle::{
    transition_guard, transition_to_active, transition_to_past_due, transition_to_suspended,
    SubscriptionStatus, TransitionError,
};
use uuid::Uuid;

// Test helper to create a test database pool
async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://subscriptions_user:subscriptions_pass@localhost:5435/subscriptions_db?sslmode=disable".to_string()
    });

    let pool = sqlx::PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    // Run migrations
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

// Test helper to create a subscription in a specific status
async fn create_test_subscription(pool: &PgPool, status: &str) -> Uuid {
    let tenant_id = format!("test-tenant-{}", Uuid::new_v4());
    let ar_customer_id = format!("customer-{}", Uuid::new_v4());

    // Create a subscription plan first
    let plan_id: Uuid = sqlx::query_scalar(
        "INSERT INTO subscription_plans (tenant_id, name, schedule, price_minor, currency)
         VALUES ($1, 'Test Plan', 'monthly', 2999, 'USD')
         RETURNING id",
    )
    .bind(&tenant_id)
    .fetch_one(pool)
    .await
    .expect("Failed to create test plan");

    // Create subscription
    let subscription_id: Uuid = sqlx::query_scalar(
        "INSERT INTO subscriptions (tenant_id, ar_customer_id, plan_id, status, schedule, price_minor, currency, start_date, next_bill_date)
         VALUES ($1, $2, $3, $4, 'monthly', 2999, 'USD', CURRENT_DATE, CURRENT_DATE + INTERVAL '1 month')
         RETURNING id"
    )
    .bind(&tenant_id)
    .bind(&ar_customer_id)
    .bind(plan_id)
    .bind(status)
    .fetch_one(pool)
    .await
    .expect("Failed to create test subscription");

    subscription_id
}

// Test helper to fetch subscription status
async fn get_subscription_status(pool: &PgPool, subscription_id: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM subscriptions WHERE id = $1")
        .bind(subscription_id)
        .fetch_one(pool)
        .await
        .expect("Failed to fetch subscription status")
}

// ============================================================================
// Transition Tests: ACTIVE → PAST_DUE
// ============================================================================

#[tokio::test]
async fn test_transition_active_to_past_due() {
    let pool = setup_test_pool().await;
    let subscription_id = create_test_subscription(&pool, "active").await;

    let result = transition_guard(
        SubscriptionStatus::Active,
        SubscriptionStatus::PastDue,
        "payment_failed",
    );

    assert!(result.is_ok(), "Active → PastDue should be allowed");

    // Clean up
    sqlx::query("DELETE FROM subscriptions WHERE id = $1")
        .bind(subscription_id)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn test_lifecycle_function_active_to_past_due() {
    let pool = setup_test_pool().await;
    let subscription_id = create_test_subscription(&pool, "active").await;

    // Call lifecycle function
    transition_to_past_due(subscription_id, "payment_failed", &pool)
        .await
        .expect("Transition should succeed");

    // Verify status was updated
    let status = get_subscription_status(&pool, subscription_id).await;
    assert_eq!(status, "past_due");

    // Clean up
    sqlx::query("DELETE FROM subscriptions WHERE id = $1")
        .bind(subscription_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// Transition Tests: PAST_DUE → SUSPENDED
// ============================================================================

#[tokio::test]
async fn test_transition_past_due_to_suspended() {
    let result = transition_guard(
        SubscriptionStatus::PastDue,
        SubscriptionStatus::Suspended,
        "grace_period_expired",
    );

    assert!(result.is_ok(), "PastDue → Suspended should be allowed");
}

#[tokio::test]
async fn test_lifecycle_function_past_due_to_suspended() {
    let pool = setup_test_pool().await;
    let subscription_id = create_test_subscription(&pool, "past_due").await;

    // Call lifecycle function
    transition_to_suspended(subscription_id, "grace_period_expired", &pool)
        .await
        .expect("Transition should succeed");

    // Verify status was updated
    let status = get_subscription_status(&pool, subscription_id).await;
    assert_eq!(status, "suspended");

    // Clean up
    sqlx::query("DELETE FROM subscriptions WHERE id = $1")
        .bind(subscription_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// Transition Tests: SUSPENDED → ACTIVE (Return Path)
// ============================================================================

#[tokio::test]
async fn test_transition_suspended_to_active() {
    let result = transition_guard(
        SubscriptionStatus::Suspended,
        SubscriptionStatus::Active,
        "payment_recovered",
    );

    assert!(result.is_ok(), "Suspended → Active should be allowed");
}

#[tokio::test]
async fn test_lifecycle_function_suspended_to_active() {
    let pool = setup_test_pool().await;
    let subscription_id = create_test_subscription(&pool, "suspended").await;

    // Call lifecycle function
    transition_to_active(subscription_id, "payment_recovered", &pool)
        .await
        .expect("Transition should succeed");

    // Verify status was updated
    let status = get_subscription_status(&pool, subscription_id).await;
    assert_eq!(status, "active");

    // Clean up
    sqlx::query("DELETE FROM subscriptions WHERE id = $1")
        .bind(subscription_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// Transition Tests: PAST_DUE → ACTIVE (Payment Recovered)
// ============================================================================

#[tokio::test]
async fn test_transition_past_due_to_active() {
    let result = transition_guard(
        SubscriptionStatus::PastDue,
        SubscriptionStatus::Active,
        "payment_recovered",
    );

    assert!(result.is_ok(), "PastDue → Active should be allowed");
}

#[tokio::test]
async fn test_lifecycle_function_past_due_to_active() {
    let pool = setup_test_pool().await;
    let subscription_id = create_test_subscription(&pool, "past_due").await;

    // Call lifecycle function
    transition_to_active(subscription_id, "payment_recovered", &pool)
        .await
        .expect("Transition should succeed");

    // Verify status was updated
    let status = get_subscription_status(&pool, subscription_id).await;
    assert_eq!(status, "active");

    // Clean up
    sqlx::query("DELETE FROM subscriptions WHERE id = $1")
        .bind(subscription_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// Illegal Transition Tests
// ============================================================================

#[tokio::test]
async fn test_illegal_transition_active_to_suspended() {
    // NOTE: Active → Suspended IS legal (dunning terminal escalation).
    // Test an actually illegal transition instead: Suspended → PastDue.
    let result = transition_guard(
        SubscriptionStatus::Suspended,
        SubscriptionStatus::PastDue,
        "backwards",
    );

    assert!(result.is_err(), "Suspended → PastDue should be rejected");
    match result {
        Err(TransitionError::IllegalTransition { from, to, .. }) => {
            assert_eq!(from, "suspended");
            assert_eq!(to, "past_due");
        }
        _ => panic!("Expected IllegalTransition error"),
    }
}

#[tokio::test]
async fn test_illegal_transition_suspended_to_past_due() {
    let result = transition_guard(
        SubscriptionStatus::Suspended,
        SubscriptionStatus::PastDue,
        "backwards",
    );

    assert!(result.is_err(), "Suspended → PastDue should be rejected");
    match result {
        Err(TransitionError::IllegalTransition { from, to, .. }) => {
            assert_eq!(from, "suspended");
            assert_eq!(to, "past_due");
        }
        _ => panic!("Expected IllegalTransition error"),
    }
}

#[tokio::test]
async fn test_lifecycle_function_rejects_illegal_transition() {
    let pool = setup_test_pool().await;
    let subscription_id = create_test_subscription(&pool, "active").await;

    // Attempt illegal transition: Active → PastDue → then try Suspended → PastDue (backwards)
    // First set the subscription to suspended state
    transition_to_past_due(subscription_id, "payment_failed", &pool)
        .await
        .unwrap();
    transition_to_suspended(subscription_id, "grace_expired", &pool)
        .await
        .unwrap();

    // Now attempt illegal Suspended → PastDue (backwards)
    let result = transition_to_past_due(subscription_id, "backwards", &pool).await;

    assert!(
        result.is_err(),
        "Lifecycle function should reject illegal transition"
    );

    // Verify status was NOT updated
    let status = get_subscription_status(&pool, subscription_id).await;
    assert_eq!(status, "suspended", "Status should remain unchanged");

    // Clean up
    sqlx::query("DELETE FROM subscriptions WHERE id = $1")
        .bind(subscription_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// Idempotency Tests
// ============================================================================

#[tokio::test]
async fn test_idempotent_transition_active_to_active() {
    let result = transition_guard(
        SubscriptionStatus::Active,
        SubscriptionStatus::Active,
        "idempotent",
    );

    assert!(
        result.is_ok(),
        "Active → Active should be allowed (idempotent)"
    );
}

#[tokio::test]
async fn test_lifecycle_function_idempotent() {
    let pool = setup_test_pool().await;
    let subscription_id = create_test_subscription(&pool, "past_due").await;

    // First transition
    transition_to_past_due(subscription_id, "idempotent_test", &pool)
        .await
        .expect("First call should succeed");

    // Second transition (idempotent)
    transition_to_past_due(subscription_id, "idempotent_test", &pool)
        .await
        .expect("Second call should succeed (idempotent)");

    // Verify status
    let status = get_subscription_status(&pool, subscription_id).await;
    assert_eq!(status, "past_due");

    // Clean up
    sqlx::query("DELETE FROM subscriptions WHERE id = $1")
        .bind(subscription_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[tokio::test]
async fn test_lifecycle_function_subscription_not_found() {
    let pool = setup_test_pool().await;
    let non_existent_id = Uuid::new_v4();

    let result = transition_to_past_due(non_existent_id, "test", &pool).await;

    assert!(
        result.is_err(),
        "Should fail when subscription doesn't exist"
    );
    match result {
        Err(TransitionError::SubscriptionNotFound { subscription_id }) => {
            assert_eq!(subscription_id, non_existent_id);
        }
        _ => panic!("Expected SubscriptionNotFound error"),
    }
}
