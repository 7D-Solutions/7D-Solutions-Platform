//! Training delivery read-only queries.

use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::service::ServiceError;

use super::{
    TrainingAssignment, TrainingAssignmentRow, TrainingCompletion, TrainingCompletionRow,
    TrainingPlan, TrainingPlanRow,
};

pub async fn get_training_plan(
    pool: &PgPool,
    tenant_id: &str,
    plan_id: Uuid,
) -> Result<Option<TrainingPlan>, ServiceError> {
    let row = sqlx::query_as::<_, TrainingPlanRow>(
        r#"
        SELECT id, tenant_id, plan_code, title, description, artifact_id,
               duration_minutes, instructor_id, material_refs, required_for_artifact_codes,
               location, scheduled_at, active, created_at, updated_at, updated_by
        FROM wc_training_plans WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(plan_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(Into::into))
}

pub async fn list_training_plans(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<TrainingPlan>, ServiceError> {
    let rows = sqlx::query_as::<_, TrainingPlanRow>(
        r#"
        SELECT id, tenant_id, plan_code, title, description, artifact_id,
               duration_minutes, instructor_id, material_refs, required_for_artifact_codes,
               location, scheduled_at, active, created_at, updated_at, updated_by
        FROM wc_training_plans WHERE tenant_id = $1 ORDER BY created_at DESC
        "#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn get_training_assignment(
    pool: &PgPool,
    tenant_id: &str,
    assignment_id: Uuid,
) -> Result<Option<TrainingAssignment>, ServiceError> {
    let row = sqlx::query_as::<_, TrainingAssignmentRow>(
        r#"
        SELECT id, tenant_id, plan_id, operator_id, assigned_by, assigned_at,
               status, scheduled_at, notes, updated_at
        FROM wc_training_assignments WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(assignment_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(Into::into))
}

pub async fn list_training_assignments(
    pool: &PgPool,
    tenant_id: &str,
    plan_id: Option<Uuid>,
    operator_id: Option<Uuid>,
) -> Result<Vec<TrainingAssignment>, ServiceError> {
    let rows = sqlx::query_as::<_, TrainingAssignmentRow>(
        r#"
        SELECT id, tenant_id, plan_id, operator_id, assigned_by, assigned_at,
               status, scheduled_at, notes, updated_at
        FROM wc_training_assignments
        WHERE tenant_id = $1
          AND ($2::UUID IS NULL OR plan_id = $2)
          AND ($3::UUID IS NULL OR operator_id = $3)
        ORDER BY assigned_at DESC
        "#,
    )
    .bind(tenant_id)
    .bind(plan_id)
    .bind(operator_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn list_training_completions(
    pool: &PgPool,
    tenant_id: &str,
    plan_id: Option<Uuid>,
    operator_id: Option<Uuid>,
) -> Result<Vec<TrainingCompletion>, ServiceError> {
    let rows = sqlx::query_as::<_, TrainingCompletionRow>(
        r#"
        SELECT id, tenant_id, assignment_id, operator_id, plan_id, completed_at,
               verified_by, outcome, notes, resulting_competence_assignment_id, created_at
        FROM wc_training_completions
        WHERE tenant_id = $1
          AND ($2::UUID IS NULL OR plan_id = $2)
          AND ($3::UUID IS NULL OR operator_id = $3)
        ORDER BY completed_at DESC
        "#,
    )
    .bind(tenant_id)
    .bind(plan_id)
    .bind(operator_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}
