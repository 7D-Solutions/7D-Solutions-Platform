use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{self, SignoffRecordedPayload};
use crate::outbox::enqueue_event_tx;
use platform_http_contracts::ApiError;

use super::{repo, ListSignoffsQuery, RecordSignoffRequest, Signoff, VALID_ENTITY_TYPES};

fn generate_signoff_number() -> String {
    format!("SO-{:06}", fastrand::u32(1..=999999))
}

pub async fn record_signoff(
    pool: &PgPool,
    tenant_id: &str,
    signed_by: Uuid,
    req: RecordSignoffRequest,
) -> Result<Signoff, ApiError> {
    if !VALID_ENTITY_TYPES.contains(&req.entity_type.as_str()) {
        return Err(ApiError::bad_request(format!(
            "Invalid entity_type: {}",
            req.entity_type
        )));
    }
    if req.signature_text.trim().is_empty() {
        return Err(ApiError::bad_request("signature_text is required"));
    }

    let now = Utc::now();
    let signoff = Signoff {
        id: Uuid::new_v4(),
        tenant_id: tenant_id.to_string(),
        entity_type: req.entity_type.clone(),
        entity_id: req.entity_id,
        role: req.role.clone(),
        signoff_number: generate_signoff_number(),
        signed_by,
        signed_at: now,
        signature_text: req.signature_text,
        notes: req.notes,
        created_at: now,
    };

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    sqlx::query(
        r#"INSERT INTO signoffs
           (id, tenant_id, entity_type, entity_id, role, signoff_number, signed_by, signed_at, signature_text, notes, created_at)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
    )
    .bind(signoff.id)
    .bind(&signoff.tenant_id)
    .bind(&signoff.entity_type)
    .bind(signoff.entity_id)
    .bind(&signoff.role)
    .bind(&signoff.signoff_number)
    .bind(signoff.signed_by)
    .bind(signoff.signed_at)
    .bind(&signoff.signature_text)
    .bind(&signoff.notes)
    .bind(signoff.created_at)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let payload = serde_json::to_value(SignoffRecordedPayload {
        tenant_id: signoff.tenant_id.clone(),
        signoff_id: signoff.id,
        signoff_number: signoff.signoff_number.clone(),
        entity_type: signoff.entity_type.clone(),
        entity_id: signoff.entity_id,
        role: signoff.role.clone(),
        signed_by: signoff.signed_by,
        signed_at: signoff.signed_at,
    })
    .map_err(|e| ApiError::internal(e.to_string()))?;

    enqueue_event_tx(
        &mut *tx,
        Uuid::new_v4(),
        events::SIGNOFF_RECORDED,
        "signoff",
        &signoff.id.to_string(),
        &payload,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(signoff)
}

pub async fn list_signoffs(
    pool: &PgPool,
    tenant_id: &str,
    q: ListSignoffsQuery,
) -> Result<Vec<Signoff>, ApiError> {
    repo::list_signoffs(pool, tenant_id, &q)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))
}

pub async fn get_signoff(pool: &PgPool, id: Uuid, tenant_id: &str) -> Result<Signoff, ApiError> {
    repo::fetch_signoff(pool, id, tenant_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Signoff not found"))
}
