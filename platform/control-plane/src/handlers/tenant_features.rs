/// GET /api/features?tenant_id={uuid}
///
/// Returns all feature flags visible to the authenticated tenant.
/// Requires a valid JWT whose tenant_id claim matches the query parameter.
use axum::{
    extract::{Extension, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use crate::models::ErrorBody;
use crate::state::AppState;
use security::VerifiedClaims;

#[derive(Debug, Deserialize)]
pub struct TenantFeaturesQuery {
    pub tenant_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TenantFeaturesResponse {
    pub tenant_id: String,
    pub flags: HashMap<String, bool>,
}

pub async fn tenant_features(
    State(state): State<Arc<AppState>>,
    claims_ext: Option<Extension<VerifiedClaims>>,
    Query(q): Query<TenantFeaturesQuery>,
) -> Result<Json<TenantFeaturesResponse>, (StatusCode, Json<ErrorBody>)> {
    // No/invalid JWT → 401
    let Extension(claims) = claims_ext.ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody {
                error: "unauthorized".to_string(),
            }),
        )
    })?;

    // Missing tenant_id param → 400
    let tenant_id_str = q.tenant_id.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "missing tenant_id parameter".to_string(),
            }),
        )
    })?;

    // Malformed UUID → 400
    let tenant_id = Uuid::parse_str(&tenant_id_str).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "invalid tenant_id: must be a valid UUID".to_string(),
            }),
        )
    })?;

    // Cross-tenant isolation: caller must own this tenant
    if tenant_id != claims.tenant_id {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorBody {
                error: "forbidden".to_string(),
            }),
        ));
    }

    let flags = feature_flags::list_flags_for_tenant(&state.pool, tenant_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list feature flags for tenant {}: {}", tenant_id, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "internal error".to_string(),
                }),
            )
        })?;

    Ok(Json(TenantFeaturesResponse {
        tenant_id: tenant_id.to_string(),
        flags,
    }))
}
