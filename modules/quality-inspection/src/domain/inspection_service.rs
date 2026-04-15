use platform_sdk::PlatformClient;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::inspection_repo as repo;
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
    validate_result(result_val)?;

    let mut tx = pool.begin().await?;

    let inspection = repo::insert_receiving_inspection(&mut tx, tenant_id, req, result_val).await?;

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
    validate_result(result_val)?;

    let mut tx = pool.begin().await?;

    let inspection =
        repo::insert_in_process_inspection(&mut tx, tenant_id, req, result_val).await?;

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
    validate_result(result_val)?;

    let mut tx = pool.begin().await?;

    let inspection = repo::insert_final_inspection(&mut tx, tenant_id, req, result_val).await?;

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
    repo::get_by_id(pool, tenant_id, inspection_id)
        .await?
        .ok_or_else(|| QiError::NotFound("Inspection not found".into()))
}

// ============================================================================
// Disposition state machine
// ============================================================================

fn validate_result(result_val: &str) -> Result<(), QiError> {
    match result_val {
        "pending" | "pass" | "fail" | "conditional" => Ok(()),
        other => Err(QiError::Validation(format!(
            "Invalid result '{}', expected one of: pending, pass, fail, conditional",
            other
        ))),
    }
}

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
    wc_client: &PlatformClient,
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
    crate::domain::wc_client::verify_inspector_authorized(wc_client, tenant_id, iid).await?;

    let inspection = get_inspection(pool, tenant_id, inspection_id).await?;
    validate_disposition_transition(&inspection.disposition, target)?;

    let mut tx = pool.begin().await?;

    let updated = repo::update_disposition(&mut tx, tenant_id, inspection_id, target).await?;

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
    wc_client: &PlatformClient,
    tenant_id: &str,
    inspection_id: Uuid,
    inspector_id: Option<Uuid>,
    reason: Option<&str>,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<Inspection, QiError> {
    transition_disposition(
        pool,
        wc_client,
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
    wc_client: &PlatformClient,
    tenant_id: &str,
    inspection_id: Uuid,
    inspector_id: Option<Uuid>,
    reason: Option<&str>,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<Inspection, QiError> {
    transition_disposition(
        pool,
        wc_client,
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
    wc_client: &PlatformClient,
    tenant_id: &str,
    inspection_id: Uuid,
    inspector_id: Option<Uuid>,
    reason: Option<&str>,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<Inspection, QiError> {
    transition_disposition(
        pool,
        wc_client,
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
    wc_client: &PlatformClient,
    tenant_id: &str,
    inspection_id: Uuid,
    inspector_id: Option<Uuid>,
    reason: Option<&str>,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<Inspection, QiError> {
    transition_disposition(
        pool,
        wc_client,
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
    repo::list_by_part_rev(pool, tenant_id, part_id, part_revision).await
}

pub async fn list_inspections_by_part_rev_paginated(
    pool: &PgPool,
    tenant_id: &str,
    part_id: Uuid,
    part_revision: Option<&str>,
    page_size: i64,
    offset: i64,
) -> Result<(Vec<Inspection>, i64), QiError> {
    repo::list_by_part_rev_paginated(pool, tenant_id, part_id, part_revision, page_size, offset)
        .await
}

pub async fn list_inspections_by_receipt(
    pool: &PgPool,
    tenant_id: &str,
    receipt_id: Uuid,
) -> Result<Vec<Inspection>, QiError> {
    repo::list_by_receipt(pool, tenant_id, receipt_id).await
}

pub async fn list_inspections_by_receipt_paginated(
    pool: &PgPool,
    tenant_id: &str,
    receipt_id: Uuid,
    page_size: i64,
    offset: i64,
) -> Result<(Vec<Inspection>, i64), QiError> {
    repo::list_by_receipt_paginated(pool, tenant_id, receipt_id, page_size, offset).await
}

pub async fn list_inspections_by_wo(
    pool: &PgPool,
    tenant_id: &str,
    wo_id: Uuid,
    inspection_type: Option<&str>,
) -> Result<Vec<Inspection>, QiError> {
    repo::list_by_wo(pool, tenant_id, wo_id, inspection_type).await
}

pub async fn list_inspections_by_wo_paginated(
    pool: &PgPool,
    tenant_id: &str,
    wo_id: Uuid,
    inspection_type: Option<&str>,
    page_size: i64,
    offset: i64,
) -> Result<(Vec<Inspection>, i64), QiError> {
    repo::list_by_wo_paginated(pool, tenant_id, wo_id, inspection_type, page_size, offset).await
}

pub async fn list_inspections_by_lot(
    pool: &PgPool,
    tenant_id: &str,
    lot_id: Uuid,
) -> Result<Vec<Inspection>, QiError> {
    repo::list_by_lot(pool, tenant_id, lot_id).await
}

pub async fn list_inspections_by_lot_paginated(
    pool: &PgPool,
    tenant_id: &str,
    lot_id: Uuid,
    page_size: i64,
    offset: i64,
) -> Result<(Vec<Inspection>, i64), QiError> {
    repo::list_by_lot_paginated(pool, tenant_id, lot_id, page_size, offset).await
}
