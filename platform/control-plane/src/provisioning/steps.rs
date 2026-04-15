//! Provisioning step execution
//!
//! Implements each of the 7 steps in the tenant provisioning sequence.
//! Step status tracking lives in the sibling `tracking` module.

use chrono::Utc;
use futures::future::join_all;
use serde_json::json;
use sqlx::{Connection, PgConnection, PgPool};
use std::collections::HashMap;
use std::time::Duration;
use tenant_registry::event_types;
use tokio::time::{sleep, timeout_at, Instant};
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
    let row: Option<(String,)> = sqlx::query_as("SELECT status FROM tenants WHERE tenant_id = $1")
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

/// Publish `tenant.provisioned` to the outbox, then poll every module's
/// `/api/ready?tenant_id=` endpoint until all respond `up` or the deadline
/// expires.
///
/// Returns `Ok` in both cases (active and degraded) — the tenant status is
/// written inside this function. The caller can inspect `checks["status"]` to
/// distinguish the two outcomes.
pub async fn activate_tenant(
    pool: &PgPool,
    tenant_id: Uuid,
    module_codes: &[String],
    registry: &ModuleRegistry,
    http_client: &reqwest::Client,
    ready_timeout: Duration,
) -> Result<StepOutcome, StepError> {
    // Verify tenant is in provisioning state
    let rows =
        sqlx::query("SELECT 1 FROM tenants WHERE tenant_id = $1 AND status = 'provisioning'")
            .bind(tenant_id)
            .fetch_optional(pool)
            .await?;

    if rows.is_none() {
        return Err(StepError::InvalidState(format!(
            "tenant {tenant_id} not in provisioning status"
        )));
    }

    // Write tenant.provisioned event first — modules need it to init tenant state
    let now = Utc::now();
    let outbox_payload = json!({
        "tenant_id": tenant_id.to_string(),
        "activated_at": now.to_rfc3339(),
    });
    sqlx::query(
        "INSERT INTO provisioning_outbox (tenant_id, event_type, payload, created_at) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(tenant_id)
    .bind(event_types::TENANT_PROVISIONED)
    .bind(&outbox_payload)
    .bind(now)
    .execute(pool)
    .await?;

    tracing::info!(tenant_id = %tenant_id, "tenant.provisioned written to outbox, polling modules");

    // Poll all modules concurrently until ready or timeout
    let failed_modules = poll_module_readiness(
        tenant_id,
        module_codes,
        registry,
        http_client,
        ready_timeout,
    )
    .await;

    let (new_status, checks) = if failed_modules.is_empty() {
        (
            "active",
            json!({
                "status": "active",
                "modules_ready": module_codes,
            }),
        )
    } else {
        // Upsert failed module statuses into cp_tenant_module_status.
        // INSERT … ON CONFLICT handles the case where no row exists yet.
        for code in &failed_modules {
            let _ = sqlx::query(
                "INSERT INTO cp_tenant_module_status \
                     (tenant_id, module_code, status, created_at, updated_at) \
                 VALUES ($1, $2, 'failed', $3, $3) \
                 ON CONFLICT (tenant_id, module_code) DO UPDATE \
                 SET status = 'failed', updated_at = EXCLUDED.updated_at",
            )
            .bind(tenant_id)
            .bind(code)
            .bind(Utc::now())
            .execute(pool)
            .await;
        }

        tracing::warn!(
            tenant_id = %tenant_id,
            failed = ?failed_modules,
            "some modules did not become ready — tenant degraded"
        );

        (
            "degraded",
            json!({
                "status": "degraded",
                "failed_modules": failed_modules,
            }),
        )
    };

    sqlx::query("UPDATE tenants SET status = $1, updated_at = $2 WHERE tenant_id = $3")
        .bind(new_status)
        .bind(Utc::now())
        .bind(tenant_id)
        .execute(pool)
        .await?;

    Ok(StepOutcome { checks })
}

/// Poll all module readiness endpoints concurrently.
///
/// Returns the list of module codes that did NOT return `up` within the
/// deadline. An empty vec means all modules are ready.
pub async fn poll_module_readiness(
    tenant_id: Uuid,
    module_codes: &[String],
    registry: &ModuleRegistry,
    http_client: &reqwest::Client,
    ready_timeout: Duration,
) -> Vec<String> {
    if module_codes.is_empty() {
        return vec![];
    }

    let deadline = Instant::now() + ready_timeout;

    let futures: Vec<_> = module_codes
        .iter()
        .map(|code| {
            let url = registry
                .get(code)
                .map(|cfg| format!("{}/api/ready?tenant_id={}", cfg.http_base_url, tenant_id));
            let code = code.clone();
            let client = http_client.clone();
            async move {
                match url {
                    None => (code, false), // unknown module — treat as failed
                    Some(url) => poll_single_module(code, url, client, deadline).await,
                }
            }
        })
        .collect();

    let results = join_all(futures).await;

    results
        .into_iter()
        .filter_map(|(code, ok)| if ok { None } else { Some(code) })
        .collect()
}

/// Poll a single module's `/api/ready?tenant_id=` endpoint with exponential
/// backoff until it returns `{ tenant: { status: "up" } }` or the deadline
/// passes.
async fn poll_single_module(
    code: String,
    url: String,
    client: reqwest::Client,
    deadline: Instant,
) -> (String, bool) {
    let mut backoff_ms: u64 = 200;

    loop {
        // Check deadline before each attempt
        if Instant::now() >= deadline {
            tracing::warn!(module = %code, "ready poll timed out");
            return (code, false);
        }

        // Attempt the request with deadline as timeout
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return (code, false);
        }

        let result = timeout_at(deadline, Box::pin(client.get(&url).send())).await;

        match result {
            Ok(Ok(resp)) if resp.status().is_success() => {
                // Parse { tenant: { status: "up" } }
                match resp.json::<serde_json::Value>().await {
                    Ok(body) => {
                        let status = body
                            .get("tenant")
                            .and_then(|t| t.get("status"))
                            .and_then(|s| s.as_str())
                            .unwrap_or("");
                        if status == "up" {
                            tracing::debug!(module = %code, "module ready");
                            return (code, true);
                        }
                        tracing::debug!(module = %code, tenant_status = %status, "module not yet ready");
                    }
                    Err(e) => {
                        tracing::debug!(module = %code, error = %e, "failed to parse ready response");
                    }
                }
            }
            Ok(Ok(resp)) => {
                tracing::debug!(module = %code, status = %resp.status(), "module ready endpoint returned non-success");
            }
            Ok(Err(e)) => {
                tracing::debug!(module = %code, error = %e, "module ready request failed");
            }
            Err(_elapsed) => {
                tracing::warn!(module = %code, "ready poll deadline exceeded");
                return (code, false);
            }
        }

        // Exponential backoff, capped at 10 seconds
        let sleep_ms = backoff_ms.min(10_000);
        let time_left = deadline.saturating_duration_since(Instant::now());
        if time_left.is_zero() {
            return (code, false);
        }
        let actual_sleep = Duration::from_millis(sleep_ms).min(time_left);
        sleep(actual_sleep).await;
        backoff_ms = (backoff_ms * 2).min(10_000);
    }
}
