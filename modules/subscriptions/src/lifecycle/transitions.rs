//! Subscription lifecycle transition functions (database operations).
//!
//! Pattern: Guard → Mutation → Side Effect.
//! Agents modifying event payloads or DB schema should edit this file.

use sqlx::PgPool;
use uuid::Uuid;

use super::state_machine::{transition_guard, SubscriptionStatus, TransitionError};

// ============================================================================
// Lifecycle Functions (Guard → Mutation → Side Effect)
// ============================================================================

/// Transition subscription to PAST_DUE status.
///
/// # Lifecycle Order
/// 1. Guard: Validate transition is legal
/// 2. Mutation: Update status in database
/// 3. Side Effect: Emit subscriptions.status.changed event
pub async fn transition_to_past_due(
    subscription_id: Uuid,
    tenant_id: &str,
    reason: &str,
    pool: &PgPool,
) -> Result<(), TransitionError> {
    // Phase 16 bd-299f: Wrap in transaction for atomicity
    let mut tx = pool.begin().await?;

    // Fetch current status (scoped by tenant_id)
    let current_status = fetch_current_status_tx(&mut tx, subscription_id, tenant_id).await?;

    // Guard: Validate transition (ZERO side effects)
    transition_guard(current_status, SubscriptionStatus::PastDue, reason)?;

    // Mutation: Update status (within transaction, scoped by tenant_id)
    update_status_tx(&mut tx, subscription_id, tenant_id, SubscriptionStatus::PastDue).await?;

    // Side Effect: Emit subscriptions.status.changed event atomically within the same TX
    let status_payload = crate::models::SubscriptionStatusChangedPayload {
        subscription_id: subscription_id.to_string(),
        tenant_id: tenant_id.to_string(),
        from_status: current_status.as_str().to_string(),
        to_status: SubscriptionStatus::PastDue.as_str().to_string(),
        reason: reason.to_string(),
    };
    let envelope = crate::envelope::create_subscriptions_envelope(
        uuid::Uuid::new_v4(),
        tenant_id.to_string(),
        "subscriptions.status.changed".to_string(),
        None,
        None,
        "LIFECYCLE".to_string(),
        status_payload,
    );
    crate::outbox::enqueue_event_tx(&mut tx, "subscriptions.status.changed", &envelope).await?;

    tx.commit().await?;
    tracing::info!(
        subscription_id = %subscription_id,
        reason = reason,
        "Subscription transitioned to PAST_DUE"
    );

    Ok(())
}

/// Transition subscription to SUSPENDED status.
///
/// # Lifecycle Order
/// 1. Guard: Validate transition is legal
/// 2. Mutation: Update status in database (atomic with outbox)
/// 3. Side Effect: Emit subscriptions.status.changed event
pub async fn transition_to_suspended(
    subscription_id: Uuid,
    tenant_id: &str,
    reason: &str,
    pool: &PgPool,
) -> Result<(), TransitionError> {
    let mut tx = pool.begin().await?;

    // Fetch current status (within transaction, scoped by tenant_id)
    let current_status = fetch_current_status_tx(&mut tx, subscription_id, tenant_id).await?;

    // Guard: Validate transition (ZERO side effects)
    transition_guard(current_status, SubscriptionStatus::Suspended, reason)?;

    // Mutation: Update status (within transaction, scoped by tenant_id)
    update_status_tx(&mut tx, subscription_id, tenant_id, SubscriptionStatus::Suspended).await?;

    // Side Effect: Emit subscriptions.status.changed event atomically
    let status_payload = crate::models::SubscriptionStatusChangedPayload {
        subscription_id: subscription_id.to_string(),
        tenant_id: tenant_id.to_string(),
        from_status: current_status.as_str().to_string(),
        to_status: SubscriptionStatus::Suspended.as_str().to_string(),
        reason: reason.to_string(),
    };
    let envelope = crate::envelope::create_subscriptions_envelope(
        uuid::Uuid::new_v4(),
        tenant_id.to_string(),
        "subscriptions.status.changed".to_string(),
        None,
        None,
        "LIFECYCLE".to_string(),
        status_payload,
    );
    crate::outbox::enqueue_event_tx(&mut tx, "subscriptions.status.changed", &envelope).await?;

    tx.commit().await?;

    tracing::info!(
        subscription_id = %subscription_id,
        reason = reason,
        "Subscription transitioned to SUSPENDED"
    );

    Ok(())
}

/// Transition subscription to ACTIVE status (reactivation).
///
/// # Lifecycle Order
/// 1. Guard: Validate transition is legal
/// 2. Mutation: Update status in database
/// 3. Side Effect: (Future) Emit event, trigger notifications
pub async fn transition_to_active(
    subscription_id: Uuid,
    tenant_id: &str,
    reason: &str,
    pool: &PgPool,
) -> Result<(), TransitionError> {
    // Fetch current status (scoped by tenant_id)
    let current_status = fetch_current_status(subscription_id, tenant_id, pool).await?;

    // Guard: Validate transition (ZERO side effects)
    transition_guard(current_status, SubscriptionStatus::Active, reason)?;

    // Mutation: Update status (scoped by tenant_id)
    update_status(subscription_id, tenant_id, SubscriptionStatus::Active, pool).await?;

    tracing::info!(
        subscription_id = %subscription_id,
        reason = reason,
        "Subscription transitioned to ACTIVE"
    );

    Ok(())
}

// ============================================================================
// Internal Helpers (NOT exported)
// ============================================================================

/// Fetch current subscription status from database.
async fn fetch_current_status(
    subscription_id: Uuid,
    tenant_id: &str,
    pool: &PgPool,
) -> Result<SubscriptionStatus, TransitionError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT status FROM subscriptions WHERE id = $1 AND tenant_id = $2")
        .bind(subscription_id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?;

    match row {
        Some((status,)) => SubscriptionStatus::from_str(&status),
        None => Err(TransitionError::SubscriptionNotFound { subscription_id }),
    }
}

/// Update subscription status in database.
async fn update_status(
    subscription_id: Uuid,
    tenant_id: &str,
    new_status: SubscriptionStatus,
    pool: &PgPool,
) -> Result<(), TransitionError> {
    let rows_affected =
        sqlx::query("UPDATE subscriptions SET status = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3")
            .bind(new_status.as_str())
            .bind(subscription_id)
            .bind(tenant_id)
            .execute(pool)
            .await?
            .rows_affected();

    if rows_affected == 0 {
        return Err(TransitionError::SubscriptionNotFound { subscription_id });
    }

    Ok(())
}

/// Fetch current subscription status from database (transaction-aware).
///
/// **Phase 16 Atomicity:** This version operates within an existing transaction
/// to ensure status read + mutation + event emission are atomic.
pub(super) async fn fetch_current_status_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    subscription_id: Uuid,
    tenant_id: &str,
) -> Result<SubscriptionStatus, TransitionError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT status FROM subscriptions WHERE id = $1 AND tenant_id = $2")
        .bind(subscription_id)
        .bind(tenant_id)
        .fetch_optional(&mut **tx)
        .await?;

    match row {
        Some((status,)) => SubscriptionStatus::from_str(&status),
        None => Err(TransitionError::SubscriptionNotFound { subscription_id }),
    }
}

/// Update subscription status in database (transaction-aware).
///
/// **Phase 16 Atomicity:** This version operates within an existing transaction
/// to ensure mutation + event emission commit atomically.
pub(super) async fn update_status_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    subscription_id: Uuid,
    tenant_id: &str,
    new_status: SubscriptionStatus,
) -> Result<(), TransitionError> {
    let rows_affected =
        sqlx::query("UPDATE subscriptions SET status = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3")
            .bind(new_status.as_str())
            .bind(subscription_id)
            .bind(tenant_id)
            .execute(&mut **tx)
            .await?
            .rows_affected();

    if rows_affected == 0 {
        return Err(TransitionError::SubscriptionNotFound { subscription_id });
    }

    Ok(())
}
