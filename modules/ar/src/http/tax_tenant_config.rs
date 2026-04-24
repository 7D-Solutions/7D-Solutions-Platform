//! Admin API for tenant tax calculation source configuration (bd-kkhf4).
//!
//! PUT /api/ar/tax/tenant-config  — set the tax source and provider for the caller's tenant.
//! GET /api/ar/tax/tenant-config  — fetch current config (or default if none set).
//!
//! Permission: `tenant_admin` or `admin` role required (enforced inside handlers).

use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use rust_decimal::Decimal;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::tax;

// ============================================================================
// Request / Response types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct PutTaxTenantConfigRequest {
    /// "platform" | "external_accounting_software"
    pub tax_calculation_source: String,
    /// "local" | "zero" | "avalara"
    pub provider_name: String,
}

#[derive(Debug, Serialize)]
pub struct TaxTenantConfigResponse {
    pub tenant_id: String,
    pub tax_calculation_source: String,
    pub provider_name: String,
    pub config_version: i64,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub reconciliation_threshold_pct: Decimal,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

// ============================================================================
// GET /api/ar/tax/tenant-config
// ============================================================================

pub async fn get_tax_tenant_config(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match super::tenant::extract_tenant(&claims) {
        Ok(id) => id,
        Err(err) => return err.into_response(),
    };

    let tenant_uuid = match uuid::Uuid::parse_str(&tenant_id) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody {
                    error: "invalid tenant_id".to_string(),
                }),
            )
                .into_response();
        }
    };

    match tax::tenant_config::get(&pool, tenant_uuid).await {
        Ok(cfg) => (
            StatusCode::OK,
            Json(TaxTenantConfigResponse {
                tenant_id: cfg.tenant_id.to_string(),
                tax_calculation_source: cfg.tax_calculation_source,
                provider_name: cfg.provider_name,
                config_version: cfg.config_version,
                updated_at: cfg.updated_at,
                reconciliation_threshold_pct: cfg.reconciliation_threshold_pct,
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "Failed to load tenant tax config");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "internal error".to_string(),
                }),
            )
                .into_response()
        }
    }
}

// ============================================================================
// PUT /api/ar/tax/tenant-config
// ============================================================================

pub async fn put_tax_tenant_config(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(body): Json<PutTaxTenantConfigRequest>,
) -> impl IntoResponse {
    let tenant_id = match super::tenant::extract_tenant(&claims) {
        Ok(id) => id,
        Err(err) => return err.into_response(),
    };

    let tenant_uuid = match uuid::Uuid::parse_str(&tenant_id) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody {
                    error: "invalid tenant_id".to_string(),
                }),
            )
                .into_response();
        }
    };

    // Enforce tenant_admin or admin role
    let caller_uuid = match &claims {
        Some(Extension(c)) => {
            if !c.roles.iter().any(|r| r == "tenant_admin" || r == "admin") {
                return (
                    StatusCode::FORBIDDEN,
                    Json(ErrorBody {
                        error: "tenant_admin role required".to_string(),
                    }),
                )
                    .into_response();
            }
            c.user_id
        }
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ErrorBody {
                    error: "authentication required".to_string(),
                }),
            )
                .into_response();
        }
    };

    // Validate values against the CHECK constraints
    let valid_sources = ["platform", "external_accounting_software"];
    if !valid_sources.contains(&body.tax_calculation_source.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: format!(
                    "tax_calculation_source must be one of: {}",
                    valid_sources.join(", ")
                ),
            }),
        )
            .into_response();
    }

    let valid_providers = ["local", "zero", "avalara"];
    if !valid_providers.contains(&body.provider_name.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: format!(
                    "provider_name must be one of: {}",
                    valid_providers.join(", ")
                ),
            }),
        )
            .into_response();
    }

    let correlation_id = uuid::Uuid::new_v4().to_string();

    match tax::tenant_config::set(
        &pool,
        tenant_uuid,
        &body.tax_calculation_source,
        &body.provider_name,
        caller_uuid,
        &correlation_id,
    )
    .await
    {
        Ok(cfg) => {
            // Write audit event
            let _ = write_audit_event(
                &pool,
                caller_uuid,
                tenant_uuid,
                &cfg.tax_calculation_source,
                &cfg.provider_name,
                cfg.config_version,
            )
            .await;

            (
                StatusCode::OK,
                Json(TaxTenantConfigResponse {
                    tenant_id: cfg.tenant_id.to_string(),
                    tax_calculation_source: cfg.tax_calculation_source,
                    provider_name: cfg.provider_name,
                    config_version: cfg.config_version,
                    updated_at: cfg.updated_at,
                    reconciliation_threshold_pct: cfg.reconciliation_threshold_pct,
                }),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to set tenant tax config");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "internal error".to_string(),
                }),
            )
                .into_response()
        }
    }
}

async fn write_audit_event(
    pool: &PgPool,
    actor_id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    source: &str,
    provider: &str,
    config_version: i64,
) -> Result<(), sqlx::Error> {
    let after = serde_json::json!({
        "tax_calculation_source": source,
        "provider_name": provider,
        "config_version": config_version,
    });

    sqlx::query(
        r#"
        INSERT INTO audit_events
            (actor_id, actor_type, action, mutation_class, entity_type, entity_id, after_snapshot)
        VALUES ($1, 'User', 'ar_tenant_tax_config.updated', 'UPDATE', 'tenant_tax_config', $2, $3)
        "#,
    )
    .bind(actor_id)
    .bind(tenant_id.to_string())
    .bind(after)
    .execute(pool)
    .await?;

    Ok(())
}
