use axum::{
    extract::{Query, State},
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::models::ApiError;

// ============================================================================
// AR AGING REPORT (bd-3cb)
// ============================================================================

/// Query parameters for the aging endpoint
#[derive(serde::Deserialize)]
pub struct AgingQuery {
    pub customer_id: Option<i32>,
}

#[utoipa::path(get, path = "/api/ar/aging", tag = "Aging",
    params(("customer_id" = Option<i32>, Query, description = "Filter by customer")),
    responses((status = 200, description = "AR aging report", body = serde_json::Value)),
    security(("bearer" = [])))]
/// GET /api/ar/aging — return pre-computed aging buckets
///
/// Returns the stored projection. Callers must POST /api/ar/aging/refresh
/// first to ensure the projection is current.
pub async fn get_aging(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<AgingQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    match params.customer_id {
        Some(customer_id) => {
            let snapshot = crate::aging::get_aging_for_customer(&db, &app_id, customer_id)
                .await
                .map_err(|e| {
                    tracing::error!(
                        "Failed to fetch aging for customer {}: {:?}",
                        customer_id,
                        e
                    );
                    ApiError::internal("Internal database error")
                })?;

            match snapshot {
                Some(s) => Ok(Json(serde_json::json!({ "aging": [s] }))),
                None => Ok(Json(serde_json::json!({ "aging": [] }))),
            }
        }
        None => {
            let snapshots = crate::aging::get_aging_for_app(&db, &app_id)
                .await
                .map_err(|e| {
                    tracing::error!(error = %e, "Failed to fetch aging for app");
                    ApiError::internal("Internal database error")
                })?;

            Ok(Json(serde_json::json!({ "aging": snapshots })))
        }
    }
}

/// Request body for POST /api/ar/aging/refresh
#[derive(serde::Deserialize)]
pub struct RefreshAgingRequest {
    pub customer_id: i32,
}

#[utoipa::path(post, path = "/api/ar/aging/refresh", tag = "Aging",
    request_body = serde_json::Value,
    responses((status = 200, description = "Updated aging snapshot", body = serde_json::Value)),
    security(("bearer" = [])))]
/// POST /api/ar/aging/refresh — recompute aging for a customer
///
/// Recomputes aging buckets from invoices minus payments and upserts the
/// projection. Returns the updated snapshot. Emits ar.ar_aging_updated
/// into the outbox in the same transaction.
pub async fn refresh_aging_route(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<RefreshAgingRequest>,
) -> Result<Json<crate::aging::AgingSnapshot>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let snapshot = crate::aging::refresh_aging(&db, &app_id, req.customer_id)
        .await
        .map_err(|e| {
            tracing::error!(
                "Failed to refresh aging for customer {}: {:?}",
                req.customer_id,
                e
            );
            ApiError::internal("Internal database error")
        })?;

    Ok(Json(snapshot))
}
