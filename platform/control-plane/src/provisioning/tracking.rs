//! Provisioning step status tracking
//!
//! Database operations for seeding, updating, and querying provisioning step
//! rows in the `provisioning_steps` table.

use chrono::Utc;
use sqlx::PgPool;
use tenant_registry::step_names;
use uuid::Uuid;

use super::steps::StepError;

/// Seed all 7 provisioning step rows for a tenant (all pending).
pub async fn seed_provisioning_steps(pool: &PgPool, tenant_id: Uuid) -> Result<(), StepError> {
    let now = Utc::now();
    let steps = tenant_registry::standard_provisioning_sequence();

    for step in &steps {
        sqlx::query(
            "INSERT INTO provisioning_steps \
             (step_id, tenant_id, step_name, step_order, status, created_at) \
             VALUES ($1, $2, $3, $4, 'pending', $5) \
             ON CONFLICT (tenant_id, step_name) DO NOTHING",
        )
        .bind(Uuid::new_v4())
        .bind(tenant_id)
        .bind(step.step_name)
        .bind(step.step_order)
        .bind(now)
        .execute(pool)
        .await?;
    }

    Ok(())
}

/// Mark a step as in_progress
pub async fn mark_step_in_progress(
    pool: &PgPool,
    tenant_id: Uuid,
    step_name: &str,
) -> Result<(), StepError> {
    sqlx::query(
        "UPDATE provisioning_steps SET status = 'in_progress', started_at = $1 \
         WHERE tenant_id = $2 AND step_name = $3",
    )
    .bind(Utc::now())
    .bind(tenant_id)
    .bind(step_name)
    .execute(pool)
    .await?;
    Ok(())
}

/// Mark a step as completed with verification result
pub async fn mark_step_completed(
    pool: &PgPool,
    tenant_id: Uuid,
    step_name: &str,
    checks: &serde_json::Value,
) -> Result<(), StepError> {
    sqlx::query(
        "UPDATE provisioning_steps \
         SET status = 'completed', completed_at = $1, verification_result = $2 \
         WHERE tenant_id = $3 AND step_name = $4",
    )
    .bind(Utc::now())
    .bind(checks)
    .bind(tenant_id)
    .bind(step_name)
    .execute(pool)
    .await?;
    Ok(())
}

/// Mark a step as failed
pub async fn mark_step_failed(
    pool: &PgPool,
    tenant_id: Uuid,
    step_name: &str,
    error: &str,
) -> Result<(), StepError> {
    sqlx::query(
        "UPDATE provisioning_steps \
         SET status = 'failed', completed_at = $1, error_message = $2 \
         WHERE tenant_id = $3 AND step_name = $4",
    )
    .bind(Utc::now())
    .bind(error)
    .bind(tenant_id)
    .bind(step_name)
    .execute(pool)
    .await?;
    Ok(())
}

/// Get the first non-completed step for resume-from-last.
/// Returns None if all steps are completed.
pub async fn first_pending_step(
    pool: &PgPool,
    tenant_id: Uuid,
) -> Result<Option<String>, StepError> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT step_name FROM provisioning_steps \
         WHERE tenant_id = $1 AND status != 'completed' \
         ORDER BY step_order ASC LIMIT 1",
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|(name,)| name))
}

/// Write a hook event to the outbox for vertical participation.
pub async fn write_hook_event(
    pool: &PgPool,
    tenant_id: Uuid,
    event_type: &str,
    payload: serde_json::Value,
) -> Result<(), StepError> {
    sqlx::query(
        "INSERT INTO provisioning_outbox (tenant_id, event_type, payload, created_at) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(tenant_id)
    .bind(event_type)
    .bind(&payload)
    .bind(Utc::now())
    .execute(pool)
    .await?;
    Ok(())
}

/// Returns true if the given step is fatal (should not be retried).
pub fn is_fatal_step(step_name: &str) -> bool {
    matches!(
        step_name,
        step_names::VALIDATE_TENANT_ID | step_names::ACTIVATE_TENANT
    )
}
