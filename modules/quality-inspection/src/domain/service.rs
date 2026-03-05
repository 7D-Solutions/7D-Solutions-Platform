use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::models::*;
use crate::domain::outbox::enqueue_event;
use crate::events::{self, QualityInspectionEventType};

#[derive(Debug, Error)]
pub enum QiError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Validation: {0}")]
    Validation(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Inspection Plans
// ============================================================================

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

// ============================================================================
// Receiving Inspections
// ============================================================================

pub async fn create_receiving_inspection(
    pool: &PgPool,
    tenant_id: &str,
    req: &CreateReceivingInspectionRequest,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<Inspection, QiError> {
    if tenant_id.is_empty() {
        return Err(QiError::Validation("tenant_id is required".into()));
    }

    let result_val = req.result.as_deref().unwrap_or("pending");
    match result_val {
        "pending" | "pass" | "fail" | "conditional" => {}
        other => {
            return Err(QiError::Validation(format!(
                "Invalid result '{}', expected one of: pending, pass, fail, conditional",
                other
            )));
        }
    }

    let mut tx = pool.begin().await?;

    let inspection = sqlx::query_as::<_, Inspection>(
        r#"
        INSERT INTO inspections
            (tenant_id, plan_id, lot_id, inspector_id, inspection_type,
             result, notes, receipt_id, part_id, part_revision,
             inspected_at)
        VALUES ($1, $2, $3, $4, 'receiving', $5, $6, $7, $8, $9,
                CASE WHEN $5 != 'pending' THEN NOW() ELSE NULL END)
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(req.plan_id)
    .bind(req.lot_id)
    .bind(req.inspector_id)
    .bind(result_val)
    .bind(&req.notes)
    .bind(req.receipt_id)
    .bind(req.part_id)
    .bind(&req.part_revision)
    .fetch_one(&mut *tx)
    .await?;

    enqueue_event(
        &mut tx,
        tenant_id,
        QualityInspectionEventType::InspectionRecorded,
        "inspection",
        &inspection.id.to_string(),
        &events::build_inspection_recorded_envelope(
            inspection.id,
            tenant_id.to_string(),
            req.receipt_id,
            req.part_id,
            req.part_revision.clone(),
            result_val.to_string(),
            correlation_id.to_string(),
            causation_id.map(String::from),
        ),
        correlation_id,
        causation_id,
    )
    .await?;

    tx.commit().await?;
    Ok(inspection)
}

pub async fn get_inspection(
    pool: &PgPool,
    tenant_id: &str,
    inspection_id: Uuid,
) -> Result<Inspection, QiError> {
    sqlx::query_as::<_, Inspection>(
        "SELECT * FROM inspections WHERE id = $1 AND tenant_id = $2",
    )
    .bind(inspection_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| QiError::NotFound("Inspection not found".into()))
}

// ============================================================================
// Query: by part revision, by receipt
// ============================================================================

pub async fn list_inspections_by_part_rev(
    pool: &PgPool,
    tenant_id: &str,
    part_id: Uuid,
    part_revision: Option<&str>,
) -> Result<Vec<Inspection>, QiError> {
    let rows = if let Some(rev) = part_revision {
        sqlx::query_as::<_, Inspection>(
            r#"
            SELECT * FROM inspections
            WHERE tenant_id = $1 AND part_id = $2 AND part_revision = $3
            ORDER BY created_at DESC
            "#,
        )
        .bind(tenant_id)
        .bind(part_id)
        .bind(rev)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, Inspection>(
            r#"
            SELECT * FROM inspections
            WHERE tenant_id = $1 AND part_id = $2
            ORDER BY created_at DESC
            "#,
        )
        .bind(tenant_id)
        .bind(part_id)
        .fetch_all(pool)
        .await?
    };
    Ok(rows)
}

pub async fn list_inspections_by_receipt(
    pool: &PgPool,
    tenant_id: &str,
    receipt_id: Uuid,
) -> Result<Vec<Inspection>, QiError> {
    let rows = sqlx::query_as::<_, Inspection>(
        r#"
        SELECT * FROM inspections
        WHERE tenant_id = $1 AND receipt_id = $2
        ORDER BY created_at DESC
        "#,
    )
    .bind(tenant_id)
    .bind(receipt_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
