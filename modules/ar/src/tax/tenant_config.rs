//! Per-tenant tax calculation source configuration repository (bd-kkhf4).
//!
//! Tenants choose whether tax is computed by the platform (Avalara/local/zero)
//! or deferred to their external accounting software (QBO AST).
//!
//! The `config_version` increments atomically on every `set` call and is
//! embedded in tax cache idempotency keys so that config changes automatically
//! invalidate cached quotes, preventing stale values on in-flight invoice batches.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    contracts::{
        build_tax_config_changed_envelope, TaxConfigChangedPayload,
        EVENT_TYPE_TAX_CONFIG_CHANGED,
    },
    outbox::enqueue_event_tx,
};

// ============================================================================
// Model
// ============================================================================

/// Resolved tenant tax configuration (or synthesised default when no row exists).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxTenantConfig {
    pub tenant_id: Uuid,
    /// "platform" | "external_accounting_software"
    pub tax_calculation_source: String,
    /// "local" | "zero" | "avalara"  (only relevant when source == platform)
    pub provider_name: String,
    /// Monotonically increasing counter; incremented on each SET call.
    pub config_version: i64,
    pub updated_at: DateTime<Utc>,
    pub updated_by: Uuid,
}

impl TaxTenantConfig {
    /// Synthesise the default config for a tenant that has no explicit row.
    ///
    /// Default: external_accounting_software — preserves legacy QBO AST behavior
    /// so that existing tenants are never silently switched to platform tax.
    pub fn default_for(tenant_id: Uuid) -> Self {
        TaxTenantConfig {
            tenant_id,
            tax_calculation_source: "external_accounting_software".to_string(),
            provider_name: "local".to_string(),
            config_version: 1,
            updated_at: Utc::now(),
            updated_by: Uuid::nil(),
        }
    }

    /// Returns true when the platform provider should compute tax.
    pub fn is_platform_source(&self) -> bool {
        self.tax_calculation_source == "platform"
    }
}

// ============================================================================
// Repository
// ============================================================================

/// Fetch the tenant's tax configuration, returning the default if no row exists.
///
/// The default is: source=external_accounting_software, provider=local, config_version=1.
/// No row is written for the default — callers must call `set` explicitly to persist.
pub async fn get(pool: &PgPool, tenant_id: Uuid) -> Result<TaxTenantConfig, sqlx::Error> {
    let row: Option<(String, String, i64, DateTime<Utc>, Uuid)> = sqlx::query_as(
        r#"
        SELECT tax_calculation_source, provider_name, config_version, updated_at, updated_by
        FROM ar_tenant_tax_config
        WHERE tenant_id = $1
        "#,
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    Ok(row
        .map(|(source, provider, version, updated_at, updated_by)| TaxTenantConfig {
            tenant_id,
            tax_calculation_source: source,
            provider_name: provider,
            config_version: version,
            updated_at,
            updated_by,
        })
        .unwrap_or_else(|| TaxTenantConfig::default_for(tenant_id)))
}

/// Upsert the tenant's tax calculation source and provider.
///
/// Increments config_version atomically on each call.
/// Emits a TaxConfigChanged event to the outbox within the same transaction,
/// stamped at the exact transition time so reconciliation workers can open a
/// new diff window at the precise moment of the flip.
///
/// Returns the updated TaxTenantConfig.
pub async fn set(
    pool: &PgPool,
    tenant_id: Uuid,
    source: &str,
    provider: &str,
    updated_by: Uuid,
    correlation_id: &str,
) -> Result<TaxTenantConfig, sqlx::Error> {
    let mut tx = pool.begin().await?;

    // Upsert with atomic config_version increment
    let row: (String, String, i64, DateTime<Utc>) = sqlx::query_as(
        r#"
        INSERT INTO ar_tenant_tax_config
            (tenant_id, tax_calculation_source, provider_name, config_version, updated_at, updated_by)
        VALUES ($1, $2, $3, 1, NOW(), $4)
        ON CONFLICT (tenant_id) DO UPDATE SET
            tax_calculation_source = EXCLUDED.tax_calculation_source,
            provider_name          = EXCLUDED.provider_name,
            config_version         = ar_tenant_tax_config.config_version + 1,
            updated_at             = NOW(),
            updated_by             = EXCLUDED.updated_by
        RETURNING tax_calculation_source, provider_name, config_version, updated_at
        "#,
    )
    .bind(tenant_id)
    .bind(source)
    .bind(provider)
    .bind(updated_by)
    .fetch_one(&mut *tx)
    .await?;

    let config = TaxTenantConfig {
        tenant_id,
        tax_calculation_source: row.0.clone(),
        provider_name: row.1.clone(),
        config_version: row.2,
        updated_at: row.3,
        updated_by,
    };

    // Emit TaxConfigChanged into the outbox before the transaction commits.
    // Subscribers (reconciliation worker) use this to open a new diff window
    // at the transition timestamp.
    let event_payload = TaxConfigChangedPayload {
        tenant_id: tenant_id.to_string(),
        tax_calculation_source: config.tax_calculation_source.clone(),
        provider_name: config.provider_name.clone(),
        config_version: config.config_version,
        updated_by: updated_by.to_string(),
        changed_at: config.updated_at,
    };

    let envelope = build_tax_config_changed_envelope(
        Uuid::new_v4(),
        tenant_id.to_string(),
        correlation_id.to_string(),
        None,
        event_payload,
    );

    enqueue_event_tx(
        &mut tx,
        EVENT_TYPE_TAX_CONFIG_CHANGED,
        "tenant_tax_config",
        &tenant_id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;

    Ok(config)
}
