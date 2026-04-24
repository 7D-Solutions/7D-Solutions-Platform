/// GET /api/features?tenant_id={uuid}
/// GET /api/schemas/features/v{N}
///
/// Returns all feature flags visible to the authenticated tenant, versioned by schema_version.
/// Requires a valid JWT whose tenant_id claim matches the query parameter.
/// Schema endpoint is unauthenticated and returns the JSON Schema for v{N} (404 if unknown).
use axum::{
    extract::{Extension, Path, Query, State},
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

/// Current schema version for TenantFeaturesResponse. Bump when the shape changes.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
pub struct TenantFeaturesQuery {
    pub tenant_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TenantFeaturesResponse {
    pub tenant_id: String,
    pub schema_version: u32,
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
            tracing::error!(
                "Failed to list feature flags for tenant {}: {}",
                tenant_id,
                e
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "internal error".to_string(),
                }),
            )
        })?;

    Ok(Json(TenantFeaturesResponse {
        tenant_id: tenant_id.to_string(),
        schema_version: SCHEMA_VERSION,
        flags,
    }))
}

/// GET /api/schemas/features/v{version}
///
/// Returns the JSON Schema document for the given feature-flags payload version.
/// Returns 404 for unknown versions so frontends can fail-fast on unrecognized schemas.
pub async fn features_schema(
    Path(version): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match version.as_str() {
        "v1" => Ok(Json(serde_json::json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "/api/schemas/features/v1",
            "type": "object",
            "required": ["tenant_id", "schema_version", "flags"],
            "properties": {
                "tenant_id": {"type": "string", "format": "uuid"},
                "schema_version": {"type": "integer", "const": 1},
                "flags": {
                    "type": "object",
                    "additionalProperties": {"type": "boolean"}
                }
            },
            "additionalProperties": false
        }))),
        _ => Err(StatusCode::NOT_FOUND),
    }
}
