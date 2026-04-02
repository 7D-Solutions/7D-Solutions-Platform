/// GET /api/control/tenants/{tenant_id}/provisioning
///
/// Returns step-level provisioning status for a tenant.
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::models::ErrorBody;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct ProvisioningStatusResponse {
    pub tenant_id: Uuid,
    pub status: String,
    pub steps: Vec<ProvisioningStepDto>,
}

#[derive(Debug, Serialize)]
pub struct ProvisioningStepDto {
    pub step: String,
    pub order: i32,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

pub async fn provisioning_status(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<Uuid>,
) -> Result<Json<ProvisioningStatusResponse>, (StatusCode, Json<ErrorBody>)> {
    // Get tenant status
    let tenant: Option<(String,)> =
        sqlx::query_as("SELECT status FROM tenants WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_optional(&state.pool)
            .await
            .map_err(db_error)?;

    let tenant_status = match tenant {
        Some((s,)) => s,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorBody {
                    error: format!("Tenant {tenant_id} not found"),
                }),
            ));
        }
    };

    // Get provisioning steps
    let rows: Vec<StepRow> = sqlx::query_as(
        "SELECT step_name, step_order, status, started_at, completed_at, error_message \
         FROM provisioning_steps \
         WHERE tenant_id = $1 \
         ORDER BY step_order ASC",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await
    .map_err(db_error)?;

    let steps: Vec<ProvisioningStepDto> = rows
        .into_iter()
        .map(|r| ProvisioningStepDto {
            step: r.step_name,
            order: r.step_order,
            status: r.status,
            started_at: r.started_at.map(|t| t.to_rfc3339()),
            completed_at: r.completed_at.map(|t| t.to_rfc3339()),
            error_message: r.error_message,
        })
        .collect();

    Ok(Json(ProvisioningStatusResponse {
        tenant_id,
        status: tenant_status,
        steps,
    }))
}

#[derive(sqlx::FromRow)]
struct StepRow {
    step_name: String,
    step_order: i32,
    status: String,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
    error_message: Option<String>,
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
