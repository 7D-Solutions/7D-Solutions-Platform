use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::models::*;
use crate::domain::outbox::enqueue_event;
use crate::domain::service::QiError;
use crate::events::{self, QualityInspectionEventType};

pub async fn create_inspection_plan(
    pool: &PgPool,
    tenant_id: &str,
    req: &CreateInspectionPlanRequest,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<InspectionPlan, QiError> {
    if tenant_id.is_empty() {
        return Err(QiError::Validation("tenant_id is required".into()));
    }
    if req.plan_name.trim().is_empty() {
        return Err(QiError::Validation("plan_name is required".into()));
    }

    let revision = req.revision.as_deref().unwrap_or("A");
    let sampling = req.sampling_method.as_deref().unwrap_or("full");
    let chars_json = serde_json::to_value(&req.characteristics)?;

    let mut tx = pool.begin().await?;

    let plan = sqlx::query_as::<_, InspectionPlan>(
        r#"
        INSERT INTO inspection_plans
            (tenant_id, part_id, plan_name, revision, characteristics,
             sampling_method, sample_size)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(req.part_id)
    .bind(&req.plan_name)
    .bind(revision)
    .bind(&chars_json)
    .bind(sampling)
    .bind(req.sample_size)
    .fetch_one(&mut *tx)
    .await?;

    enqueue_event(
        &mut tx,
        tenant_id,
        QualityInspectionEventType::InspectionPlanCreated,
        "inspection_plan",
        &plan.id.to_string(),
        &events::build_plan_created_envelope(
            plan.id,
            tenant_id.to_string(),
            req.part_id,
            plan.revision.clone(),
            correlation_id.to_string(),
            causation_id.map(String::from),
        ),
        correlation_id,
        causation_id,
    )
    .await?;

    tx.commit().await?;
    Ok(plan)
}

pub async fn get_inspection_plan(
    pool: &PgPool,
    tenant_id: &str,
    plan_id: Uuid,
) -> Result<InspectionPlan, QiError> {
    sqlx::query_as::<_, InspectionPlan>(
        "SELECT * FROM inspection_plans WHERE id = $1 AND tenant_id = $2",
    )
    .bind(plan_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| QiError::NotFound("Inspection plan not found".into()))
}

pub async fn activate_plan(
    pool: &PgPool,
    tenant_id: &str,
    plan_id: Uuid,
) -> Result<InspectionPlan, QiError> {
    let plan = get_inspection_plan(pool, tenant_id, plan_id).await?;
    if plan.status != "draft" {
        return Err(QiError::Validation(format!(
            "Plan status is '{}', expected 'draft'",
            plan.status
        )));
    }

    let updated = sqlx::query_as::<_, InspectionPlan>(
        r#"
        UPDATE inspection_plans
        SET status = 'active', updated_at = NOW()
        WHERE id = $1 AND tenant_id = $2
        RETURNING *
        "#,
    )
    .bind(plan_id)
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    Ok(updated)
}
