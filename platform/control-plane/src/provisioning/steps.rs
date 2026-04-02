//! Provisioning step execution
//!
//! Implements each of the 7 steps in the tenant provisioning sequence.
//! Step status tracking lives in the sibling `tracking` module.

use chrono::Utc;
use serde_json::json;
use sqlx::{Connection, PgConnection, PgPool};
use std::collections::HashMap;
use tenant_registry::event_types;
use uuid::Uuid;

use super::registry::{ModuleProvisioningConfig, ModuleRegistry};

/// Errors from provisioning steps
#[derive(Debug, thiserror::Error)]
pub enum StepError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migration(String),

    #[error("tenant not found or wrong status: {0}")]
    InvalidState(String),

    #[error("module not in registry: {0}")]
    ModuleNotFound(String),
}

/// Outcome of running a single step
pub struct StepOutcome {
    pub checks: serde_json::Value,
}

/// Build the database name for a tenant-module pair.
/// Replaces hyphens in the UUID with underscores.
pub fn tenant_db_name(tenant_id: Uuid, module_code: &str) -> String {
    let sanitized = tenant_id.to_string().replace('-', "_");
    format!("tenant_{sanitized}_{module_code}_db")
}

/// Resolve which module codes are in this tenant's bundle.
pub async fn resolve_bundle_modules(
    pool: &PgPool,
    tenant_id: Uuid,
) -> Result<Vec<String>, StepError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT bm.module_code \
         FROM cp_tenant_bundle tb \
         JOIN cp_bundle_modules bm ON bm.bundle_id = tb.bundle_id \
         WHERE tb.tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|(code,)| code).collect())
}

// ============================================================================
// Step 1 — Validate tenant
// ============================================================================

pub async fn validate_tenant(pool: &PgPool, tenant_id: Uuid) -> Result<StepOutcome, StepError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT status FROM tenants WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_optional(pool)
            .await?;

    match row {
        Some((status,)) if status == "pending" || status == "provisioning" => Ok(StepOutcome {
            checks: json!({"tenant_exists": true, "status": status}),
        }),
        Some((status,)) => Err(StepError::InvalidState(format!(
            "tenant {tenant_id} in unexpected status: {status}"
        ))),
        None => Err(StepError::InvalidState(format!(
            "tenant {tenant_id} not found"
        ))),
    }
}

// ============================================================================
// Step 2 — Create tenant databases
// ============================================================================

pub async fn create_tenant_databases(
    tenant_id: Uuid,
    module_codes: &[String],
    registry: &ModuleRegistry,
) -> Result<StepOutcome, StepError> {
    let mut created = Vec::new();

    for code in module_codes {
        let config = registry
            .get(code)
            .ok_or_else(|| StepError::ModuleNotFound(code.clone()))?;

        let db_name = tenant_db_name(tenant_id, code);
        create_single_database(config, &db_name).await?;
        created.push(code.clone());
    }

    Ok(StepOutcome {
        checks: json!({"databases_created": created}),
    })
}

async fn create_single_database(
    config: &ModuleProvisioningConfig,
    db_name: &str,
) -> Result<(), StepError> {
    let mut conn = PgConnection::connect(&config.admin_url())
        .await
        .map_err(|e| StepError::Migration(format!("connect to {}: {e}", config.module_code)))?;

    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(db_name)
            .fetch_one(&mut conn)
            .await?;

    if !exists {
        // Database names are derived from UUIDs (alphanumeric + underscores only)
        // so quoting is safe without parameterisation risk.
        sqlx::query(&format!("CREATE DATABASE \"{db_name}\""))
            .execute(&mut conn)
            .await
            .map_err(|e| StepError::Migration(format!("CREATE DATABASE {db_name}: {e}")))?;

        tracing::info!(db = %db_name, "created tenant database");
    } else {
        tracing::info!(db = %db_name, "tenant database already exists");
    }

    conn.close().await.ok();
    Ok(())
}

// ============================================================================
// Step 3 — Run schema migrations
// ============================================================================

pub async fn run_schema_migrations(
    tenant_id: Uuid,
    module_codes: &[String],
    registry: &ModuleRegistry,
) -> Result<StepOutcome, StepError> {
    let mut versions: HashMap<String, String> = HashMap::new();

    for code in module_codes {
        let config = registry
            .get(code)
            .ok_or_else(|| StepError::ModuleNotFound(code.clone()))?;

        let db_name = tenant_db_name(tenant_id, code);
        let url = config.tenant_db_url(&db_name);

        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .map_err(|e| StepError::Migration(format!("connect to {db_name}: {e}")))?;

        let migrator = sqlx::migrate::Migrator::new(config.migrations_path.as_path())
            .await
            .map_err(|e| {
                StepError::Migration(format!(
                    "load migrations for {} from {}: {e}",
                    code,
                    config.migrations_path.display()
                ))
            })?;

        migrator
            .run(&pool)
            .await
            .map_err(|e| StepError::Migration(format!("run migrations for {code}: {e}")))?;

        let version = latest_migration_version(&pool).await?;
        versions.insert(code.clone(), version);
        pool.close().await;

        tracing::info!(module = %code, db = %db_name, "migrations applied");
    }

    Ok(StepOutcome {
        checks: json!({"module_versions": versions}),
    })
}

async fn latest_migration_version(pool: &PgPool) -> Result<String, StepError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_name = '_sqlx_migrations')",
    )
    .fetch_one(pool)
    .await?;

    if !exists {
        return Ok("none".to_string());
    }

    let version: Option<i64> =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version DESC LIMIT 1")
            .fetch_optional(pool)
            .await?;

    Ok(version
        .map(|v| v.to_string())
        .unwrap_or_else(|| "none".to_string()))
}

// ============================================================================
// Step 4 — Seed initial data
// ============================================================================

pub async fn seed_initial_data(
    tenant_id: Uuid,
    module_codes: &[String],
    registry: &ModuleRegistry,
) -> Result<StepOutcome, StepError> {
    let mut seeded = Vec::new();

    for code in module_codes {
        let config = registry
            .get(code)
            .ok_or_else(|| StepError::ModuleNotFound(code.clone()))?;

        let db_name = tenant_db_name(tenant_id, code);
        let url = config.tenant_db_url(&db_name);

        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .map_err(|e| StepError::Migration(format!("connect to {db_name}: {e}")))?;

        match code.as_str() {
            "gl" => {
                tenant_registry::seed_gl_module(&pool, tenant_id)
                    .await
                    .map_err(|e| StepError::Migration(format!("seed gl: {e}")))?;
            }
            "ar" => {
                tenant_registry::seed_ar_module(&pool, tenant_id)
                    .await
                    .map_err(|e| StepError::Migration(format!("seed ar: {e}")))?;
            }
            "subscriptions" => {
                tenant_registry::seed_subscriptions_module(&pool, tenant_id)
                    .await
                    .map_err(|e| StepError::Migration(format!("seed subscriptions: {e}")))?;
            }
            _ => {
                tracing::debug!(module = %code, "no seed logic for module — skipping");
            }
        }

        seeded.push(code.clone());
        pool.close().await;
    }

    Ok(StepOutcome {
        checks: json!({"modules_seeded": seeded}),
    })
}

// ============================================================================
// Step 5 — Verify database connectivity
// ============================================================================

pub async fn verify_database_connectivity(
    tenant_id: Uuid,
    module_codes: &[String],
    registry: &ModuleRegistry,
) -> Result<StepOutcome, StepError> {
    let mut results: HashMap<String, bool> = HashMap::new();

    for code in module_codes {
        let config = registry
            .get(code)
            .ok_or_else(|| StepError::ModuleNotFound(code.clone()))?;

        let db_name = tenant_db_name(tenant_id, code);
        let url = config.tenant_db_url(&db_name);

        let ok = match PgConnection::connect(&url).await {
            Ok(mut conn) => {
                let ping = sqlx::query("SELECT 1").execute(&mut conn).await.is_ok();
                conn.close().await.ok();
                ping
            }
            Err(_) => false,
        };

        results.insert(code.clone(), ok);
        if !ok {
            return Err(StepError::Migration(format!(
                "connectivity check failed for {code} ({db_name})"
            )));
        }
    }

    Ok(StepOutcome {
        checks: json!({"connectivity": results}),
    })
}

// ============================================================================
// Step 6 — Verify & record schema versions
// ============================================================================

pub async fn verify_schema_versions(
    registry_pool: &PgPool,
    tenant_id: Uuid,
    module_codes: &[String],
    registry: &ModuleRegistry,
) -> Result<StepOutcome, StepError> {
    let mut versions: HashMap<String, String> = HashMap::new();

    for code in module_codes {
        let config = registry
            .get(code)
            .ok_or_else(|| StepError::ModuleNotFound(code.clone()))?;

        let db_name = tenant_db_name(tenant_id, code);
        let url = config.tenant_db_url(&db_name);

        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .map_err(|e| StepError::Migration(format!("connect to {db_name}: {e}")))?;

        let version = latest_migration_version(&pool).await?;
        versions.insert(code.clone(), version);
        pool.close().await;
    }

    // Write versions to tenants.module_schema_versions
    let versions_json = serde_json::to_value(&versions).unwrap_or_else(|_| json!({}));

    sqlx::query(
        "UPDATE tenants SET module_schema_versions = $1, updated_at = $2 WHERE tenant_id = $3",
    )
    .bind(&versions_json)
    .bind(Utc::now())
    .bind(tenant_id)
    .execute(registry_pool)
    .await?;

    Ok(StepOutcome {
        checks: json!({"schema_versions": versions}),
    })
}

// ============================================================================
// Step 7 — Activate tenant
// ============================================================================

pub async fn activate_tenant(pool: &PgPool, tenant_id: Uuid) -> Result<StepOutcome, StepError> {
    let now = Utc::now();

    let rows = sqlx::query(
        "UPDATE tenants SET status = 'active', updated_at = $1 \
         WHERE tenant_id = $2 AND status = 'provisioning'",
    )
    .bind(now)
    .bind(tenant_id)
    .execute(pool)
    .await?
    .rows_affected();

    if rows == 0 {
        return Err(StepError::InvalidState(format!(
            "tenant {tenant_id} not in provisioning status"
        )));
    }

    // Write tenant.provisioned event to outbox
    let payload = json!({
        "tenant_id": tenant_id.to_string(),
        "activated_at": now.to_rfc3339(),
    });
    sqlx::query(
        "INSERT INTO provisioning_outbox (tenant_id, event_type, payload, created_at) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(tenant_id)
    .bind(event_types::TENANT_PROVISIONED)
    .bind(&payload)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(StepOutcome {
        checks: json!({"status": "active"}),
    })
}
