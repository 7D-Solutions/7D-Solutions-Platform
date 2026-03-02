//! POST /confirm — confirm a gap-free reservation.
//!
//! Flow: Guard → Mutation → Outbox in a single transaction.
//! Idempotent: confirming an already-confirmed number returns success.

use axum::{extract::State, http::StatusCode, Extension, Json};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::outbox;

use super::allocate::ErrorResponse;

#[derive(Debug, Deserialize)]
pub struct ConfirmRequest {
    pub entity: String,
    pub idempotency_key: String,
}

#[derive(Debug, Serialize)]
pub struct ConfirmResponse {
    pub tenant_id: String,
    pub entity: String,
    pub number_value: i64,
    pub idempotency_key: String,
    pub status: String,
    pub replay: bool,
}

#[derive(Debug, Serialize)]
struct NumberConfirmedPayload {
    pub tenant_id: String,
    pub entity: String,
    pub number_value: i64,
    pub idempotency_key: String,
}

#[derive(Debug, sqlx::FromRow)]
struct IssuedRow {
    number_value: i64,
    status: String,
}

/// POST /confirm
///
/// Confirms a previously reserved number, making it permanent.
/// Idempotent: confirming an already-confirmed reservation returns 200.
/// Returns 404 if no reservation exists for the given idempotency_key.
pub async fn confirm(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<ConfirmRequest>,
) -> Result<(StatusCode, Json<ConfirmResponse>), (StatusCode, Json<ErrorResponse>)> {
    let tenant_id = match &claims {
        Some(Extension(c)) => c.tenant_id,
        None => {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    error: "unauthorized".to_string(),
                    message: "Missing or invalid authentication".to_string(),
                }),
            ));
        }
    };

    if req.entity.is_empty() || req.entity.len() > 100 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "invalid_entity".to_string(),
                message: "entity must be 1-100 characters".to_string(),
            }),
        ));
    }

    if req.idempotency_key.is_empty() || req.idempotency_key.len() > 512 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "invalid_idempotency_key".to_string(),
                message: "idempotency_key must be 1-512 characters".to_string(),
            }),
        ));
    }

    // ── Begin transaction: Guard → Mutation → Outbox ───────────────────
    let mut tx = state.pool.begin().await.map_err(|e| {
        tracing::error!("Numbering: confirm begin tx failed: {}", e);
        db_error("Failed to begin transaction")
    })?;

    // Guard: lock the issued row
    let row = sqlx::query_as::<_, IssuedRow>(
        "SELECT number_value, status FROM issued_numbers \
         WHERE tenant_id = $1 AND idempotency_key = $2 \
         FOR UPDATE",
    )
    .bind(tenant_id)
    .bind(&req.idempotency_key)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Numbering: confirm lookup failed: {}", e);
        db_error("Failed to look up reservation")
    })?;

    let row = match row {
        Some(r) => r,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "not_found".to_string(),
                    message: "No allocation found for this idempotency_key".to_string(),
                }),
            ));
        }
    };

    // Already confirmed — idempotent replay
    if row.status == "confirmed" {
        return Ok((
            StatusCode::OK,
            Json(ConfirmResponse {
                tenant_id: tenant_id.to_string(),
                entity: req.entity,
                number_value: row.number_value,
                idempotency_key: req.idempotency_key,
                status: "confirmed".to_string(),
                replay: true,
            }),
        ));
    }

    // Reservation was recycled (status changed or idem_key reassigned).
    // This shouldn't happen since we looked up by idem_key, but guard anyway.
    if row.status != "reserved" {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "invalid_state".to_string(),
                message: format!("Cannot confirm allocation in '{}' state", row.status),
            }),
        ));
    }

    // ── Mutation: transition reserved → confirmed ──────────────────────
    sqlx::query(
        "UPDATE issued_numbers SET status = 'confirmed', expires_at = NULL \
         WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant_id)
    .bind(&req.idempotency_key)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Numbering: confirm update failed: {}", e);
        db_error("Failed to confirm reservation")
    })?;

    // ── Outbox: enqueue confirmation event ─────────────────────────────
    let event_payload = NumberConfirmedPayload {
        tenant_id: tenant_id.to_string(),
        entity: req.entity.clone(),
        number_value: row.number_value,
        idempotency_key: req.idempotency_key.clone(),
    };

    let event_id = Uuid::new_v4();
    outbox::enqueue_event_tx(
        &mut tx,
        event_id,
        "numbering.events.number.confirmed",
        "number",
        &format!("{}:{}", tenant_id, req.entity),
        &event_payload,
    )
    .await
    .map_err(|e| {
        tracing::error!("Numbering: confirm outbox failed: {}", e);
        db_error("Failed to enqueue confirmation event")
    })?;

    tx.commit().await.map_err(|e| {
        tracing::error!("Numbering: confirm commit failed: {}", e);
        db_error("Failed to commit confirmation")
    })?;

    tracing::info!(
        tenant_id = %tenant_id,
        entity = %req.entity,
        number = row.number_value,
        "Numbering: confirmed"
    );

    Ok((
        StatusCode::OK,
        Json(ConfirmResponse {
            tenant_id: tenant_id.to_string(),
            entity: req.entity,
            number_value: row.number_value,
            idempotency_key: req.idempotency_key,
            status: "confirmed".to_string(),
            replay: false,
        }),
    ))
}

fn db_error(msg: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "database_error".to_string(),
            message: msg.to_string(),
        }),
    )
}
