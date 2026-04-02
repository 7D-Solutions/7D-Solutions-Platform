//! POST /allocate — atomic, idempotent number allocation.
//!
//! Flow: Guard → Mutation → Outbox in a single transaction.
//! Concurrency safety via SELECT FOR UPDATE on the sequences row.
//!
//! Supports two modes:
//! - **Standard** (default): Numbers are immediately confirmed.
//! - **Gap-free**: Numbers are reserved and must be confirmed via POST /confirm.
//!   Expired reservations are recycled before advancing the counter.

use axum::{extract::State, http::StatusCode, Extension, Json};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use platform_sdk::extract_tenant;
use super::tenant::with_request_id;
use crate::{format, outbox, policy};

#[derive(Debug, Deserialize, ToSchema)]
pub struct AllocateRequest {
    pub entity: String,
    pub idempotency_key: String,
    /// Enable gap-free mode for this sequence.  Only honoured on the first
    /// allocation that creates the sequence row; ignored for existing sequences.
    #[serde(default)]
    pub gap_free: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AllocateResponse {
    pub tenant_id: String,
    pub entity: String,
    pub number_value: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formatted_number: Option<String>,
    pub idempotency_key: String,
    pub replay: bool,
    /// "confirmed" for standard allocations, "reserved" for gap-free.
    pub status: String,
    /// Present only for gap-free reservations — ISO 8601 expiry timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

// ErrorResponse removed — using platform_http_contracts::ApiError instead

#[derive(Debug, Serialize)]
struct NumberAllocatedPayload {
    pub tenant_id: String,
    pub entity: String,
    pub number_value: i64,
    pub idempotency_key: String,
    pub status: String,
}

#[utoipa::path(
    post, path = "/allocate", tag = "Numbering",
    request_body = AllocateRequest,
    responses(
        (status = 201, description = "Number allocated", body = AllocateResponse),
        (status = 200, description = "Idempotent replay", body = AllocateResponse),
        (status = 400, body = ApiError), (status = 401, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn allocate(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(req): Json<AllocateRequest>,
) -> Result<(StatusCode, Json<AllocateResponse>), ApiError> {
    let tenant_id: Uuid = extract_tenant(&claims)
        .map_err(|e| with_request_id(e, &ctx))?
        .parse().expect("tenant_id is a valid UUID");

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

    // ── Idempotency check ──────────────────────────────────────────────
    let existing = sqlx::query_as::<_, IssuedRowFull>(
        "SELECT number_value, status, expires_at \
         FROM issued_numbers WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant_id)
    .bind(&req.idempotency_key)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("Numbering: idempotency check failed: {}", e);
        with_request_id(ApiError::internal("Failed to check idempotency key"), &ctx)
    })?;

    if let Some(row) = existing {
        state
            .metrics
            .replays_total
            .with_label_values(&[&req.entity])
            .inc();
        let formatted =
            format_if_policy(&state.pool, tenant_id, &req.entity, row.number_value).await;
        return Ok((
            StatusCode::OK,
            Json(AllocateResponse {
                tenant_id: tenant_id.to_string(),
                entity: req.entity,
                number_value: row.number_value,
                formatted_number: formatted,
                idempotency_key: req.idempotency_key,
                replay: true,
                status: row.status,
                expires_at: row.expires_at.map(|t| t.to_rfc3339()),
            }),
        ));
    }

    // ── Begin transaction: Guard → Mutation → Outbox ───────────────────
    let mut tx = state.pool.begin().await.map_err(|e| {
        tracing::error!("Numbering: failed to begin transaction: {}", e);
        with_request_id(ApiError::internal("Failed to begin transaction"), &ctx)
    })?;

    let gap_free_requested = req.gap_free.unwrap_or(false);
    let alloc = allocate_next_value(&mut tx, tenant_id, &req.entity, gap_free_requested)
        .await
        .map_err(|e| {
            tracing::error!("Numbering: allocation failed: {}", e);
            with_request_id(ApiError::internal("Failed to allocate number"), &ctx)
        })?;

    let (issued_status, expires_at) = if alloc.gap_free {
        let ttl = chrono::Duration::seconds(alloc.reservation_ttl_secs as i64);
        let exp = chrono::Utc::now() + ttl;
        ("reserved".to_string(), Some(exp))
    } else {
        ("confirmed".to_string(), None)
    };

    // ── Mutation: record the issued number ─────────────────────────────
    // If this number is a recycled reservation, update the existing row
    // instead of inserting a new one.
    if alloc.recycled {
        sqlx::query(
            "UPDATE issued_numbers \
             SET idempotency_key = $1, status = $2, expires_at = $3 \
             WHERE tenant_id = $4 AND entity = $5 AND number_value = $6",
        )
        .bind(&req.idempotency_key)
        .bind(&issued_status)
        .bind(expires_at)
        .bind(tenant_id)
        .bind(&req.entity)
        .bind(alloc.value)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!("Numbering: failed to recycle issued number: {}", e);
            with_request_id(ApiError::internal("Failed to recycle issued number"), &ctx)
        })?;
    } else {
        sqlx::query(
            "INSERT INTO issued_numbers \
             (tenant_id, entity, number_value, idempotency_key, status, expires_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(tenant_id)
        .bind(&req.entity)
        .bind(alloc.value)
        .bind(&req.idempotency_key)
        .bind(&issued_status)
        .bind(expires_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!("Numbering: failed to insert issued number: {}", e);
            with_request_id(ApiError::internal("Failed to record issued number"), &ctx)
        })?;
    }

    // ── Outbox: enqueue event ──────────────────────────────────────────
    let event_payload = NumberAllocatedPayload {
        tenant_id: tenant_id.to_string(),
        entity: req.entity.clone(),
        number_value: alloc.value,
        idempotency_key: req.idempotency_key.clone(),
        status: issued_status.clone(),
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
        with_request_id(ApiError::internal("Failed to enqueue allocation event"), &ctx)
    })?;

    // ── Commit the atomic unit ─────────────────────────────────────────
    tx.commit().await.map_err(|e| {
        tracing::error!("Numbering: failed to commit transaction: {}", e);
        with_request_id(ApiError::internal("Failed to commit allocation"), &ctx)
    })?;

    state
        .metrics
        .allocations_total
        .with_label_values(&[&req.entity])
        .inc();

    let formatted = format_if_policy(&state.pool, tenant_id, &req.entity, alloc.value).await;

    tracing::info!(
        tenant_id = %tenant_id,
        entity = %req.entity,
        number = alloc.value,
        formatted = ?formatted,
        status = %issued_status,
        recycled = alloc.recycled,
        "Numbering: allocated"
    );

    Ok((
        StatusCode::CREATED,
        Json(AllocateResponse {
            tenant_id: tenant_id.to_string(),
            entity: req.entity,
            number_value: alloc.value,
            formatted_number: formatted,
            idempotency_key: req.idempotency_key,
            replay: false,
            status: issued_status,
            expires_at: expires_at.map(|t| t.to_rfc3339()),
        }),
    ))
}

// ── Internal types ─────────────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
struct IssuedRowFull {
    number_value: i64,
    status: String,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, sqlx::FromRow)]
struct SequenceRow {
    current_value: i64,
    gap_free: bool,
    reservation_ttl_secs: i32,
}

#[derive(Debug, sqlx::FromRow)]
struct RecyclableRow {
    number_value: i64,
}

struct Allocation {
    value: i64,
    gap_free: bool,
    reservation_ttl_secs: i32,
    recycled: bool,
}

// db_error removed — using ApiError::internal() with with_request_id instead

/// Look up a formatting policy and apply it. Returns None if no policy exists.
///
/// This is intentionally outside the allocation transaction — formatting is
/// a read-only decoration that must not extend lock duration.
async fn format_if_policy(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    entity: &str,
    number: i64,
) -> Option<String> {
    match policy::get_policy(pool, tenant_id, entity).await {
        Ok(Some(row)) => {
            let fp = format::FormatPolicy {
                pattern: row.pattern,
                prefix: row.prefix,
                padding: row.padding as u32,
            };
            let today = chrono::Utc::now().date_naive();
            Some(format::format_number(&fp, number, today))
        }
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(
                tenant_id = %tenant_id,
                entity = %entity,
                "Numbering: failed to read policy for formatting: {}",
                e
            );
            None
        }
    }
}

/// Allocate the next value for a tenant+entity sequence.
///
/// For **standard** sequences: uses INSERT ON CONFLICT DO UPDATE to atomically
/// advance the counter.
///
/// For **gap-free** sequences: first checks for recyclable expired reservations.
/// If one is found, reuses that number.  Otherwise advances the counter.
///
/// The `gap_free_requested` flag is only used when *creating* a new sequence.
/// Existing sequences always follow their stored `gap_free` setting.
async fn allocate_next_value(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
    entity: &str,
    gap_free_requested: bool,
) -> Result<Allocation, sqlx::Error> {
    // Try to fetch + lock the existing sequence.
    let existing_seq = sqlx::query_as::<_, SequenceRow>(
        "SELECT current_value, gap_free, reservation_ttl_secs \
         FROM sequences WHERE tenant_id = $1 AND entity = $2 FOR UPDATE",
    )
    .bind(tenant_id)
    .bind(entity)
    .fetch_optional(&mut **tx)
    .await?;

    match existing_seq {
        Some(seq) => {
            // ── Gap-free path: try to recycle an expired reservation ────
            if seq.gap_free {
                if let Some(recycled) = try_recycle(tx, tenant_id, entity).await? {
                    return Ok(Allocation {
                        value: recycled,
                        gap_free: true,
                        reservation_ttl_secs: seq.reservation_ttl_secs,
                        recycled: true,
                    });
                }
            }

            // Advance counter
            let row = sqlx::query_as::<_, CounterRow>(
                "UPDATE sequences SET current_value = current_value + 1, updated_at = NOW() \
                 WHERE tenant_id = $1 AND entity = $2 RETURNING current_value",
            )
            .bind(tenant_id)
            .bind(entity)
            .fetch_one(&mut **tx)
            .await?;

            Ok(Allocation {
                value: row.current_value,
                gap_free: seq.gap_free,
                reservation_ttl_secs: seq.reservation_ttl_secs,
                recycled: false,
            })
        }
        None => {
            // First allocation — create the sequence row.
            let row = sqlx::query_as::<_, SequenceRow>(
                "INSERT INTO sequences (tenant_id, entity, current_value, gap_free) \
                 VALUES ($1, $2, 1, $3) \
                 ON CONFLICT (tenant_id, entity) \
                 DO UPDATE SET current_value = sequences.current_value + 1, updated_at = NOW() \
                 RETURNING current_value, gap_free, reservation_ttl_secs",
            )
            .bind(tenant_id)
            .bind(entity)
            .bind(gap_free_requested)
            .fetch_one(&mut **tx)
            .await?;

            Ok(Allocation {
                value: row.current_value,
                gap_free: row.gap_free,
                reservation_ttl_secs: row.reservation_ttl_secs,
                recycled: false,
            })
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct CounterRow {
    current_value: i64,
}

/// Find and lock the smallest expired reservation for recycling.
///
/// Uses `FOR UPDATE SKIP LOCKED` so concurrent recyclers never fight over
/// the same row.
async fn try_recycle(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
    entity: &str,
) -> Result<Option<i64>, sqlx::Error> {
    let row = sqlx::query_as::<_, RecyclableRow>(
        "SELECT number_value FROM issued_numbers \
         WHERE tenant_id = $1 AND entity = $2 \
           AND status = 'reserved' AND expires_at < NOW() \
         ORDER BY number_value ASC LIMIT 1 \
         FOR UPDATE SKIP LOCKED",
    )
    .bind(tenant_id)
    .bind(entity)
    .fetch_optional(&mut **tx)
    .await?;

    Ok(row.map(|r| r.number_value))
}
