//! Tenant-level maintenance configuration.
//!
//! Controls per-tenant behavior:
//! - `auto_create_on_due`: When a plan assignment becomes due, automatically create a work order.
//! - `approvals_required`: Auto-created work orders start as `awaiting_approval` instead of `scheduled`.
//!
//! If no row exists for a tenant, defaults apply (both false).

use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// Per-tenant maintenance configuration.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TenantConfig {
    pub tenant_id: String,
    pub auto_create_on_due: bool,
    pub approvals_required: bool,
}

impl TenantConfig {
    /// Default config when no row exists for a tenant.
    pub fn default_for(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            auto_create_on_due: false,
            approvals_required: false,
        }
    }
}

pub struct TenantConfigRepo;

impl TenantConfigRepo {
    /// Get tenant config, returning defaults if no row exists.
    pub async fn get_or_default(pool: &PgPool, tenant_id: &str) -> Result<TenantConfig, sqlx::Error> {
        let row = sqlx::query_as::<_, TenantConfig>(
            "SELECT tenant_id, auto_create_on_due, approvals_required FROM maintenance_tenant_config WHERE tenant_id = $1",
        )
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?;

        Ok(row.unwrap_or_else(|| TenantConfig::default_for(tenant_id)))
    }

    /// Get tenant config within an existing transaction.
    pub async fn get_or_default_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: &str,
    ) -> Result<TenantConfig, sqlx::Error> {
        let row = sqlx::query_as::<_, TenantConfig>(
            "SELECT tenant_id, auto_create_on_due, approvals_required FROM maintenance_tenant_config WHERE tenant_id = $1",
        )
        .bind(tenant_id)
        .fetch_optional(&mut **tx)
        .await?;

        Ok(row.unwrap_or_else(|| TenantConfig::default_for(tenant_id)))
    }

    /// Upsert tenant configuration.
    pub async fn upsert(
        pool: &PgPool,
        tenant_id: &str,
        auto_create_on_due: bool,
        approvals_required: bool,
    ) -> Result<TenantConfig, sqlx::Error> {
        sqlx::query_as::<_, TenantConfig>(
            r#"
            INSERT INTO maintenance_tenant_config (tenant_id, auto_create_on_due, approvals_required)
            VALUES ($1, $2, $3)
            ON CONFLICT (tenant_id) DO UPDATE SET
                auto_create_on_due = $2,
                approvals_required = $3,
                updated_at = NOW()
            RETURNING tenant_id, auto_create_on_due, approvals_required
            "#,
        )
        .bind(tenant_id)
        .bind(auto_create_on_due)
        .bind(approvals_required)
        .fetch_one(pool)
        .await
    }
}
