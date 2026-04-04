//! Repository for close checklist items and approval signoffs.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct ChecklistItemRow {
    pub id: Uuid,
    pub tenant_id: String,
    pub period_id: Uuid,
    pub label: String,
    pub status: String,
    pub completed_by: Option<String>,
    pub completed_at: Option<DateTime<Utc>>,
    pub waive_reason: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct ApprovalRow {
    pub id: Uuid,
    pub tenant_id: String,
    pub period_id: Uuid,
    pub actor_id: String,
    pub approval_type: String,
    pub notes: Option<String>,
    pub approved_at: DateTime<Utc>,
}

pub async fn create_checklist_item(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    label: &str,
) -> Result<ChecklistItemRow, sqlx::Error> {
    sqlx::query_as::<_, ChecklistItemRow>(
        r#"
        INSERT INTO close_checklist_items (tenant_id, period_id, label)
        VALUES ($1, $2, $3)
        RETURNING id, tenant_id, period_id, label, status, completed_by, completed_at, waive_reason
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .bind(label)
    .fetch_one(pool)
    .await
}

pub async fn complete_checklist_item(
    pool: &PgPool,
    completed_by: &str,
    item_id: Uuid,
    period_id: Uuid,
    tenant_id: &str,
) -> Result<Option<ChecklistItemRow>, sqlx::Error> {
    sqlx::query_as::<_, ChecklistItemRow>(
        r#"
        UPDATE close_checklist_items
        SET status = 'complete', completed_by = $1, completed_at = NOW(), updated_at = NOW()
        WHERE id = $2 AND period_id = $3 AND tenant_id = $4
        RETURNING id, tenant_id, period_id, label, status, completed_by, completed_at, waive_reason
        "#,
    )
    .bind(completed_by)
    .bind(item_id)
    .bind(period_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
}

pub async fn waive_checklist_item(
    pool: &PgPool,
    completed_by: &str,
    waive_reason: &str,
    item_id: Uuid,
    period_id: Uuid,
    tenant_id: &str,
) -> Result<Option<ChecklistItemRow>, sqlx::Error> {
    sqlx::query_as::<_, ChecklistItemRow>(
        r#"
        UPDATE close_checklist_items
        SET status = 'waived', completed_by = $1, completed_at = NOW(),
            waive_reason = $2, updated_at = NOW()
        WHERE id = $3 AND period_id = $4 AND tenant_id = $5
        RETURNING id, tenant_id, period_id, label, status, completed_by, completed_at, waive_reason
        "#,
    )
    .bind(completed_by)
    .bind(waive_reason)
    .bind(item_id)
    .bind(period_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
}

pub async fn list_checklist_items(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
) -> Result<Vec<ChecklistItemRow>, sqlx::Error> {
    sqlx::query_as::<_, ChecklistItemRow>(
        r#"
        SELECT id, tenant_id, period_id, label, status, completed_by, completed_at, waive_reason
        FROM close_checklist_items
        WHERE tenant_id = $1 AND period_id = $2
        ORDER BY created_at ASC
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .fetch_all(pool)
    .await
}

pub async fn create_approval(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    actor_id: &str,
    approval_type: &str,
    notes: Option<&str>,
) -> Result<ApprovalRow, sqlx::Error> {
    sqlx::query_as::<_, ApprovalRow>(
        r#"
        INSERT INTO close_approvals (tenant_id, period_id, actor_id, approval_type, notes)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (tenant_id, period_id, approval_type) DO UPDATE
            SET actor_id = EXCLUDED.actor_id, notes = EXCLUDED.notes, approved_at = NOW()
        RETURNING id, tenant_id, period_id, actor_id, approval_type, notes, approved_at
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .bind(actor_id)
    .bind(approval_type)
    .bind(notes)
    .fetch_one(pool)
    .await
}

pub async fn list_approvals(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
) -> Result<Vec<ApprovalRow>, sqlx::Error> {
    sqlx::query_as::<_, ApprovalRow>(
        r#"
        SELECT id, tenant_id, period_id, actor_id, approval_type, notes, approved_at
        FROM close_approvals
        WHERE tenant_id = $1 AND period_id = $2
        ORDER BY approved_at ASC
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .fetch_all(pool)
    .await
}

pub async fn ping(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT 1").execute(pool).await?;
    Ok(())
}
