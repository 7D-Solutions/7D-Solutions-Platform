/// POST /api/control/platform-billing-runs handler
///
/// Runs the platform billing cycle for a given period (e.g. "2026-02").
/// The platform bills tenants — invoices are created in AR under the PLATFORM app_id.
/// Events are emitted to the provisioning_outbox with merchant_context=PLATFORM.
///
/// Idempotency:
///   - Re-running for the same period produces no duplicate invoices.
///   - Each tenant's invoice is keyed by correlation_id = "plat-{tenant_id}-{period}".
use axum::{extract::State, http::StatusCode, Json};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::clients::ar::{
    create_platform_invoice_idempotent, find_or_create_platform_customer,
    InvoiceResult, PLATFORM_APP_ID,
};
use crate::clients::tenant_registry::{fetch_eligible_tenants, fetch_plan_fee_minor};
use crate::models::ErrorBody;
use crate::state::AppState;

// ============================================================================
// Request / Response types
// ============================================================================

/// Request body for POST /api/control/platform-billing-runs
#[derive(Debug, Deserialize)]
pub struct PlatformBillingRunRequest {
    /// Billing period in "YYYY-MM" format (e.g. "2026-02").
    pub period: String,
}

/// Summary of a processed tenant invoice.
#[derive(Debug, Serialize)]
pub struct ProcessedEntry {
    pub tenant_id: Uuid,
    pub invoice_id: i32,
    pub amount_cents: i32,
    pub plan_code: String,
}

/// Summary of a skipped tenant (invoice already existed for the period).
#[derive(Debug, Serialize)]
pub struct SkippedEntry {
    pub tenant_id: Uuid,
    pub reason: String,
}

/// Response body for POST /api/control/platform-billing-runs
#[derive(Debug, Serialize)]
pub struct PlatformBillingRunResponse {
    pub period: String,
    pub eligible_tenant_count: usize,
    pub processed: Vec<ProcessedEntry>,
    pub skipped: Vec<SkippedEntry>,
    pub merchant_context: String,
}

// ============================================================================
// Handler
// ============================================================================

/// POST /api/control/platform-billing-runs
pub async fn platform_billing_run(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PlatformBillingRunRequest>,
) -> Result<(StatusCode, Json<PlatformBillingRunResponse>), (StatusCode, Json<ErrorBody>)> {
    // Validate period format: must be "YYYY-MM"
    if !is_valid_period(&req.period) {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody {
                error: "period must be in YYYY-MM format (e.g. '2026-02')".to_string(),
            }),
        ));
    }

    // AR pool must be configured
    let ar_pool = match &state.ar_pool {
        Some(p) => p,
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorBody {
                    error: "AR database not configured; set AR_DATABASE_URL".to_string(),
                }),
            ));
        }
    };

    // Fetch eligible tenants from tenant-registry
    let tenants = fetch_eligible_tenants(&state.pool).await.map_err(|e| {
        tracing::error!("Failed to fetch eligible tenants: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!("Failed to fetch tenants: {e}"),
            }),
        )
    })?;

    let eligible_count = tenants.len();
    let mut processed = Vec::new();
    let mut skipped = Vec::new();

    for tenant in tenants {
        // Look up monthly fee from cp_plans using the tenant's product tier.
        // Returns 0 if the product_code is not found in cp_plans (logs a warning).
        let amount_cents: i32 = match fetch_plan_fee_minor(&state.pool, &tenant.product_code).await {
            Ok(Some(fee)) => fee as i32,
            Ok(None) => {
                tracing::warn!(
                    tenant_id = %tenant.tenant_id,
                    product_code = %tenant.product_code,
                    "No cp_plans entry for product_code — billing $0"
                );
                0
            }
            Err(e) => {
                tracing::error!(
                    tenant_id = %tenant.tenant_id,
                    "Failed to fetch plan fee: {}", e
                );
                skipped.push(SkippedEntry {
                    tenant_id: tenant.tenant_id,
                    reason: format!("plan_fee_lookup_failed: {e}"),
                });
                continue;
            }
        };

        // Find or create AR customer under PLATFORM app_id
        let customer_id = match find_or_create_platform_customer(ar_pool, tenant.tenant_id).await {
            Ok(id) => id,
            Err(e) => {
                tracing::error!(
                    tenant_id = %tenant.tenant_id,
                    "Failed to upsert PLATFORM customer: {}", e
                );
                skipped.push(SkippedEntry {
                    tenant_id: tenant.tenant_id,
                    reason: format!("customer_upsert_failed: {e}"),
                });
                continue;
            }
        };

        // Create invoice (idempotent per period)
        match create_platform_invoice_idempotent(
            ar_pool,
            customer_id,
            tenant.tenant_id,
            &req.period,
            amount_cents,
        )
        .await
        {
            Ok(InvoiceResult::Created(invoice_id)) => {
                tracing::info!(
                    tenant_id = %tenant.tenant_id,
                    invoice_id,
                    amount_cents,
                    period = %req.period,
                    "PLATFORM invoice created"
                );

                // Emit billing event to provisioning_outbox with merchant_context=PLATFORM
                emit_billing_event(
                    &state.pool,
                    tenant.tenant_id,
                    &req.period,
                    invoice_id,
                    amount_cents,
                    &tenant.plan_code,
                )
                .await;

                processed.push(ProcessedEntry {
                    tenant_id: tenant.tenant_id,
                    invoice_id,
                    amount_cents,
                    plan_code: tenant.plan_code,
                });
            }
            Ok(InvoiceResult::AlreadyExists) => {
                tracing::debug!(
                    tenant_id = %tenant.tenant_id,
                    period = %req.period,
                    "Invoice already exists — skipping (idempotent)"
                );
                skipped.push(SkippedEntry {
                    tenant_id: tenant.tenant_id,
                    reason: "already_billed".to_string(),
                });
            }
            Err(e) => {
                tracing::error!(
                    tenant_id = %tenant.tenant_id,
                    "Failed to create PLATFORM invoice: {}", e
                );
                skipped.push(SkippedEntry {
                    tenant_id: tenant.tenant_id,
                    reason: format!("invoice_creation_failed: {e}"),
                });
            }
        }
    }

    Ok((
        StatusCode::OK,
        Json(PlatformBillingRunResponse {
            period: req.period,
            eligible_tenant_count: eligible_count,
            processed,
            skipped,
            merchant_context: PLATFORM_APP_ID.to_uppercase(),
        }),
    ))
}

// ============================================================================
// Helpers
// ============================================================================

/// Validate that the period string is in "YYYY-MM" format.
fn is_valid_period(period: &str) -> bool {
    let parts: Vec<&str> = period.splitn(2, '-').collect();
    if parts.len() != 2 {
        return false;
    }
    let year_ok = parts[0].len() == 4 && parts[0].parse::<i32>().is_ok();
    let month_ok = parts[1].len() == 2 && matches!(parts[1].parse::<u32>(), Ok(m) if (1..=12).contains(&m));
    year_ok && month_ok
}

/// Emit a platform.billing.invoice_created event to the provisioning_outbox.
///
/// Events carry merchant_context=PLATFORM to satisfy the money-mixing prevention rule.
/// Errors are logged but do not abort the billing run.
async fn emit_billing_event(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    period: &str,
    invoice_id: i32,
    amount_cents: i32,
    plan_code: &str,
) {
    let now = Utc::now();
    let payload = json!({
        "tenant_id": tenant_id,
        "period": period,
        "invoice_id": invoice_id,
        "amount_cents": amount_cents,
        "plan_code": plan_code,
        "app_id": PLATFORM_APP_ID,
        "merchant_context": "PLATFORM",
        "occurred_at": now,
    });

    if let Err(e) = sqlx::query(
        r#"
        INSERT INTO provisioning_outbox (tenant_id, event_type, payload, created_at)
        VALUES ($1, 'platform.billing.invoice_created', $2, $3)
        "#,
    )
    .bind(tenant_id)
    .bind(&payload)
    .bind(now)
    .execute(pool)
    .await
    {
        tracing::error!(tenant_id = %tenant_id, "Failed to emit billing event: {}", e);
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn period_validation_accepts_valid_formats() {
        assert!(is_valid_period("2026-02"));
        assert!(is_valid_period("2026-12"));
        assert!(is_valid_period("2026-01"));
    }

    #[test]
    fn period_validation_rejects_invalid_formats() {
        assert!(!is_valid_period("2026-2"));    // month must be 2 digits
        assert!(!is_valid_period("26-02"));     // year must be 4 digits
        assert!(!is_valid_period("2026-00"));   // month 0 invalid
        assert!(!is_valid_period("2026-13"));   // month 13 invalid
        assert!(!is_valid_period("2026"));      // missing month
        assert!(!is_valid_period(""));
    }

    #[tokio::test]
    async fn platform_billing_run_integration_test() {
        let tr_db_url = std::env::var("TENANT_REGISTRY_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://tenant_registry_user:tenant_registry_pass@localhost:5441/tenant_registry_db"
                .to_string()
        });
        let ar_db_url = std::env::var("AR_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://ar_user:ar_pass@localhost:5434/ar_db".to_string()
        });

        let tr_pool = match sqlx::PgPool::connect(&tr_db_url).await {
            Ok(p) => p,
            Err(_) => return, // skip if TR DB unavailable
        };
        let ar_pool = match sqlx::PgPool::connect(&ar_db_url).await {
            Ok(p) => p,
            Err(_) => return, // skip if AR DB unavailable
        };

        // Insert an active test tenant with plan_code
        let tenant_id = Uuid::new_v4();
        let app_id = format!("app-{}", &tenant_id.to_string().replace('-', "")[..12]);
        sqlx::query(
            r#"INSERT INTO tenants
               (tenant_id, status, environment, module_schema_versions,
                product_code, plan_code, app_id, created_at, updated_at)
               VALUES ($1, 'active', 'development', '{}'::jsonb, 'starter', 'monthly', $2, NOW(), NOW())"#,
        )
        .bind(tenant_id)
        .bind(&app_id)
        .execute(&tr_pool)
        .await
        .expect("insert test tenant");

        let state = Arc::new(AppState {
            pool: tr_pool.clone(),
            ar_pool: Some(ar_pool.clone()),
        });

        let period = format!("2099-{:02}", (tenant_id.as_u128() % 12 + 1) as u32);

        // First run: should process the tenant
        let result = platform_billing_run(
            State(state.clone()),
            Json(PlatformBillingRunRequest { period: period.clone() }),
        )
        .await
        .expect("billing run should succeed");

        let (status, Json(resp)) = result;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(resp.merchant_context, "PLATFORM");
        let entry = resp.processed.iter().find(|e| e.tenant_id == tenant_id)
            .expect("tenant should appear in processed list");
        // Pricing sourced from cp_plans: starter = 2900 minor units
        assert_eq!(entry.amount_cents, 2900, "starter plan fee should be 2900 (from cp_plans)");

        // Second run (same period): should skip — idempotent
        let result2 = platform_billing_run(
            State(state.clone()),
            Json(PlatformBillingRunRequest { period: period.clone() }),
        )
        .await
        .expect("second billing run should succeed");

        let (_, Json(resp2)) = result2;
        assert!(resp2.skipped.iter().any(|e| e.tenant_id == tenant_id));
        assert!(resp2.skipped.iter().any(|e| e.reason == "already_billed"));

        // Cleanup AR records
        sqlx::query(
            "DELETE FROM ar_invoices WHERE app_id = $1 AND correlation_id LIKE $2",
        )
        .bind(PLATFORM_APP_ID)
        .bind(format!("plat-{}-%", tenant_id))
        .execute(&ar_pool)
        .await
        .ok();
        sqlx::query(
            "DELETE FROM ar_customers WHERE app_id = $1 AND external_customer_id = $2",
        )
        .bind(PLATFORM_APP_ID)
        .bind(tenant_id.to_string())
        .execute(&ar_pool)
        .await
        .ok();

        // Cleanup tenant-registry
        sqlx::query("DELETE FROM tenants WHERE tenant_id = $1")
            .bind(tenant_id)
            .execute(&tr_pool)
            .await
            .ok();
    }
}
