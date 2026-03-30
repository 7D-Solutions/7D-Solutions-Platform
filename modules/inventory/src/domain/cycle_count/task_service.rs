//! Cycle count task creation service.
//!
//! Creates a cycle_count_task + cycle_count_lines in a single transaction.
//! Stock changes are NOT applied here — adjustments happen on submit (bd-1q0j).
//!
//! Full scope:  lines are auto-populated from item_on_hand for the location.
//! Partial scope: caller provides explicit item_ids; expected_qty still fetched.
//!
//! Guards:
//!   - tenant_id must be non-empty
//!   - location_id must exist, be active, and belong to tenant + warehouse
//!   - partial scope requires at least one item_id

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

// ============================================================================
// Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum TaskScope {
    Full,
    Partial,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateTaskRequest {
    pub tenant_id: String,
    pub warehouse_id: Uuid,
    pub location_id: Uuid,
    pub scope: TaskScope,
    /// Required when scope = partial; ignored (and safe to omit) for full.
    #[serde(default)]
    pub item_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TaskLine {
    pub line_id: Uuid,
    pub item_id: Uuid,
    pub expected_qty: i64,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateTaskResult {
    pub task_id: Uuid,
    pub tenant_id: String,
    pub warehouse_id: Uuid,
    pub location_id: Uuid,
    pub scope: TaskScope,
    pub status: String,
    pub line_count: usize,
    pub lines: Vec<TaskLine>,
}

#[derive(Debug, Error)]
pub enum TaskError {
    #[error("tenant_id is required")]
    MissingTenant,

    #[error("partial scope requires at least one item_id")]
    EmptyPartialItemList,

    #[error("location not found, inactive, or does not belong to this tenant/warehouse")]
    LocationNotFound,

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Service
// ============================================================================

/// Create a cycle count task with its snapshot lines.
///
/// Returns the created task including all line details.
pub async fn create_cycle_count_task(
    pool: &PgPool,
    req: &CreateTaskRequest,
) -> Result<CreateTaskResult, TaskError> {
    validate_request(req)?;

    // Guard: location must exist, be active, and belong to this tenant/warehouse
    let loc_exists: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM locations
            WHERE id = $1
              AND tenant_id = $2
              AND warehouse_id = $3
              AND is_active = TRUE
        )
        "#,
    )
    .bind(req.location_id)
    .bind(&req.tenant_id)
    .bind(req.warehouse_id)
    .fetch_one(pool)
    .await?;

    if !loc_exists {
        return Err(TaskError::LocationNotFound);
    }

    let mut tx = pool.begin().await?;

    // Insert the task header
    let scope_str = match req.scope {
        TaskScope::Full => "full",
        TaskScope::Partial => "partial",
    };

    let task_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO cycle_count_tasks
            (tenant_id, warehouse_id, location_id, scope, status)
        VALUES ($1, $2, $3, $4::cycle_count_scope, 'open')
        RETURNING id
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.warehouse_id)
    .bind(req.location_id)
    .bind(scope_str)
    .fetch_one(&mut *tx)
    .await?;

    // Determine the item set for lines
    let item_ids: Vec<Uuid> = match req.scope {
        TaskScope::Full => {
            // All items with on-hand stock at this specific location
            sqlx::query_scalar::<_, Uuid>(
                r#"
                SELECT item_id
                FROM item_on_hand
                WHERE tenant_id  = $1
                  AND warehouse_id = $2
                  AND location_id  = $3
                  AND quantity_on_hand > 0
                "#,
            )
            .bind(&req.tenant_id)
            .bind(req.warehouse_id)
            .bind(req.location_id)
            .fetch_all(&mut *tx)
            .await?
        }
        TaskScope::Partial => req.item_ids.clone(),
    };

    // Insert lines with snapshotted expected_qty
    let mut lines: Vec<TaskLine> = Vec::with_capacity(item_ids.len());
    for item_id in &item_ids {
        let expected_qty: i64 = sqlx::query_scalar(
            r#"
            SELECT COALESCE(quantity_on_hand, 0)
            FROM item_on_hand
            WHERE tenant_id   = $1
              AND item_id     = $2
              AND warehouse_id = $3
              AND location_id  = $4
            "#,
        )
        .bind(&req.tenant_id)
        .bind(item_id)
        .bind(req.warehouse_id)
        .bind(req.location_id)
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(0);

        let line_id = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO cycle_count_lines
                (task_id, tenant_id, item_id, expected_qty)
            VALUES ($1, $2, $3, $4)
            RETURNING id
            "#,
        )
        .bind(task_id)
        .bind(&req.tenant_id)
        .bind(item_id)
        .bind(expected_qty)
        .fetch_one(&mut *tx)
        .await?;

        lines.push(TaskLine {
            line_id,
            item_id: *item_id,
            expected_qty,
        });
    }

    tx.commit().await?;

    Ok(CreateTaskResult {
        task_id,
        tenant_id: req.tenant_id.clone(),
        warehouse_id: req.warehouse_id,
        location_id: req.location_id,
        scope: req.scope.clone(),
        status: "open".to_string(),
        line_count: lines.len(),
        lines,
    })
}

// ============================================================================
// Validation
// ============================================================================

fn validate_request(req: &CreateTaskRequest) -> Result<(), TaskError> {
    if req.tenant_id.trim().is_empty() {
        return Err(TaskError::MissingTenant);
    }
    if req.scope == TaskScope::Partial && req.item_ids.is_empty() {
        return Err(TaskError::EmptyPartialItemList);
    }
    Ok(())
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn req(scope: TaskScope, item_ids: Vec<Uuid>) -> CreateTaskRequest {
        CreateTaskRequest {
            tenant_id: "t1".to_string(),
            warehouse_id: Uuid::new_v4(),
            location_id: Uuid::new_v4(),
            scope,
            item_ids,
        }
    }

    #[test]
    fn validate_rejects_empty_tenant() {
        let mut r = req(TaskScope::Full, vec![]);
        r.tenant_id = "".to_string();
        assert!(matches!(
            validate_request(&r),
            Err(TaskError::MissingTenant)
        ));
    }

    #[test]
    fn validate_rejects_partial_with_no_items() {
        let r = req(TaskScope::Partial, vec![]);
        assert!(matches!(
            validate_request(&r),
            Err(TaskError::EmptyPartialItemList)
        ));
    }

    #[test]
    fn validate_accepts_full_with_no_items() {
        let r = req(TaskScope::Full, vec![]);
        assert!(validate_request(&r).is_ok());
    }

    #[test]
    fn validate_accepts_partial_with_items() {
        let r = req(TaskScope::Partial, vec![Uuid::new_v4()]);
        assert!(validate_request(&r).is_ok());
    }
}
