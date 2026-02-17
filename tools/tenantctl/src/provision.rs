//! Tenant provisioning logic
//!
//! Creates tenant records, initializes module databases, runs migrations,
//! and transitions tenant to active status with deterministic outcomes.

use anyhow::{Context, Result};
use sqlx::{Connection, PgConnection};
use tenant_registry::{ModuleSchemaVersions, TenantId};
use tracing::{info, warn};

/// Module configuration for provisioning
#[derive(Debug, Clone)]
pub struct ModuleConfig {
    pub name: &'static str,
    pub postgres_host: &'static str,
    pub postgres_port: u16,
    pub postgres_user: &'static str,
    pub postgres_password: &'static str,
    pub migrations_path: &'static str,
    pub http_port: u16,
}

/// Standard module configurations
pub const MODULES: &[ModuleConfig] = &[
    ModuleConfig {
        name: "ar",
        postgres_host: "localhost",
        postgres_port: 5434,
        postgres_user: "ar_user",
        postgres_password: "ar_pass",
        migrations_path: "./modules/ar/db/migrations",
        http_port: 8086,
    },
    ModuleConfig {
        name: "payments",
        postgres_host: "localhost",
        postgres_port: 5436,
        postgres_user: "payments_user",
        postgres_password: "payments_pass",
        migrations_path: "./modules/payments/db/migrations",
        http_port: 8088,
    },
    ModuleConfig {
        name: "subscriptions",
        postgres_host: "localhost",
        postgres_port: 5435,
        postgres_user: "subscriptions_user",
        postgres_password: "subscriptions_pass",
        migrations_path: "./modules/subscriptions/migrations",
        http_port: 8087,
    },
    ModuleConfig {
        name: "gl",
        postgres_host: "localhost",
        postgres_port: 5438,
        postgres_user: "gl_user",
        postgres_password: "gl_pass",
        migrations_path: "./modules/gl/db/migrations",
        http_port: 8090,
    },
    ModuleConfig {
        name: "notifications",
        postgres_host: "localhost",
        postgres_port: 5437,
        postgres_user: "notifications_user",
        postgres_password: "notifications_pass",
        migrations_path: "./modules/notifications/db/migrations",
        http_port: 8089,
    },
];

/// Result of tenant provisioning operation
pub struct ProvisioningResult {
    pub tenant_id: TenantId,
    pub success: bool,
    #[allow(dead_code)] // Used for future registry integration
    pub module_versions: ModuleSchemaVersions,
    pub error_message: Option<String>,
}

/// Create a new tenant: initialize databases and run migrations
pub async fn create_tenant(tenant_id: &str) -> Result<ProvisioningResult> {
    info!("Creating tenant: {}", tenant_id);

    // Parse tenant ID (for now, just create a new one based on the input)
    let tid = if tenant_id.len() == 36 {
        // Try to parse as UUID
        let uuid = tenant_id.parse::<uuid::Uuid>()
            .context("Failed to parse tenant ID as UUID")?;
        TenantId::from_uuid(uuid)
    } else {
        // For short names like "t1", create a deterministic UUID from the string
        // This ensures same input always produces same tenant ID
        let namespace = uuid::Uuid::NAMESPACE_DNS;
        let uuid = uuid::Uuid::new_v5(&namespace, tenant_id.as_bytes());
        TenantId::from_uuid(uuid)
    };

    info!("Tenant ID resolved to: {}", tid);

    let mut module_versions = ModuleSchemaVersions::new();
    let mut overall_success = true;
    let mut error_messages = Vec::new();

    // Provision each module database
    for module in MODULES {
        match provision_tenant_module(tid, module).await {
            Ok(version) => {
                module_versions.insert(module.name.to_string(), version);
                info!("✓ Tenant {} - {} provisioned successfully", tid, module.name);
            }
            Err(e) => {
                overall_success = false;
                let error_msg = format!("{}: {}", module.name, e);
                error_messages.push(error_msg.clone());
                warn!("✗ Tenant {} - {} provisioning failed: {}", tid, module.name, e);
            }
        }
    }

    let result = ProvisioningResult {
        tenant_id: tid,
        success: overall_success,
        module_versions,
        error_message: if error_messages.is_empty() {
            None
        } else {
            Some(error_messages.join("; "))
        },
    };

    if result.success {
        info!("✅ Tenant {} created successfully", tid);
    } else {
        warn!("⚠️  Tenant {} creation completed with errors", tid);
    }

    Ok(result)
}

/// Provision a single module's database for a tenant
async fn provision_tenant_module(tenant_id: TenantId, module: &ModuleConfig) -> Result<String> {
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

    // Check if database exists
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

        info!("  Created database: {}", tenant_db_name);
    } else {
        info!("  Database already exists: {}", tenant_db_name);
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

    // Run migrations
    let migrator = sqlx::migrate::Migrator::new(std::path::Path::new(module.migrations_path))
        .await
        .context("Failed to create migrator")?;

    migrator
        .run(&pool)
        .await
        .context(format!("Failed to run migrations for {} module", module.name))?;

    info!("  Migrations applied to {}", tenant_db_name);

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

/// Activate a tenant (transition from provisioning to active)
pub async fn activate_tenant(tenant_id: &str) -> Result<()> {
    info!("Activating tenant: {}", tenant_id);

    // For now, this is a placeholder - in full implementation this would:
    // 1. Update tenant registry status to Active
    // 2. Enable access controls
    // 3. Initialize any runtime state

    info!("✅ Tenant {} activated (registry update pending full implementation)", tenant_id);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_configurations_complete() {
        assert_eq!(MODULES.len(), 5);
        assert!(MODULES.iter().any(|m| m.name == "ar"));
        assert!(MODULES.iter().any(|m| m.name == "payments"));
        assert!(MODULES.iter().any(|m| m.name == "subscriptions"));
        assert!(MODULES.iter().any(|m| m.name == "gl"));
        assert!(MODULES.iter().any(|m| m.name == "notifications"));
    }

    #[test]
    fn module_ports_unique() {
        let mut postgres_ports = MODULES.iter().map(|m| m.postgres_port).collect::<Vec<_>>();
        postgres_ports.sort();
        postgres_ports.dedup();
        assert_eq!(postgres_ports.len(), MODULES.len(), "PostgreSQL ports must be unique");

        let mut http_ports = MODULES.iter().map(|m| m.http_port).collect::<Vec<_>>();
        http_ports.sort();
        http_ports.dedup();
        assert_eq!(http_ports.len(), MODULES.len(), "HTTP ports must be unique");
    }

    #[test]
    fn deterministic_tenant_id_from_string() {
        // Same input should produce same UUID
        let namespace = uuid::Uuid::NAMESPACE_DNS;
        let uuid1 = uuid::Uuid::new_v5(&namespace, "t1".as_bytes());
        let uuid2 = uuid::Uuid::new_v5(&namespace, "t1".as_bytes());
        assert_eq!(uuid1, uuid2);

        // Different inputs should produce different UUIDs
        let uuid3 = uuid::Uuid::new_v5(&namespace, "t2".as_bytes());
        assert_ne!(uuid1, uuid3);
    }
}
