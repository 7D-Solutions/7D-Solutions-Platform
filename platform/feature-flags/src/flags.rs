//! Core feature-flag lookup and mutation against the database.
//!
//! The `feature_flags` table stores two kinds of rows:
//! - **Global** (`tenant_id IS NULL`): applies to every tenant unless overridden.
//! - **Per-tenant** (`tenant_id IS NOT NULL`): overrides the global value for that tenant.
//!
//! Lookup priority: per-tenant → global → `false` (absent row = disabled).

use sqlx::PgPool;
use uuid::Uuid;

/// Errors returned by flag operations.
#[derive(Debug, thiserror::Error)]
pub enum FlagError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

/// Returns whether a feature flag is enabled for the given tenant.
///
/// Resolution order:
/// 1. Per-tenant row (if `tenant_id` is `Some` and a row exists).
/// 2. Global row (`tenant_id IS NULL`).
/// 3. `false` when no row matches — absent flags are disabled by default.
pub async fn is_enabled(
    pool: &PgPool,
    flag: &str,
    tenant_id: Option<Uuid>,
) -> Result<bool, FlagError> {
    // Per-tenant override takes precedence.
    if let Some(tid) = tenant_id {
        let row: Option<bool> = sqlx::query_scalar(
            "SELECT enabled FROM feature_flags WHERE flag_name = $1 AND tenant_id = $2",
        )
        .bind(flag)
        .bind(tid)
        .fetch_optional(pool)
        .await?;

        if let Some(enabled) = row {
            return Ok(enabled);
        }
    }

    // Fall back to the global flag.
    let global: Option<bool> = sqlx::query_scalar(
        "SELECT enabled FROM feature_flags WHERE flag_name = $1 AND tenant_id IS NULL",
    )
    .bind(flag)
    .fetch_optional(pool)
    .await?;

    Ok(global.unwrap_or(false))
}

/// Insert or update a feature flag.
///
/// Pass `tenant_id = None` to set the global default.
/// Pass `tenant_id = Some(id)` to set a per-tenant override.
pub async fn set_flag(
    pool: &PgPool,
    flag: &str,
    tenant_id: Option<Uuid>,
    enabled: bool,
) -> Result<(), FlagError> {
    if tenant_id.is_none() {
        sqlx::query(
            r#"
            INSERT INTO feature_flags (flag_name, tenant_id, enabled)
            VALUES ($1, NULL, $2)
            ON CONFLICT (flag_name) WHERE tenant_id IS NULL
            DO UPDATE SET enabled = EXCLUDED.enabled, updated_at = now()
            "#,
        )
        .bind(flag)
        .bind(enabled)
        .execute(pool)
        .await?;
    } else {
        sqlx::query(
            r#"
            INSERT INTO feature_flags (flag_name, tenant_id, enabled)
            VALUES ($1, $2, $3)
            ON CONFLICT (flag_name, tenant_id) WHERE tenant_id IS NOT NULL
            DO UPDATE SET enabled = EXCLUDED.enabled, updated_at = now()
            "#,
        )
        .bind(flag)
        .bind(tenant_id)
        .bind(enabled)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Returns all feature flags visible to the given tenant.
///
/// For each flag name, returns the per-tenant row if one exists, otherwise
/// the global default row (`tenant_id IS NULL`). Flags with no DB rows at
/// all are absent from the map — callers should treat absence as disabled.
pub async fn list_flags_for_tenant(
    pool: &PgPool,
    tenant_id: Uuid,
) -> Result<std::collections::HashMap<String, bool>, FlagError> {
    let rows: Vec<(String, bool)> = sqlx::query_as(
        "SELECT DISTINCT ON (flag_name) flag_name, enabled \
         FROM feature_flags \
         WHERE tenant_id = $1 OR tenant_id IS NULL \
         ORDER BY flag_name, (tenant_id IS NOT NULL) DESC",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().collect())
}

/// Remove a feature flag row.
///
/// Pass `tenant_id = None` to delete the global row.
/// Pass `tenant_id = Some(id)` to delete a per-tenant override (the global row remains).
pub async fn delete_flag(
    pool: &PgPool,
    flag: &str,
    tenant_id: Option<Uuid>,
) -> Result<(), FlagError> {
    sqlx::query(
        "DELETE FROM feature_flags WHERE flag_name = $1 AND (tenant_id = $2 OR ($2 IS NULL AND tenant_id IS NULL))",
    )
    .bind(flag)
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}
