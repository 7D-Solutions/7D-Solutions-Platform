/// POST /api/control/tenants/{tenant_id}/retry
///
/// Re-triggers provisioning for a tenant in `failed` or `degraded` status.
///
/// - `failed` → full reset: tenant returns to `pending`, all failed/in-progress steps
///   are reset, and a new `tenant.provisioning_started` event is queued.
/// - `degraded` → targeted re-poll: only the modules currently marked `failed` in
///   `cp_tenant_module_status` are re-polled against `/api/ready?tenant_id=`. If all
///   respond `up` within 90 s, the tenant is promoted to `active`. Otherwise the tenant
///   stays `degraded` and the response body lists which modules are still failing.
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::Utc;
use serde::Serialize;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use tenant_registry::event_types;

use crate::models::ErrorBody;
use crate::provisioning::{load_registry_from_db, steps};
use crate::state::AppState;

/// Timeout for re-polling module readiness during a degraded retry.
const DEGRADED_RETRY_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Serialize)]
pub struct RetryResponse {
    pub tenant_id: Uuid,
    pub status: String,
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub still_failing: Vec<String>,
}

pub async fn retry_provisioning(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<Uuid>,
) -> Result<(StatusCode, Json<RetryResponse>), (StatusCode, Json<ErrorBody>)> {
    let row: Option<(String,)> = sqlx::query_as("SELECT status FROM tenants WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(db_error)?;

    match row {
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorBody {
                    error: format!("Tenant {tenant_id} not found"),
                }),
            ));
        }
        Some((ref status,)) if status == "degraded" => {
            return retry_degraded(&state, tenant_id).await;
        }
        Some((ref status,)) if status != "failed" => {
            return Err((
                StatusCode::CONFLICT,
                Json(ErrorBody {
                    error: format!(
                        "Tenant {tenant_id} is in '{status}' status — retry is only allowed for 'failed' or 'degraded' tenants"
                    ),
                }),
            ));
        }
        _ => {} // status == "failed" — fall through to full reset
    }

    // Full reset for failed tenants
    let now = Utc::now();

    sqlx::query("UPDATE tenants SET status = 'pending', updated_at = $1 WHERE tenant_id = $2")
        .bind(now)
        .bind(tenant_id)
        .execute(&state.pool)
        .await
        .map_err(db_error)?;

    sqlx::query(
        "UPDATE provisioning_steps \
         SET status = 'pending', started_at = NULL, completed_at = NULL, error_message = NULL \
         WHERE tenant_id = $1 AND status IN ('failed', 'in_progress')",
    )
    .bind(tenant_id)
    .execute(&state.pool)
    .await
    .map_err(db_error)?;

    let payload = json!({
        "tenant_id": tenant_id,
        "retry": true,
        "occurred_at": now,
    });

    sqlx::query(
        "INSERT INTO provisioning_outbox (tenant_id, event_type, payload, created_at) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(tenant_id)
    .bind(event_types::TENANT_PROVISIONING_STARTED)
    .bind(&payload)
    .bind(now)
    .execute(&state.pool)
    .await
    .map_err(db_error)?;

    Ok((
        StatusCode::ACCEPTED,
        Json(RetryResponse {
            tenant_id,
            status: "pending".to_string(),
            message: "Provisioning retry queued. Completed steps will be skipped.".to_string(),
            still_failing: vec![],
        }),
    ))
}

/// Targeted re-poll for a degraded tenant.
///
/// Queries `cp_tenant_module_status` for modules in `failed` state, re-polls
/// each module's `/api/ready?tenant_id=` endpoint, and updates status accordingly.
/// Promotes the tenant to `active` when all modules are now ready.
async fn retry_degraded(
    state: &AppState,
    tenant_id: Uuid,
) -> Result<(StatusCode, Json<RetryResponse>), (StatusCode, Json<ErrorBody>)> {
    // Find which modules are currently failing
    let failed_modules: Vec<String> = sqlx::query_scalar(
        "SELECT module_code FROM cp_tenant_module_status \
         WHERE tenant_id = $1 AND status = 'failed' \
         ORDER BY module_code",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await
    .map_err(db_error)?;

    if failed_modules.is_empty() {
        // No failing modules — promote to active
        let now = Utc::now();
        sqlx::query("UPDATE tenants SET status = 'active', updated_at = $1 WHERE tenant_id = $2")
            .bind(now)
            .bind(tenant_id)
            .execute(&state.pool)
            .await
            .map_err(db_error)?;

        return Ok((
            StatusCode::OK,
            Json(RetryResponse {
                tenant_id,
                status: "active".to_string(),
                message: "All modules are ready — tenant activated.".to_string(),
                still_failing: vec![],
            }),
        ));
    }

    // Load registry (module base URLs come from env vars via cp_bundle_modules)
    let registry = load_registry_from_db(&state.pool).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!("Failed to load module registry: {e}"),
            }),
        )
    })?;

    let http_client = reqwest::Client::new();

    // Re-poll only the failing modules
    let still_failing = steps::poll_module_readiness(
        tenant_id,
        &failed_modules,
        &registry,
        &http_client,
        DEGRADED_RETRY_TIMEOUT,
    )
    .await;

    let now = Utc::now();

    // Mark newly-ready modules
    for code in &failed_modules {
        if !still_failing.contains(code) {
            let _ = sqlx::query(
                "UPDATE cp_tenant_module_status \
                 SET status = 'ready', updated_at = $1 \
                 WHERE tenant_id = $2 AND module_code = $3",
            )
            .bind(now)
            .bind(tenant_id)
            .bind(code)
            .execute(&state.pool)
            .await;
        }
    }

    if still_failing.is_empty() {
        sqlx::query("UPDATE tenants SET status = 'active', updated_at = $1 WHERE tenant_id = $2")
            .bind(now)
            .bind(tenant_id)
            .execute(&state.pool)
            .await
            .map_err(db_error)?;

        Ok((
            StatusCode::OK,
            Json(RetryResponse {
                tenant_id,
                status: "active".to_string(),
                message: "All modules are now ready — tenant activated.".to_string(),
                still_failing: vec![],
            }),
        ))
    } else {
        tracing::warn!(
            tenant_id = %tenant_id,
            still_failing = ?still_failing,
            "degraded retry: some modules still not ready"
        );

        Ok((
            StatusCode::ACCEPTED,
            Json(RetryResponse {
                tenant_id,
                status: "degraded".to_string(),
                message: format!(
                    "Re-poll completed. {} module(s) still not ready.",
                    still_failing.len()
                ),
                still_failing,
            }),
        ))
    }
}

fn db_error(e: sqlx::Error) -> (StatusCode, Json<ErrorBody>) {
    tracing::error!("Database error: {}", e);
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorBody {
            error: "Internal database error".to_string(),
        }),
    )
}
