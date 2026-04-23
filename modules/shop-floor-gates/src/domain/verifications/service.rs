use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{self, VerificationCompletedPayload, VerificationOperatorConfirmedPayload};
use crate::outbox::enqueue_event_tx;
use platform_http_contracts::ApiError;

use super::{
    repo, CreateVerificationRequest, ListVerificationsQuery, OperationStartVerification,
    OperatorConfirmRequest, SkipVerificationRequest, VerifyRequest,
};

pub async fn create_verification(
    pool: &PgPool,
    tenant_id: &str,
    operator_id: Uuid,
    req: CreateVerificationRequest,
) -> Result<OperationStartVerification, ApiError> {
    // Enforce uniqueness at the application layer (DB has a unique constraint too)
    let existing = repo::fetch_verification_for_operation(
        pool,
        req.work_order_id,
        req.operation_id,
        tenant_id,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if existing.is_some() {
        return Err(ApiError::conflict(
            "Verification already exists for this operation",
        ));
    }

    let now = Utc::now();
    let v = OperationStartVerification {
        id: Uuid::new_v4(),
        tenant_id: tenant_id.to_string(),
        work_order_id: req.work_order_id,
        operation_id: req.operation_id,
        status: "pending".to_string(),
        drawing_verified: false,
        material_verified: false,
        instruction_verified: false,
        operator_id,
        operator_confirmed_at: None,
        verifier_id: None,
        verified_at: None,
        skipped_by: None,
        skipped_at: None,
        skip_reason: None,
        notes: req.notes,
        created_at: now,
        updated_at: now,
    };

    repo::insert_verification(pool, &v)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(v)
}

pub async fn operator_confirm(
    pool: &PgPool,
    tenant_id: &str,
    verification_id: Uuid,
    operator_id: Uuid,
    req: OperatorConfirmRequest,
) -> Result<OperationStartVerification, ApiError> {
    let v = repo::fetch_verification(pool, verification_id, tenant_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Verification not found"))?;

    if v.status != "pending" {
        return Err(ApiError::bad_request(format!(
            "Verification is already {}",
            v.status
        )));
    }
    if v.operator_id != operator_id {
        return Err(ApiError::forbidden(
            "Only the assigned operator can confirm",
        ));
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let updated = sqlx::query_as::<_, OperationStartVerification>(
        r#"UPDATE operation_start_verifications
           SET drawing_verified = $3, material_verified = $4, instruction_verified = $5,
               operator_confirmed_at = NOW(), updated_at = NOW()
           WHERE id = $1 AND tenant_id = $2 AND status = 'pending'
           RETURNING *"#,
    )
    .bind(verification_id)
    .bind(tenant_id)
    .bind(req.drawing_verified)
    .bind(req.material_verified)
    .bind(req.instruction_verified)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::conflict("Verification modified concurrently"))?;

    let payload = serde_json::to_value(VerificationOperatorConfirmedPayload {
        tenant_id: updated.tenant_id.clone(),
        verification_id: updated.id,
        work_order_id: updated.work_order_id,
        operation_id: updated.operation_id,
        operator_id,
        confirmed_at: updated.operator_confirmed_at.unwrap_or_else(Utc::now),
    })
    .map_err(|e| ApiError::internal(e.to_string()))?;

    enqueue_event_tx(
        &mut *tx,
        Uuid::new_v4(),
        events::VERIFICATION_OPERATOR_CONFIRMED,
        "operation_start_verification",
        &updated.id.to_string(),
        &payload,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(updated)
}

pub async fn verify(
    pool: &PgPool,
    tenant_id: &str,
    verification_id: Uuid,
    verifier_id: Uuid,
    _req: VerifyRequest,
) -> Result<OperationStartVerification, ApiError> {
    let v = repo::fetch_verification(pool, verification_id, tenant_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Verification not found"))?;

    if v.status != "pending" {
        return Err(ApiError::bad_request(format!(
            "Verification is already {}",
            v.status
        )));
    }
    // Two-step invariant: operator must have confirmed AND all checkboxes must be true
    if v.operator_confirmed_at.is_none() {
        return Err(ApiError::bad_request(
            "Operator must confirm before verifier can verify",
        ));
    }
    if !v.drawing_verified || !v.material_verified || !v.instruction_verified {
        return Err(ApiError::bad_request(
            "All verification checkboxes must be set before completing verification",
        ));
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let updated = sqlx::query_as::<_, OperationStartVerification>(
        r#"UPDATE operation_start_verifications
           SET status = 'verified', verifier_id = $3, verified_at = NOW(), updated_at = NOW()
           WHERE id = $1 AND tenant_id = $2 AND status = 'pending'
           RETURNING *"#,
    )
    .bind(verification_id)
    .bind(tenant_id)
    .bind(verifier_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::conflict("Verification modified concurrently"))?;

    let payload = serde_json::to_value(VerificationCompletedPayload {
        tenant_id: updated.tenant_id.clone(),
        verification_id: updated.id,
        work_order_id: updated.work_order_id,
        operation_id: updated.operation_id,
        verifier_id,
        verified_at: updated.verified_at.unwrap_or_else(Utc::now),
    })
    .map_err(|e| ApiError::internal(e.to_string()))?;

    enqueue_event_tx(
        &mut *tx,
        Uuid::new_v4(),
        events::VERIFICATION_COMPLETED,
        "operation_start_verification",
        &updated.id.to_string(),
        &payload,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(updated)
}

pub async fn skip_verification(
    pool: &PgPool,
    tenant_id: &str,
    verification_id: Uuid,
    skipped_by: Uuid,
    req: SkipVerificationRequest,
) -> Result<OperationStartVerification, ApiError> {
    let v = repo::fetch_verification(pool, verification_id, tenant_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Verification not found"))?;

    if v.status != "pending" {
        return Err(ApiError::bad_request(format!(
            "Verification is already {}",
            v.status
        )));
    }

    let updated = sqlx::query_as::<_, OperationStartVerification>(
        r#"UPDATE operation_start_verifications
           SET status = 'skipped', skipped_by = $3, skipped_at = NOW(),
               skip_reason = $4, updated_at = NOW()
           WHERE id = $1 AND tenant_id = $2 AND status = 'pending'
           RETURNING *"#,
    )
    .bind(verification_id)
    .bind(tenant_id)
    .bind(skipped_by)
    .bind(&req.skip_reason)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::conflict("Verification modified concurrently"))?;

    Ok(updated)
}

pub async fn list_verifications(
    pool: &PgPool,
    tenant_id: &str,
    q: ListVerificationsQuery,
) -> Result<Vec<OperationStartVerification>, ApiError> {
    repo::list_verifications(pool, tenant_id, &q)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))
}

pub async fn get_verification(
    pool: &PgPool,
    id: Uuid,
    tenant_id: &str,
) -> Result<OperationStartVerification, ApiError> {
    repo::fetch_verification(pool, id, tenant_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Verification not found"))
}
