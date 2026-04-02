/// POST /api/control/tenants/{tenant_id}/retry
///
/// Re-triggers provisioning for a tenant in `failed` status.
/// Resets the tenant to `pending` and writes a new
/// `tenant.provisioning_started` event to the outbox.
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::Utc;
use serde::Serialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use tenant_registry::event_types;

use crate::models::ErrorBody;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct RetryResponse {
    pub tenant_id: Uuid,
    pub status: String,
    pub message: String,
}

pub async fn retry_provisioning(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<Uuid>,
) -> Result<(StatusCode, Json<RetryResponse>), (StatusCode, Json<ErrorBody>)> {
    let now = Utc::now();

    // Guard: tenant must be in failed status
    let row: Option<(String,)> =
        sqlx::query_as("SELECT status FROM tenants WHERE tenant_id = $1")
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
        Some((status,)) if status != "failed" => {
            return Err((
                StatusCode::CONFLICT,
                Json(ErrorBody {
                    error: format!(
                        "Tenant {tenant_id} is in '{status}' status — retry is only allowed for 'failed' tenants"
                    ),
                }),
            ));
        }
        _ => {}
    }

    // Reset tenant to pending
    sqlx::query("UPDATE tenants SET status = 'pending', updated_at = $1 WHERE tenant_id = $2")
        .bind(now)
        .bind(tenant_id)
        .execute(&state.pool)
        .await
        .map_err(db_error)?;

    // Reset failed/in_progress provisioning steps to pending
    sqlx::query(
        "UPDATE provisioning_steps \
         SET status = 'pending', started_at = NULL, completed_at = NULL, error_message = NULL \
         WHERE tenant_id = $1 AND status IN ('failed', 'in_progress')",
    )
    .bind(tenant_id)
    .execute(&state.pool)
    .await
    .map_err(db_error)?;

    // Write a new provisioning_started event to the outbox
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
        }),
    ))
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
