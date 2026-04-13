//! Async bundle worker — per-module provisioning with independent status tracking.
//!
//! Provisions each module in a tenant's bundle independently:
//! create database → run migrations → seed initial data → verify connectivity.
//!
//! Unlike the 7-step tenant-level sequence, this worker tracks success/failure
//! at the module level so that a failing module does not block others.
//! Per-module status is stored in `cp_tenant_module_status`.

use chrono::Utc;
use serde_json::json;
use sqlx::{Connection, PgConnection, PgPool};
use uuid::Uuid;

use super::registry::ModuleRegistry;
use super::steps::{tenant_db_name, StepError};
use tenant_registry::event_types;

// ============================================================================
// Public types
// ============================================================================

/// Outcome of provisioning a single module.
#[derive(Debug, Clone)]
pub struct ModuleResult {
    pub module_code: String,
    pub success: bool,
    pub error: Option<String>,
}

/// Summary returned by `provision_all_modules`.
pub struct AllModulesResult {
    pub results: Vec<ModuleResult>,
    pub ready_count: usize,
    pub failed_count: usize,
}

impl AllModulesResult {
    pub fn all_ready(&self) -> bool {
        self.failed_count == 0 && !self.results.is_empty()
    }
}

// ============================================================================
// Status table helpers
// ============================================================================

/// Seed `pending` rows for each module. Idempotent via ON CONFLICT DO NOTHING.
pub async fn seed_module_statuses(
    pool: &PgPool,
    tenant_id: Uuid,
    module_codes: &[String],
) -> Result<(), sqlx::Error> {
    let now = Utc::now();
    for code in module_codes {
        sqlx::query(
            "INSERT INTO cp_tenant_module_status \
             (tenant_id, module_code, status, created_at, updated_at) \
             VALUES ($1, $2, 'pending', $3, $3) \
             ON CONFLICT (tenant_id, module_code) DO NOTHING",
        )
        .bind(tenant_id)
        .bind(code)
        .bind(now)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Read all per-module statuses for a tenant.
pub async fn module_statuses(
    pool: &PgPool,
    tenant_id: Uuid,
) -> Result<Vec<(String, String)>, sqlx::Error> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT module_code, status FROM cp_tenant_module_status \
         WHERE tenant_id = $1 ORDER BY module_code",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

async fn set_status(
    pool: &PgPool,
    tenant_id: Uuid,
    module_code: &str,
    status: &str,
    error_msg: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE cp_tenant_module_status \
         SET status = $1, error_msg = $2, updated_at = $3 \
         WHERE tenant_id = $4 AND module_code = $5",
    )
    .bind(status)
    .bind(error_msg)
    .bind(Utc::now())
    .bind(tenant_id)
    .bind(module_code)
    .execute(pool)
    .await?;
    Ok(())
}

// ============================================================================
// Main entry point
// ============================================================================

/// Provision every module in the bundle independently.
///
/// Each module goes through: create DB → run migrations → seed → verify.
/// Failures in one module do not stop other modules from being provisioned.
/// Returns a summary of which modules succeeded and which failed.
pub async fn provision_all_modules(
    pool: &PgPool,
    registry: &ModuleRegistry,
    tenant_id: Uuid,
    module_codes: &[String],
) -> Result<AllModulesResult, sqlx::Error> {
    // Seed pending rows (idempotent)
    seed_module_statuses(pool, tenant_id, module_codes).await?;

    let mut results = Vec::with_capacity(module_codes.len());

    for code in module_codes {
        // Skip if already ready (idempotent on retry)
        let current: Option<(String,)> = sqlx::query_as(
            "SELECT status FROM cp_tenant_module_status \
             WHERE tenant_id = $1 AND module_code = $2",
        )
        .bind(tenant_id)
        .bind(code)
        .fetch_optional(pool)
        .await?;

        if matches!(current.as_ref().map(|s| s.0.as_str()), Some("ready")) {
            tracing::info!(
                tenant_id = %tenant_id,
                module = %code,
                "module already ready — skipping"
            );
            results.push(ModuleResult {
                module_code: code.clone(),
                success: true,
                error: None,
            });
            continue;
        }

        // Transition to provisioning
        set_status(pool, tenant_id, code, "provisioning", None)
            .await
            .ok();

        tracing::info!(tenant_id = %tenant_id, module = %code, "provisioning module");

        match provision_one_module(pool, registry, tenant_id, code).await {
            Ok(()) => {
                set_status(pool, tenant_id, code, "ready", None)
                    .await
                    .ok();
                tracing::info!(tenant_id = %tenant_id, module = %code, "module ready");
                results.push(ModuleResult {
                    module_code: code.clone(),
                    success: true,
                    error: None,
                });
            }
            Err(err) => {
                let err_str = err.to_string();
                set_status(pool, tenant_id, code, "failed", Some(&err_str))
                    .await
                    .ok();
                tracing::error!(
                    tenant_id = %tenant_id,
                    module = %code,
                    error = %err_str,
                    "module provisioning failed"
                );
                results.push(ModuleResult {
                    module_code: code.clone(),
                    success: false,
                    error: Some(err_str),
                });
            }
        }
    }

    let ready_count = results.iter().filter(|r| r.success).count();
    let failed_count = results.iter().filter(|r| !r.success).count();

    Ok(AllModulesResult {
        results,
        ready_count,
        failed_count,
    })
}

// ============================================================================
// Single-module provisioning
// ============================================================================

async fn provision_one_module(
    pool: &PgPool,
    registry: &ModuleRegistry,
    tenant_id: Uuid,
    module_code: &str,
) -> Result<(), StepError> {
    let config = registry
        .get(module_code)
        .ok_or_else(|| StepError::ModuleNotFound(module_code.to_string()))?;

    let db_name = tenant_db_name(tenant_id, module_code);

    // Step A: create database
    create_module_db(config, &db_name).await?;

    // Step B: run migrations
    let db_url = config.tenant_db_url(&db_name);
    run_module_migrations(&db_url, config, &db_name).await?;

    // Step C: seed initial data
    seed_module(pool, tenant_id, module_code, &db_url).await?;

    // Step D: verify connectivity
    verify_module_connectivity(&db_url, module_code).await?;

    Ok(())
}

async fn create_module_db(
    config: &super::registry::ModuleProvisioningConfig,
    db_name: &str,
) -> Result<(), StepError> {
    let mut conn = PgConnection::connect(&config.admin_url())
        .await
        .map_err(|e| StepError::Migration(format!("connect admin for {}: {e}", config.module_code)))?;

    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(db_name)
            .fetch_one(&mut conn)
            .await?;

    if !exists {
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

async fn run_module_migrations(
    db_url: &str,
    config: &super::registry::ModuleProvisioningConfig,
    db_name: &str,
) -> Result<(), StepError> {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(db_url)
        .await
        .map_err(|e| StepError::Migration(format!("connect to {db_name}: {e}")))?;

    let migrator = sqlx::migrate::Migrator::new(config.migrations_path.as_path())
        .await
        .map_err(|e| {
            StepError::Migration(format!(
                "load migrations for {} from {}: {e}",
                config.module_code,
                config.migrations_path.display()
            ))
        })?;

    migrator
        .run(&pool)
        .await
        .map_err(|e| StepError::Migration(format!("run migrations for {}: {e}", config.module_code)))?;

    pool.close().await;
    tracing::info!(module = %config.module_code, db = %db_name, "migrations applied");
    Ok(())
}

async fn seed_module(
    pool: &PgPool,
    tenant_id: Uuid,
    module_code: &str,
    db_url: &str,
) -> Result<(), StepError> {
    let module_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(db_url)
        .await
        .map_err(|e| StepError::Migration(format!("connect for seed {module_code}: {e}")))?;

    let result = match module_code {
        "gl" => tenant_registry::seed_gl_module(&module_pool, tenant_id)
            .await
            .map_err(|e| StepError::Migration(format!("seed gl: {e}"))),
        "ar" => tenant_registry::seed_ar_module(&module_pool, tenant_id)
            .await
            .map_err(|e| StepError::Migration(format!("seed ar: {e}"))),
        "subscriptions" => tenant_registry::seed_subscriptions_module(&module_pool, tenant_id)
            .await
            .map_err(|e| StepError::Migration(format!("seed subscriptions: {e}"))),
        _ => {
            tracing::debug!(module = %module_code, "no seed logic — skipping");
            Ok(())
        }
    };

    module_pool.close().await;

    // Write seed event to outbox
    if result.is_ok() {
        let payload = json!({
            "tenant_id": tenant_id.to_string(),
            "module_code": module_code,
        });
        if let Err(e) = sqlx::query(
            "INSERT INTO provisioning_outbox (tenant_id, event_type, payload, created_at) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(tenant_id)
        .bind(event_types::TENANT_PROVISIONING_STARTED)
        .bind(&payload)
        .bind(Utc::now())
        .execute(pool)
        .await
        {
            tracing::warn!(module = %module_code, error = %e, "failed to write module seed event (non-fatal)");
        }
    }

    result
}

async fn verify_module_connectivity(db_url: &str, module_code: &str) -> Result<(), StepError> {
    let mut conn = PgConnection::connect(db_url)
        .await
        .map_err(|e| StepError::Migration(format!("connectivity check connect {module_code}: {e}")))?;

    sqlx::query("SELECT 1")
        .execute(&mut conn)
        .await
        .map_err(|e| StepError::Migration(format!("connectivity ping failed for {module_code}: {e}")))?;

    conn.close().await.ok();
    Ok(())
}
