/// GET /api/control/tenants/{tenant_id}/vitals
///
/// Aggregates provisioning step status and per-module operational vitals for a tenant.
/// Module HTTP calls use a 2s per-module timeout; non-responding modules are excluded
/// from the overall_healthy gate rather than treated as failures.
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::Utc;
use futures::future::join_all;
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

use crate::models::ErrorBody;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct TenantVitalsResponse {
    pub tenant_id: Uuid,
    pub tenant_status: String,
    pub provisioning: ProvisioningVitals,
    pub modules: Vec<ModuleVitalsEntry>,
    pub overall_healthy: bool,
    pub timestamp: String,
}

#[derive(Debug, Serialize)]
pub struct ProvisioningVitals {
    pub all_steps_complete: bool,
    pub steps: Vec<ProvisioningStepSummary>,
    pub module_status: Vec<ModuleStatusSummary>,
}

#[derive(Debug, Serialize)]
pub struct ProvisioningStepSummary {
    pub step: String,
    pub order: i32,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct ModuleStatusSummary {
    pub module_code: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct ModuleVitalsEntry {
    pub module: String,
    pub vitals: Option<health::VitalsResponse>,
    pub latency_ms: u64,
    pub error: Option<String>,
}

pub async fn tenant_vitals(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<Uuid>,
) -> Result<Json<TenantVitalsResponse>, (StatusCode, Json<ErrorBody>)> {
    // 1. Verify tenant exists and get its status
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

    // 2. Fetch provisioning steps
    let step_rows: Vec<StepRow> = sqlx::query_as(
        "SELECT step_name, step_order, status \
         FROM provisioning_steps \
         WHERE tenant_id = $1 \
         ORDER BY step_order",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await
    .map_err(db_error)?;

    let all_steps_complete = !step_rows.is_empty()
        && step_rows.iter().all(|r| r.status == "completed");

    let steps: Vec<ProvisioningStepSummary> = step_rows
        .iter()
        .map(|r| ProvisioningStepSummary {
            step: r.step_name.clone(),
            order: r.step_order,
            status: r.status.clone(),
        })
        .collect();

    // 3. Fetch per-module provisioning status from tenant's bundle
    let module_rows: Vec<ModuleStatusRow> = sqlx::query_as(
        "SELECT bm.module_code, COALESCE(ms.status, 'pending') AS status \
         FROM cp_tenant_bundle tb \
         JOIN cp_bundle_modules bm ON bm.bundle_id = tb.bundle_id \
         LEFT JOIN cp_tenant_module_status ms \
             ON ms.tenant_id = tb.tenant_id AND ms.module_code = bm.module_code \
         WHERE tb.tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await
    .map_err(db_error)?;

    let module_status: Vec<ModuleStatusSummary> = module_rows
        .iter()
        .map(|r| ModuleStatusSummary {
            module_code: r.module_code.clone(),
            status: r.status.clone(),
        })
        .collect();

    // 4. Look up base URLs from service catalog (skip if no modules)
    let module_codes: Vec<String> = module_rows.iter().map(|r| r.module_code.clone()).collect();

    let catalog_rows: Vec<CatalogRow> = if module_codes.is_empty() {
        vec![]
    } else {
        sqlx::query_as(
            "SELECT module_code, base_url FROM cp_service_catalog \
             WHERE module_code = ANY($1)",
        )
        .bind(&module_codes)
        .fetch_all(&state.pool)
        .await
        .map_err(db_error)?
    };

    // 5. Fan out parallel vitals requests with 2s per-module timeout
    let client = state.http_client.clone();
    let vitals_futures: Vec<_> = catalog_rows
        .into_iter()
        .map(|row| {
            let client = client.clone();
            let url = format!("{}/api/vitals?tenant_id={}", row.base_url, tenant_id);
            let module = row.module_code.clone();
            async move {
                let t0 = Instant::now();
                let result = client
                    .get(&url)
                    .timeout(std::time::Duration::from_secs(2))
                    .send()
                    .await;
                let latency_ms = t0.elapsed().as_millis() as u64;

                match result {
                    Ok(resp) if resp.status().is_success() => {
                        match resp.json::<health::VitalsResponse>().await {
                            Ok(v) => ModuleVitalsEntry {
                                module,
                                vitals: Some(v),
                                latency_ms,
                                error: None,
                            },
                            Err(e) => ModuleVitalsEntry {
                                module,
                                vitals: None,
                                latency_ms,
                                error: Some(format!("parse error: {e}")),
                            },
                        }
                    }
                    Ok(resp) => ModuleVitalsEntry {
                        module,
                        vitals: None,
                        latency_ms,
                        error: Some(format!("HTTP {}", resp.status().as_u16())),
                    },
                    Err(e) => ModuleVitalsEntry {
                        module,
                        vitals: None,
                        latency_ms,
                        error: Some(e.to_string()),
                    },
                }
            }
        })
        .collect();

    let modules: Vec<ModuleVitalsEntry> = join_all(vitals_futures).await;

    // 6. Compute overall_healthy:
    //    - All provisioning steps must be completed
    //    - Every module that responded (vitals != None) must have:
    //        dlq.total == 0 AND outbox.pending == 0 AND tenant_ready != false
    //    - Modules with vitals == None are excluded (migration window)
    //    - tenant_ready == None is treated as false (conservative)
    let modules_healthy = modules.iter().all(|m| match &m.vitals {
        None => true, // excluded from gate
        Some(v) => {
            v.dlq.total == 0
                && v.outbox.pending == 0
                && v.tenant_ready.unwrap_or(false)
        }
    });

    let overall_healthy = all_steps_complete && modules_healthy;

    Ok(Json(TenantVitalsResponse {
        tenant_id,
        tenant_status,
        provisioning: ProvisioningVitals {
            all_steps_complete,
            steps,
            module_status,
        },
        modules,
        overall_healthy,
        timestamp: Utc::now().to_rfc3339(),
    }))
}

#[derive(sqlx::FromRow)]
struct StepRow {
    step_name: String,
    step_order: i32,
    status: String,
}

#[derive(sqlx::FromRow)]
struct ModuleStatusRow {
    module_code: String,
    status: String,
}

#[derive(sqlx::FromRow)]
struct CatalogRow {
    module_code: String,
    base_url: String,
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
