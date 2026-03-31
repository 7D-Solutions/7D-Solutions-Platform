//! POST /confirm — confirm a gap-free reservation.
//!
//! Flow: Guard → Mutation → Outbox in a single transaction.
//! Idempotent: confirming an already-confirmed number returns success.

use axum::{extract::State, http::StatusCode, Extension, Json};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::tenant::{extract_tenant, with_request_id};
use crate::outbox;

#[derive(Debug, Deserialize, ToSchema)]
pub struct ConfirmRequest {
    pub entity: String,
    pub idempotency_key: String,
}

#[derive(Debug, Serialize, ToSchema)]
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

#[utoipa::path(
    post, path = "/confirm", tag = "Numbering",
    request_body = ConfirmRequest,
    responses(
        (status = 200, description = "Confirmed", body = ConfirmResponse),
        (status = 404, body = ApiError), (status = 409, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn confirm(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(req): Json<ConfirmRequest>,
) -> Result<(StatusCode, Json<ConfirmResponse>), ApiError> {
    let tenant_id = extract_tenant(&claims)
        .map_err(|e| with_request_id(e, &ctx))?;

    if req.entity.is_empty() || req.entity.len() > 100 {
        return Err(with_request_id(
            ApiError::bad_request("entity must be 1-100 characters"),
            &ctx,
        ));
    }

    if req.idempotency_key.is_empty() || req.idempotency_key.len() > 512 {
        return Err(with_request_id(
            ApiError::bad_request("idempotency_key must be 1-512 characters"),
            &ctx,
        ));
    }

    let mut tx = state.pool.begin().await.map_err(|e| {
        tracing::error!("Numbering: confirm begin tx failed: {}", e);
        with_request_id(ApiError::internal("Failed to begin transaction"), &ctx)
    })?;

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
        with_request_id(ApiError::internal("Failed to look up reservation"), &ctx)
    })?;

    let row = match row {
        Some(r) => r,
        None => {
            return Err(with_request_id(
                ApiError::not_found("No allocation found for this idempotency_key"),
                &ctx,
            ));
        }
    };

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

    if row.status != "reserved" {
        return Err(with_request_id(
            ApiError::conflict(format!(
                "Cannot confirm allocation in '{}' state",
                row.status
            )),
            &ctx,
        ));
    }

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
        with_request_id(ApiError::internal("Failed to confirm reservation"), &ctx)
    })?;

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
        with_request_id(ApiError::internal("Failed to enqueue confirmation event"), &ctx)
    })?;

    tx.commit().await.map_err(|e| {
        tracing::error!("Numbering: confirm commit failed: {}", e);
        with_request_id(ApiError::internal("Failed to commit confirmation"), &ctx)
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
