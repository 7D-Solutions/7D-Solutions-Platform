//! POST /allocate — atomic, idempotent number allocation.
//!
//! Flow: Guard → Mutation → Outbox in a single transaction.
//! Concurrency safety via SELECT FOR UPDATE on the sequences row.

use axum::{extract::State, http::StatusCode, Extension, Json};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::outbox;

#[derive(Debug, Deserialize)]
pub struct AllocateRequest {
    pub entity: String,
    pub idempotency_key: String,
}

#[derive(Debug, Serialize)]
pub struct AllocateResponse {
    pub tenant_id: String,
    pub entity: String,
    pub number_value: i64,
    pub idempotency_key: String,
    pub replay: bool,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
struct NumberAllocatedPayload {
    pub tenant_id: String,
    pub entity: String,
    pub number_value: i64,
    pub idempotency_key: String,
}

/// POST /allocate
///
/// Allocates the next number for a given tenant + entity.
/// Idempotent: duplicate idempotency_key returns the previously allocated number.
/// Atomic: sequence increment + issued record + outbox event in one transaction.
pub async fn allocate(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<AllocateRequest>,
) -> Result<(StatusCode, Json<AllocateResponse>), (StatusCode, Json<ErrorResponse>)> {
    // Extract tenant_id from JWT claims — never trust client-supplied values
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

    // Validate inputs
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

    // Check idempotency: has this key already been used?
    let existing = sqlx::query_as::<_, IssuedRow>(
        "SELECT number_value FROM issued_numbers WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant_id)
    .bind(&req.idempotency_key)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("Numbering: idempotency check failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "database_error".to_string(),
                message: "Failed to check idempotency key".to_string(),
            }),
        )
    })?;

    if let Some(row) = existing {
        state
            .metrics
            .replays_total
            .with_label_values(&[&req.entity])
            .inc();
        return Ok((
            StatusCode::OK,
            Json(AllocateResponse {
                tenant_id: tenant_id.to_string(),
                entity: req.entity,
                number_value: row.number_value,
                idempotency_key: req.idempotency_key,
                replay: true,
            }),
        ));
    }

    // Begin transaction: Guard → Mutation → Outbox
    let mut tx = state.pool.begin().await.map_err(|e| {
        tracing::error!("Numbering: failed to begin transaction: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "database_error".to_string(),
                message: "Failed to begin transaction".to_string(),
            }),
        )
    })?;

    // Guard: lock the sequence row (or create it if first allocation)
    let next_value = allocate_next_value(&mut tx, tenant_id, &req.entity)
        .await
        .map_err(|e| {
            tracing::error!("Numbering: allocation failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "allocation_error".to_string(),
                    message: "Failed to allocate number".to_string(),
                }),
            )
        })?;

    // Mutation: record the issued number
    sqlx::query(
        "INSERT INTO issued_numbers (tenant_id, entity, number_value, idempotency_key) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(tenant_id)
    .bind(&req.entity)
    .bind(next_value)
    .bind(&req.idempotency_key)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Numbering: failed to insert issued number: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "database_error".to_string(),
                message: "Failed to record issued number".to_string(),
            }),
        )
    })?;

    // Outbox: enqueue event
    let event_payload = NumberAllocatedPayload {
        tenant_id: tenant_id.to_string(),
        entity: req.entity.clone(),
        number_value: next_value,
        idempotency_key: req.idempotency_key.clone(),
    };

    let event_id = Uuid::new_v4();
    outbox::enqueue_event_tx(
        &mut tx,
        event_id,
        "numbering.events.number.allocated",
        "number",
        &format!("{}:{}", tenant_id, req.entity),
        &event_payload,
    )
    .await
    .map_err(|e| {
        tracing::error!("Numbering: failed to enqueue event: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "database_error".to_string(),
                message: "Failed to enqueue allocation event".to_string(),
            }),
        )
    })?;

    // Commit the atomic unit
    tx.commit().await.map_err(|e| {
        tracing::error!("Numbering: failed to commit transaction: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "database_error".to_string(),
                message: "Failed to commit allocation".to_string(),
            }),
        )
    })?;

    state
        .metrics
        .allocations_total
        .with_label_values(&[&req.entity])
        .inc();

    tracing::info!(
        tenant_id = %tenant_id,
        entity = %req.entity,
        number = next_value,
        "Numbering: allocated"
    );

    Ok((
        StatusCode::CREATED,
        Json(AllocateResponse {
            tenant_id: tenant_id.to_string(),
            entity: req.entity,
            number_value: next_value,
            idempotency_key: req.idempotency_key,
            replay: false,
        }),
    ))
}

#[derive(Debug, sqlx::FromRow)]
struct IssuedRow {
    number_value: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct SequenceRow {
    current_value: i64,
}

/// Atomically allocate the next value for a tenant+entity sequence.
///
/// Uses INSERT ... ON CONFLICT DO UPDATE to handle the race where multiple
/// concurrent transactions try to create the initial sequence row simultaneously.
/// The ON CONFLICT clause serialises via the row lock that the UPSERT acquires.
async fn allocate_next_value(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
    entity: &str,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query_as::<_, SequenceRow>(
        r#"
        INSERT INTO sequences (tenant_id, entity, current_value)
        VALUES ($1, $2, 1)
        ON CONFLICT (tenant_id, entity)
        DO UPDATE SET current_value = sequences.current_value + 1,
                      updated_at = NOW()
        RETURNING current_value
        "#,
    )
    .bind(tenant_id)
    .bind(entity)
    .fetch_one(&mut **tx)
    .await?;

    Ok(row.current_value)
}
