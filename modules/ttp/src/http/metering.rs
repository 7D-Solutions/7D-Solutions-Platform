/// HTTP handlers for TTP metering endpoints.
///
/// POST /api/metering/events  — idempotent event ingestion
/// GET  /api/metering/trace   — deterministic price trace
use axum::{
    extract::{Query, State},
    Extension, Json,
};
use chrono::{DateTime, Utc};
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::metering::{self, MeteringEventInput, PriceTrace};
use crate::AppState;

// ---------------------------------------------------------------------------
// POST /api/metering/events
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct IngestEventRequest {
    pub events: Vec<EventItem>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct EventItem {
    pub dimension: String,
    pub quantity: i64,
    pub occurred_at: DateTime<Utc>,
    pub idempotency_key: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct IngestEventResponse {
    pub ingested: u32,
    pub duplicates: u32,
    pub results: Vec<IngestResultItem>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct IngestResultItem {
    pub idempotency_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<Uuid>,
    pub was_duplicate: bool,
}

#[utoipa::path(
    post, path = "/api/metering/events", tag = "Metering",
    request_body = IngestEventRequest,
    responses(
        (status = 200, description = "Events ingested", body = IngestEventResponse),
        (status = 400, description = "Validation error", body = ApiError),
        (status = 401, description = "Missing or invalid authentication", body = ApiError),
        (status = 500, description = "Ingestion failed", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn ingest_events(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<IngestEventRequest>,
) -> Result<Json<IngestEventResponse>, ApiError> {
    let tenant_id = claims
        .map(|Extension(c)| c.tenant_id)
        .ok_or_else(|| ApiError::unauthorized("Missing or invalid authentication"))?;

    if req.events.is_empty() {
        return Err(ApiError::bad_request("events array must not be empty"));
    }

    // Validate all events before ingesting any
    for item in &req.events {
        if item.dimension.trim().is_empty() {
            return Err(ApiError::bad_request("dimension must not be empty"));
        }
        if item.quantity <= 0 {
            return Err(ApiError::bad_request("quantity must be positive"));
        }
        if item.idempotency_key.trim().is_empty() {
            return Err(ApiError::bad_request("idempotency_key must not be empty"));
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
            tracing::error!(error = %e, "Metering ingestion error");
            ApiError::internal(e.to_string())
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

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct TraceQuery {
    pub period: String,
}

#[utoipa::path(
    get, path = "/api/metering/trace", tag = "Metering",
    params(TraceQuery),
    responses(
        (status = 200, description = "Price trace for period", body = PriceTrace),
        (status = 400, description = "Invalid period format", body = ApiError),
        (status = 401, description = "Missing or invalid authentication", body = ApiError),
        (status = 500, description = "Trace failed", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_trace(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<TraceQuery>,
) -> Result<Json<PriceTrace>, ApiError> {
    let tenant_id = claims
        .map(|Extension(c)| c.tenant_id)
        .ok_or_else(|| ApiError::unauthorized("Missing or invalid authentication"))?;

    let trace = metering::compute_price_trace(&state.pool, tenant_id, &query.period)
        .await
        .map_err(|e| match &e {
            metering::MeteringError::InvalidPeriod(_) => ApiError::bad_request(e.to_string()),
            _ => {
                tracing::error!(error = %e, "Metering trace error");
                ApiError::internal(e.to_string())
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
        let req: IngestEventRequest = serde_json::from_value(json)
            .expect("IngestEventRequest should deserialize from valid JSON");
        assert_eq!(req.events.len(), 1);
        assert_eq!(req.events[0].dimension, "api_calls");
        assert_eq!(req.events[0].quantity, 42);
    }

    #[test]
    fn trace_query_deserializes() {
        let json = serde_json::json!({
            "period": "2026-02"
        });
        let q: TraceQuery =
            serde_json::from_value(json).expect("TraceQuery should deserialize from valid JSON");
        assert_eq!(q.period, "2026-02");
    }
}
