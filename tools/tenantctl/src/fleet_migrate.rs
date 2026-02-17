//! Fleet migration runner - migrate N tenants with bounded parallelism
//!
//! This module provides deterministic multi-tenant database migration
//! with clear logging, failure reporting, and resumability.

use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use sqlx::{Connection, PgConnection};
use std::sync::Arc;
use tenant_registry::{ModuleSchemaVersions, TenantId};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

/// Module configuration for migrations
#[derive(Debug, Clone)]
struct ModuleConfig {
    name: &'static str,
    postgres_host: &'static str,
    postgres_port: u16,
    postgres_user: &'static str,
    postgres_password: &'static str,
    migrations_path: &'static str,
}

/// Standard module configurations
const MODULES: &[ModuleConfig] = &[
    ModuleConfig {
        name: "ar",
        postgres_host: "localhost",
        postgres_port: 5434,
        postgres_user: "ar_user",
        postgres_password: "ar_pass",
        migrations_path: "./modules/ar/db/migrations",
    },
    ModuleConfig {
        name: "payments",
        postgres_host: "localhost",
        postgres_port: 5436,
        postgres_user: "payments_user",
        postgres_password: "payments_pass",
        migrations_path: "./modules/payments/db/migrations",
    },
    ModuleConfig {
        name: "subscriptions",
        postgres_host: "localhost",
        postgres_port: 5435,
        postgres_user: "subscriptions_user",
        postgres_password: "subscriptions_pass",
        migrations_path: "./modules/subscriptions/db/migrations",
    },
    ModuleConfig {
        name: "gl",
        postgres_host: "localhost",
        postgres_port: 5438,
        postgres_user: "gl_user",
        postgres_password: "gl_pass",
        migrations_path: "./modules/gl/db/migrations",
    },
    ModuleConfig {
        name: "notifications",
        postgres_host: "localhost",
        postgres_port: 5437,
        postgres_user: "notifications_user",
        postgres_password: "notifications_pass",
        migrations_path: "./modules/notifications/db/migrations",
    },
];

/// Migration result for a single tenant
#[derive(Debug)]
pub struct TenantMigrationResult {
    pub tenant_id: TenantId,
    pub success: bool,
    pub module_versions: ModuleSchemaVersions,
    pub error_message: Option<String>,
}

/// Fleet migration results tracker
type ResultsTracker = Arc<Mutex<Vec<TenantMigrationResult>>>;

/// Run fleet migration for N tenants with bounded parallelism
pub async fn run_fleet_migration(num_tenants: usize, parallelism: usize) -> Result<()> {
    info!(
        "Starting fleet migration: {} tenants, parallelism={}",
        num_tenants, parallelism
    );

    // Generate N tenant IDs deterministically
    let tenant_ids: Vec<TenantId> = (0..num_tenants).map(|_| TenantId::new()).collect();

    // Track results
    let results: ResultsTracker = Arc::new(Mutex::new(Vec::new()));

    // Process tenants with bounded parallelism
    stream::iter(tenant_ids)
        .map(|tenant_id| {
            let results = Arc::clone(&results);
            async move { migrate_single_tenant(tenant_id, results).await }
        })
        .buffer_unordered(parallelism)
        .collect::<Vec<_>>()
        .await;

    // Report results
    let final_results = results.lock().await;
    report_migration_results(&final_results);

    Ok(())
}

/// Migrate a single tenant across all modules
async fn migrate_single_tenant(tenant_id: TenantId, results: ResultsTracker) -> Result<()> {
    info!("Migrating tenant: {}", tenant_id);

    let mut module_versions = ModuleSchemaVersions::new();
    let mut overall_success = true;
    let mut error_messages = Vec::new();

    for module in MODULES {
        match migrate_tenant_module(tenant_id, module).await {
            Ok(version) => {
                module_versions.insert(module.name.to_string(), version);
                info!(
                    "✓ Tenant {} - {} migration complete",
                    tenant_id, module.name
                );
            }
            Err(e) => {
                overall_success = false;
                let error_msg = format!("{}: {}", module.name, e);
                error_messages.push(error_msg.clone());
                error!("✗ Tenant {} - {} migration failed: {}", tenant_id, module.name, e);
            }
        }
    }

    // Record result
    let result = TenantMigrationResult {
        tenant_id,
        success: overall_success,
        module_versions,
        error_message: if error_messages.is_empty() {
            None
        } else {
            Some(error_messages.join("; "))
        },
    };

    results.lock().await.push(result);

    Ok(())
}

/// Migrate a single tenant's database for a specific module
async fn migrate_tenant_module(tenant_id: TenantId, module: &ModuleConfig) -> Result<String> {
    // Generate tenant-specific database name
    let tenant_db_name = format!("tenant_{}_{}_db", tenant_id, module.name);

    // Connect to PostgreSQL instance (using default postgres DB)
    let base_url = format!(
        "postgres://{}:{}@{}:{}/postgres",
        module.postgres_user, module.postgres_password, module.postgres_host, module.postgres_port
    );

    let mut conn = PgConnection::connect(&base_url)
        .await
        .context("Failed to connect to PostgreSQL")?;

    // Check if database exists, create if not
    let db_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)"
    )
    .bind(&tenant_db_name)
    .fetch_one(&mut conn)
    .await
    .context("Failed to check if database exists")?;

    if !db_exists {
        // Create database
        sqlx::query(&format!("CREATE DATABASE \"{}\"", tenant_db_name))
            .execute(&mut conn)
            .await
            .context("Failed to create database")?;

        info!("Created database: {}", tenant_db_name);
    } else {
        info!("Database already exists: {}", tenant_db_name);
    }

    // Close connection to postgres DB
    conn.close().await?;

    // Connect to tenant database and run migrations
    let tenant_url = format!(
        "postgres://{}:{}@{}:{}/{}",
        module.postgres_user,
        module.postgres_password,
        module.postgres_host,
        module.postgres_port,
        tenant_db_name
    );

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&tenant_url)
        .await
        .context("Failed to connect to tenant database")?;

    // Run migrations using sqlx::migrate! macro would require compile-time paths
    // For now, we'll use a runtime migrator
    let migrator = sqlx::migrate::Migrator::new(std::path::Path::new(module.migrations_path))
        .await
        .context("Failed to create migrator")?;

    migrator
        .run(&pool)
        .await
        .context("Failed to run migrations")?;

    // Get the latest migration version applied
    let version = get_latest_migration_version(&pool).await?;

    pool.close().await;

    Ok(version)
}

/// Get the latest migration version from _sqlx_migrations table
async fn get_latest_migration_version(pool: &sqlx::PgPool) -> Result<String> {
    // Check if migrations table exists
    let table_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT FROM information_schema.tables
            WHERE table_schema = 'public'
            AND table_name = '_sqlx_migrations'
        )"
    )
    .fetch_one(pool)
    .await
    .context("Failed to check migrations table")?;

    if !table_exists {
        return Ok("none".to_string());
    }

    // Get latest version
    let version: Option<i64> = sqlx::query_scalar(
        "SELECT version FROM _sqlx_migrations ORDER BY version DESC LIMIT 1"
    )
    .fetch_optional(pool)
    .await
    .context("Failed to query migration version")?;

    Ok(version.map(|v| v.to_string()).unwrap_or_else(|| "none".to_string()))
}

/// Report migration results in a structured format
fn report_migration_results(results: &[TenantMigrationResult]) {
    let total = results.len();
    let successful = results.iter().filter(|r| r.success).count();
    let failed = total - successful;

    info!("\n{}", "=".repeat(60));
    info!("FLEET MIGRATION SUMMARY");
    info!("{}", "=".repeat(60));
    info!("Total Tenants:     {}", total);
    info!("Successful:        {} ({:.1}%)", successful, (successful as f64 / total as f64) * 100.0);
    info!("Failed:            {} ({:.1}%)", failed, (failed as f64 / total as f64) * 100.0);
    info!("{}\n", "=".repeat(60));

    if failed > 0 {
        warn!("Failed tenants:");
        for result in results.iter().filter(|r| !r.success) {
            warn!(
                "  - {}: {}",
                result.tenant_id,
                result.error_message.as_ref().unwrap_or(&"Unknown error".to_string())
            );
        }
    }

    info!("Schema versions recorded in registry (placeholder - real DB integration pending)");
    for result in results.iter().filter(|r| r.success).take(3) {
        info!("  Tenant {}: {:?}", result.tenant_id, result.module_versions);
    }
    if successful > 3 {
        info!("  ... and {} more tenants", successful - 3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_configurations() {
        assert_eq!(MODULES.len(), 5);
        assert!(MODULES.iter().any(|m| m.name == "ar"));
        assert!(MODULES.iter().any(|m| m.name == "payments"));
        assert!(MODULES.iter().any(|m| m.name == "subscriptions"));
        assert!(MODULES.iter().any(|m| m.name == "gl"));
        assert!(MODULES.iter().any(|m| m.name == "notifications"));
    }

    #[test]
    fn test_module_ports_unique() {
        let mut ports = MODULES.iter().map(|m| m.postgres_port).collect::<Vec<_>>();
        ports.sort();
        ports.dedup();
        assert_eq!(ports.len(), MODULES.len(), "Module ports must be unique");
    }
}
