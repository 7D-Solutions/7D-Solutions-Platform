use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::models::*;
use crate::domain::outbox::enqueue_event;
use crate::domain::service::QiError;
use crate::events::{self, QualityInspectionEventType};

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
            "receiving".to_string(),
            req.receipt_id,
            None,
            None,
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

// ============================================================================
// In-Process Inspections
// ============================================================================

pub async fn create_in_process_inspection(
    pool: &PgPool,
    tenant_id: &str,
    req: &CreateInProcessInspectionRequest,
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
             result, notes, wo_id, op_instance_id, part_id, part_revision,
             inspected_at)
        VALUES ($1, $2, $3, $4, 'in_process', $5, $6, $7, $8, $9, $10,
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
    .bind(req.wo_id)
    .bind(req.op_instance_id)
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
            "in_process".to_string(),
            None,
            Some(req.wo_id),
            Some(req.op_instance_id),
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

// ============================================================================
// Final Inspections
// ============================================================================

pub async fn create_final_inspection(
    pool: &PgPool,
    tenant_id: &str,
    req: &CreateFinalInspectionRequest,
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
             result, notes, wo_id, part_id, part_revision,
             inspected_at)
        VALUES ($1, $2, $3, $4, 'final', $5, $6, $7, $8, $9,
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
    .bind(req.wo_id)
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
            "final".to_string(),
            None,
            Some(req.wo_id),
            None,
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
// Disposition state machine
// ============================================================================

fn validate_disposition_transition(current: &str, target: &str) -> Result<(), QiError> {
    let allowed = match current {
        "pending" => &["held"][..],
        "held" => &["accepted", "rejected", "released"][..],
        _ => &[][..],
    };
    if allowed.contains(&target) {
        Ok(())
    } else {
        Err(QiError::Validation(format!(
            "Cannot transition from '{}' to '{}'. Allowed transitions from '{}': {:?}",
            current, target, current, allowed
        )))
    }
}

async fn transition_disposition(
    pool: &PgPool,
    wc_pool: &PgPool,
    tenant_id: &str,
    inspection_id: Uuid,
    target: &str,
    event_type: QualityInspectionEventType,
    inspector_id: Option<Uuid>,
    reason: Option<&str>,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<Inspection, QiError> {
    let iid = inspector_id.ok_or_else(|| {
        QiError::Validation("inspector_id is required for disposition actions".into())
    })?;
    crate::domain::wc_client::verify_inspector_authorized(wc_pool, tenant_id, iid).await?;

    let inspection = get_inspection(pool, tenant_id, inspection_id).await?;
    validate_disposition_transition(&inspection.disposition, target)?;

    let mut tx = pool.begin().await?;

    let updated = sqlx::query_as::<_, Inspection>(
        r#"
        UPDATE inspections
        SET disposition = $1, updated_at = NOW()
        WHERE id = $2 AND tenant_id = $3
        RETURNING *
        "#,
    )
    .bind(target)
    .bind(inspection_id)
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await?;

    enqueue_event(
        &mut tx,
        tenant_id,
        event_type,
        "inspection",
        &inspection_id.to_string(),
        &events::build_disposition_transition_envelope(
            event_type,
            inspection_id,
            tenant_id.to_string(),
            inspection.disposition.clone(),
            target.to_string(),
            inspector_id,
            reason.map(String::from),
            correlation_id.to_string(),
            causation_id.map(String::from),
        ),
        correlation_id,
        causation_id,
    )
    .await?;

    tx.commit().await?;
    Ok(updated)
}

pub async fn hold_inspection(
    pool: &PgPool,
    wc_pool: &PgPool,
    tenant_id: &str,
    inspection_id: Uuid,
    inspector_id: Option<Uuid>,
    reason: Option<&str>,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<Inspection, QiError> {
    transition_disposition(
        pool,
        wc_pool,
        tenant_id,
        inspection_id,
        "held",
        QualityInspectionEventType::InspectionHeld,
        inspector_id,
        reason,
        correlation_id,
        causation_id,
    )
    .await
}

pub async fn release_inspection(
    pool: &PgPool,
    wc_pool: &PgPool,
    tenant_id: &str,
    inspection_id: Uuid,
    inspector_id: Option<Uuid>,
    reason: Option<&str>,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<Inspection, QiError> {
    transition_disposition(
        pool,
        wc_pool,
        tenant_id,
        inspection_id,
        "released",
        QualityInspectionEventType::InspectionReleased,
        inspector_id,
        reason,
        correlation_id,
        causation_id,
    )
    .await
}

pub async fn accept_inspection(
    pool: &PgPool,
    wc_pool: &PgPool,
    tenant_id: &str,
    inspection_id: Uuid,
    inspector_id: Option<Uuid>,
    reason: Option<&str>,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<Inspection, QiError> {
    transition_disposition(
        pool,
        wc_pool,
        tenant_id,
        inspection_id,
        "accepted",
        QualityInspectionEventType::InspectionAccepted,
        inspector_id,
        reason,
        correlation_id,
        causation_id,
    )
    .await
}

pub async fn reject_inspection(
    pool: &PgPool,
    wc_pool: &PgPool,
    tenant_id: &str,
    inspection_id: Uuid,
    inspector_id: Option<Uuid>,
    reason: Option<&str>,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<Inspection, QiError> {
    transition_disposition(
        pool,
        wc_pool,
        tenant_id,
        inspection_id,
        "rejected",
        QualityInspectionEventType::InspectionRejected,
        inspector_id,
        reason,
        correlation_id,
        causation_id,
    )
    .await
}

// ============================================================================
// Query: by part revision, by receipt, by work order, by lot
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

pub async fn list_inspections_by_wo(
    pool: &PgPool,
    tenant_id: &str,
    wo_id: Uuid,
    inspection_type: Option<&str>,
) -> Result<Vec<Inspection>, QiError> {
    let rows = if let Some(itype) = inspection_type {
        sqlx::query_as::<_, Inspection>(
            r#"
            SELECT * FROM inspections
            WHERE tenant_id = $1 AND wo_id = $2 AND inspection_type = $3
            ORDER BY created_at DESC
            "#,
        )
        .bind(tenant_id)
        .bind(wo_id)
        .bind(itype)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, Inspection>(
            r#"
            SELECT * FROM inspections
            WHERE tenant_id = $1 AND wo_id = $2
            ORDER BY created_at DESC
            "#,
        )
        .bind(tenant_id)
        .bind(wo_id)
        .fetch_all(pool)
        .await?
    };
    Ok(rows)
}

pub async fn list_inspections_by_lot(
    pool: &PgPool,
    tenant_id: &str,
    lot_id: Uuid,
) -> Result<Vec<Inspection>, QiError> {
    let rows = sqlx::query_as::<_, Inspection>(
        r#"
        SELECT * FROM inspections
        WHERE tenant_id = $1 AND lot_id = $2
        ORDER BY created_at DESC
        "#,
    )
    .bind(tenant_id)
    .bind(lot_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
