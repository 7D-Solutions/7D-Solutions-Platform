/// HTTP handlers for TTP metering endpoints.
///
/// POST /api/metering/events  — idempotent event ingestion
/// GET  /api/metering/trace   — deterministic price trace
use axum::{
    extract::{Query, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::{DateTime, Utc};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::metering::{self, MeteringEventInput, PriceTrace};
use crate::AppState;

// ---------------------------------------------------------------------------
// Shared error body
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    error: String,
    code: String,
}

// ---------------------------------------------------------------------------
// POST /api/metering/events
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct IngestEventRequest {
    pub events: Vec<EventItem>,
}

#[derive(Debug, Deserialize)]
pub struct EventItem {
    pub dimension: String,
    pub quantity: i64,
    pub occurred_at: DateTime<Utc>,
    pub idempotency_key: String,
    #[serde(default)]
    pub source_ref: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct IngestEventResponse {
    pub ingested: u32,
    pub duplicates: u32,
    pub results: Vec<IngestResultItem>,
}

#[derive(Debug, Serialize)]
pub struct IngestResultItem {
    pub idempotency_key: String,
    pub event_id: Option<Uuid>,
    pub was_duplicate: bool,
}

pub async fn ingest_events(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<IngestEventRequest>,
) -> Result<Json<IngestEventResponse>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = claims.map(|Extension(c)| c.tenant_id).ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody {
                error: "Missing or invalid authentication".to_string(),
                code: "unauthorized".to_string(),
            }),
        )
    })?;

    if req.events.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "events array must not be empty".to_string(),
                code: "validation_error".to_string(),
            }),
        ));
    }

    // Validate all events before ingesting any
    for item in &req.events {
        if item.dimension.trim().is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorBody {
                    error: "dimension must not be empty".to_string(),
                    code: "validation_error".to_string(),
                }),
            ));
        }
        if item.quantity <= 0 {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorBody {
                    error: "quantity must be positive".to_string(),
                    code: "validation_error".to_string(),
                }),
            ));
        }
        if item.idempotency_key.trim().is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorBody {
                    error: "idempotency_key must not be empty".to_string(),
                    code: "validation_error".to_string(),
                }),
            ));
        }
    }

    let inputs: Vec<MeteringEventInput> = req
        .events
        .iter()
        .map(|e| MeteringEventInput {
            tenant_id,
            dimension: e.dimension.clone(),
            quantity: e.quantity,
            occurred_at: e.occurred_at,
            idempotency_key: e.idempotency_key.clone(),
            source_ref: e.source_ref.clone(),
        })
        .collect();

    let results = metering::ingest_events(&state.pool, &inputs)
        .await
        .map_err(|e| {
            tracing::error!("Metering ingestion error: {:?}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: e.to_string(),
                    code: "ingestion_failed".to_string(),
                }),
            )
        })?;

    let mut ingested = 0u32;
    let mut duplicates = 0u32;
    let result_items: Vec<IngestResultItem> = results
        .iter()
        .zip(req.events.iter())
        .map(|(r, e)| {
            if r.was_duplicate {
                duplicates += 1;
            } else {
                ingested += 1;
            }
            IngestResultItem {
                idempotency_key: e.idempotency_key.clone(),
                event_id: r.event_id,
                was_duplicate: r.was_duplicate,
            }
        })
        .collect();

    Ok(Json(IngestEventResponse {
        ingested,
        duplicates,
        results: result_items,
    }))
}

// ---------------------------------------------------------------------------
// GET /api/metering/trace
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct TraceQuery {
    pub period: String,
}

pub async fn get_trace(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<TraceQuery>,
) -> Result<Json<PriceTrace>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = claims.map(|Extension(c)| c.tenant_id).ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody {
                error: "Missing or invalid authentication".to_string(),
                code: "unauthorized".to_string(),
            }),
        )
    })?;

    let trace = metering::compute_price_trace(&state.pool, tenant_id, &query.period)
        .await
        .map_err(|e| match &e {
            metering::MeteringError::InvalidPeriod(_) => (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody {
                    error: e.to_string(),
                    code: "validation_error".to_string(),
                }),
            ),
            _ => {
                tracing::error!("Metering trace error: {:?}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorBody {
                        error: e.to_string(),
                        code: "trace_failed".to_string(),
                    }),
                )
            }
        })?;

    Ok(Json(trace))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingest_request_deserializes() {
        let json = serde_json::json!({
            "events": [{
                "dimension": "api_calls",
                "quantity": 42,
                "occurred_at": "2026-02-15T10:00:00Z",
                "idempotency_key": "evt-001"
            }]
        });
        let req: IngestEventRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.events.len(), 1);
        assert_eq!(req.events[0].dimension, "api_calls");
        assert_eq!(req.events[0].quantity, 42);
    }

    #[test]
    fn trace_query_deserializes() {
        let json = serde_json::json!({
            "period": "2026-02"
        });
        let q: TraceQuery = serde_json::from_value(json).unwrap();
        assert_eq!(q.period, "2026-02");
    }
}
