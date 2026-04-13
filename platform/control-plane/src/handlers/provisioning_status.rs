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
    pub module_statuses: Vec<ModuleStatusDto>,
}

#[derive(Debug, Serialize)]
pub struct ModuleStatusDto {
    pub module_code: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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

    // Get per-module status from cp_tenant_module_status.
    // Modules in the tenant's bundle that haven't been seeded yet appear as 'pending'.
    let module_rows: Vec<ModuleStatusRow> = sqlx::query_as(
        "SELECT bm.module_code, \
                COALESCE(ms.status, 'pending') AS status, \
                ms.error_msg \
         FROM   cp_tenant_bundle tb \
         JOIN   cp_bundle_modules bm ON bm.bundle_id = tb.bundle_id \
         LEFT JOIN cp_tenant_module_status ms \
             ON ms.tenant_id = tb.tenant_id AND ms.module_code = bm.module_code \
         WHERE  tb.tenant_id = $1 \
         ORDER BY bm.module_code",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await
    .map_err(db_error)?;

    let module_statuses: Vec<ModuleStatusDto> = module_rows
        .into_iter()
        .map(|r| ModuleStatusDto {
            module_code: r.module_code,
            status: r.status,
            error: r.error_msg,
        })
        .collect();

    Ok(Json(ProvisioningStatusResponse {
        tenant_id,
        status: tenant_status,
        steps,
        module_statuses,
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

#[derive(sqlx::FromRow)]
struct ModuleStatusRow {
    module_code: String,
    status: String,
    error_msg: Option<String>,
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
