/// HTTP handler: GET /api/ttp/service-agreements
///
/// Lists service agreements (plans) for a tenant, sorted by plan_code then
/// agreement_id for stable, deterministic output.
///
/// Tenant is derived from the JWT `VerifiedClaims`.
///
/// # Query Parameters
///
/// - `status` (optional): filter by status — `active` | `suspended` | `cancelled`
///   Defaults to `active`.
///
/// # Response — 200 OK
///
/// ```json
/// {
///   "tenant_id": "uuid",
///   "items": [
///     {
///       "agreement_id": "uuid",
///       "party_id": "uuid",
///       "plan_code": "starter",
///       "amount_minor": 9900,
///       "currency": "usd",
///       "billing_cycle": "monthly",
///       "status": "active",
///       "effective_from": "2026-01-01"
///     }
///   ],
///   "count": 1
/// }
/// ```
use axum::{
    extract::{Query, State},
    Extension, Json,
};
use chrono::NaiveDate;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

use crate::AppState;

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListQuery {
    /// Filter by status (default: "active"). Pass "all" to see every status.
    #[serde(default = "default_status")]
    pub status: String,
}

fn default_status() -> String {
    "active".to_string()
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ServiceAgreementItem {
    pub agreement_id: Uuid,
    pub party_id: Uuid,
    pub plan_code: String,
    pub amount_minor: i64,
    pub currency: String,
    pub billing_cycle: String,
    pub status: String,
    pub effective_from: NaiveDate,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_to: Option<NaiveDate>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ListServiceAgreementsResponse {
    pub tenant_id: Uuid,
    pub items: Vec<ServiceAgreementItem>,
    pub count: usize,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// GET /api/ttp/service-agreements
#[utoipa::path(
    get, path = "/api/ttp/service-agreements", tag = "Service Agreements",
    params(ListQuery),
    responses(
        (status = 200, description = "Service agreements list", body = ListServiceAgreementsResponse),
        (status = 400, description = "Invalid status filter", body = ApiError),
        (status = 401, description = "Missing or invalid authentication", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn list_service_agreements(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListQuery>,
) -> Result<Json<ListServiceAgreementsResponse>, ApiError> {
    let tenant_id = claims
        .map(|Extension(c)| c.tenant_id)
        .ok_or_else(|| ApiError::unauthorized("Missing or invalid authentication"))?;

    // Validate status value
    let status_filter = query.status.as_str();
    let valid_statuses = ["active", "suspended", "cancelled", "all"];
    if !valid_statuses.contains(&status_filter) {
        return Err(ApiError::bad_request(format!(
            "status must be one of: active, suspended, cancelled, all; got '{}'",
            status_filter
        )));
    }

    let rows = if status_filter == "all" {
        sqlx::query(
            r#"
            SELECT agreement_id, party_id, plan_code, amount_minor, currency,
                   billing_cycle, status, effective_from, effective_to
            FROM ttp_service_agreements
            WHERE tenant_id = $1
            ORDER BY plan_code, agreement_id
            "#,
        )
        .bind(tenant_id)
        .fetch_all(&state.pool)
        .await
    } else {
        sqlx::query(
            r#"
            SELECT agreement_id, party_id, plan_code, amount_minor, currency,
                   billing_cycle, status, effective_from, effective_to
            FROM ttp_service_agreements
            WHERE tenant_id = $1
              AND status = $2
            ORDER BY plan_code, agreement_id
            "#,
        )
        .bind(tenant_id)
        .bind(status_filter)
        .fetch_all(&state.pool)
        .await
    }
    .map_err(|e| {
        tracing::error!("service-agreements list error: {:?}", e);
        ApiError::internal(e.to_string())
    })?;

    let items: Result<Vec<ServiceAgreementItem>, _> = rows
        .iter()
        .map(|row| {
            Ok(ServiceAgreementItem {
                agreement_id: row.try_get("agreement_id")?,
                party_id: row.try_get("party_id")?,
                plan_code: row.try_get("plan_code")?,
                amount_minor: row.try_get("amount_minor")?,
                currency: row.try_get("currency")?,
                billing_cycle: row.try_get("billing_cycle")?,
                status: row.try_get("status")?,
                effective_from: row.try_get("effective_from")?,
                effective_to: row.try_get("effective_to")?,
            })
        })
        .collect();

    let items = items.map_err(|e: sqlx::Error| {
        tracing::error!("service-agreements row mapping error: {:?}", e);
        ApiError::internal(e.to_string())
    })?;

    let count = items.len();
    Ok(Json(ListServiceAgreementsResponse {
        tenant_id,
        items,
        count,
    }))
}
