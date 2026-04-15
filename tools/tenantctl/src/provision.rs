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
    pub postgres_password: String,
    pub migrations_path: &'static str,
    pub http_port: u16,
}

/// Module definition without credentials (static data only)
struct ModuleDef {
    name: &'static str,
    postgres_host: &'static str,
    postgres_port: u16,
    postgres_user: &'static str,
    password_env_var: &'static str,
    migrations_path: &'static str,
    http_port: u16,
}

const MODULE_DEFS: &[ModuleDef] = &[
    ModuleDef {
        name: "ar",
        postgres_host: "localhost",
        postgres_port: 5434,
        postgres_user: "ar_user",
        password_env_var: "AR_DB_PASSWORD",
        migrations_path: "./modules/ar/db/migrations",
        http_port: 8086,
    },
    ModuleDef {
        name: "payments",
        postgres_host: "localhost",
        postgres_port: 5436,
        postgres_user: "payments_user",
        password_env_var: "PAYMENTS_DB_PASSWORD",
        migrations_path: "./modules/payments/db/migrations",
        http_port: 8088,
    },
    ModuleDef {
        name: "subscriptions",
        postgres_host: "localhost",
        postgres_port: 5435,
        postgres_user: "subscriptions_user",
        password_env_var: "SUBSCRIPTIONS_DB_PASSWORD",
        migrations_path: "./modules/subscriptions/db/migrations",
        http_port: 8087,
    },
    ModuleDef {
        name: "gl",
        postgres_host: "localhost",
        postgres_port: 5438,
        postgres_user: "gl_user",
        password_env_var: "GL_DB_PASSWORD",
        migrations_path: "./modules/gl/db/migrations",
        http_port: 8090,
    },
    ModuleDef {
        name: "notifications",
        postgres_host: "localhost",
        postgres_port: 5437,
        postgres_user: "notifications_user",
        password_env_var: "NOTIFICATIONS_DB_PASSWORD",
        migrations_path: "./modules/notifications/db/migrations",
        http_port: 8089,
    },
];

/// Build module configurations by reading passwords from environment variables.
///
/// Required env vars: AR_DB_PASSWORD, PAYMENTS_DB_PASSWORD,
/// SUBSCRIPTIONS_DB_PASSWORD, GL_DB_PASSWORD, NOTIFICATIONS_DB_PASSWORD.
///
/// Panics with a clear message if any required env var is unset.
pub fn load_modules() -> Vec<ModuleConfig> {
    MODULE_DEFS
        .iter()
        .map(|def| {
            let password = std::env::var(def.password_env_var).unwrap_or_else(|_| {
                panic!(
                    "Required environment variable {} is not set. \
                     Set it to the {} module's database password.",
                    def.password_env_var, def.name
                )
            });
            ModuleConfig {
                name: def.name,
                postgres_host: def.postgres_host,
                postgres_port: def.postgres_port,
                postgres_user: def.postgres_user,
                postgres_password: password,
                migrations_path: def.migrations_path,
                http_port: def.http_port,
            }
        })
        .collect()
}

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
        let uuid = tenant_id
            .parse::<uuid::Uuid>()
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

    let modules = load_modules();
    let mut module_versions = ModuleSchemaVersions::new();
    let mut overall_success = true;
    let mut error_messages = Vec::new();

    // Provision each module database
    for module in &modules {
        match provision_tenant_module(tid, module).await {
            Ok(version) => {
                module_versions.insert(module.name.to_string(), version);
                info!(
                    "✓ Tenant {} - {} provisioned successfully",
                    tid, module.name
                );
            }
            Err(e) => {
                overall_success = false;
                let error_msg = format!("{}: {}", module.name, e);
                error_messages.push(error_msg.clone());
                warn!(
                    "✗ Tenant {} - {} provisioning failed: {}",
                    tid, module.name, e
                );
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

/// Validate that a database name contains only alphanumeric characters and underscores.
///
/// Returns `Ok(())` if valid, or an error with a clear message if not.
fn validate_db_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("Database name must not be empty");
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        anyhow::bail!(
            "Database name '{}' contains invalid characters. \
             Only alphanumeric characters and underscores are allowed.",
            name
        );
    }
    Ok(())
}

/// Provision a single module's database for a tenant
async fn provision_tenant_module(tenant_id: TenantId, module: &ModuleConfig) -> Result<String> {
    // Generate tenant-specific database name, replacing UUID hyphens with underscores
    let sanitized_id = tenant_id.to_string().replace('-', "_");
    let tenant_db_name = format!("tenant_{}_{}_db", sanitized_id, module.name);

    validate_db_name(&tenant_db_name).context("Tenant database name validation failed")?;

    // Connect to PostgreSQL instance (using default postgres DB)
    let base_url = format!(
        "postgres://{}:{}@{}:{}/postgres",
        module.postgres_user, module.postgres_password, module.postgres_host, module.postgres_port
    );

    let mut conn = PgConnection::connect(&base_url)
        .await
        .context("Failed to connect to PostgreSQL")?;

    // Check if database exists
    let db_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)")
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

    migrator.run(&pool).await.context(format!(
        "Failed to run migrations for {} module",
        module.name
    ))?;

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
        )",
    )
    .fetch_one(pool)
    .await
    .context("Failed to check migrations table")?;

    if !table_exists {
        return Ok("none".to_string());
    }

    // Get latest version
    let version: Option<i64> =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version DESC LIMIT 1")
            .fetch_optional(pool)
            .await
            .context("Failed to query migration version")?;

    Ok(version
        .map(|v| v.to_string())
        .unwrap_or_else(|| "none".to_string()))
}

/// Activate a tenant (transition from provisioning to active)
pub async fn activate_tenant(tenant_id: &str) -> Result<()> {
    info!("Activating tenant: {}", tenant_id);

    // For now, this is a placeholder - in full implementation this would:
    // 1. Update tenant registry status to Active
    // 2. Enable access controls
    // 3. Initialize any runtime state

    info!(
        "✅ Tenant {} activated (registry update pending full implementation)",
        tenant_id
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_configurations_complete() {
        // Set env vars for test
        std::env::set_var("AR_DB_PASSWORD", "test");
        std::env::set_var("PAYMENTS_DB_PASSWORD", "test");
        std::env::set_var("SUBSCRIPTIONS_DB_PASSWORD", "test");
        std::env::set_var("GL_DB_PASSWORD", "test");
        std::env::set_var("NOTIFICATIONS_DB_PASSWORD", "test");

        let modules = load_modules();
        assert_eq!(modules.len(), 5);
        assert!(modules.iter().any(|m| m.name == "ar"));
        assert!(modules.iter().any(|m| m.name == "payments"));
        assert!(modules.iter().any(|m| m.name == "subscriptions"));
        assert!(modules.iter().any(|m| m.name == "gl"));
        assert!(modules.iter().any(|m| m.name == "notifications"));
    }

    #[test]
    fn module_ports_unique() {
        std::env::set_var("AR_DB_PASSWORD", "test");
        std::env::set_var("PAYMENTS_DB_PASSWORD", "test");
        std::env::set_var("SUBSCRIPTIONS_DB_PASSWORD", "test");
        std::env::set_var("GL_DB_PASSWORD", "test");
        std::env::set_var("NOTIFICATIONS_DB_PASSWORD", "test");

        let modules = load_modules();
        let mut postgres_ports = modules.iter().map(|m| m.postgres_port).collect::<Vec<_>>();
        postgres_ports.sort();
        postgres_ports.dedup();
        assert_eq!(
            postgres_ports.len(),
            modules.len(),
            "PostgreSQL ports must be unique"
        );

        let mut http_ports = modules.iter().map(|m| m.http_port).collect::<Vec<_>>();
        http_ports.sort();
        http_ports.dedup();
        assert_eq!(http_ports.len(), modules.len(), "HTTP ports must be unique");
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

    #[test]
    fn validate_db_name_accepts_valid_names() {
        assert!(validate_db_name("tenant_abc123_ar_db").is_ok());
        assert!(validate_db_name("tenant_550e8400_e29b_41d4_a716_446655440000_gl_db").is_ok());
        assert!(validate_db_name("simple").is_ok());
    }

    #[test]
    fn validate_db_name_rejects_empty() {
        let err = validate_db_name("").unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn validate_db_name_rejects_hyphens() {
        let err = validate_db_name("tenant_550e8400-e29b_db").unwrap_err();
        assert!(err.to_string().contains("invalid characters"));
    }

    #[test]
    fn validate_db_name_rejects_special_chars() {
        assert!(validate_db_name("tenant; DROP TABLE --").is_err());
        assert!(validate_db_name("name\"with\"quotes").is_err());
        assert!(validate_db_name("name.with.dots").is_err());
    }

    #[test]
    fn sanitized_tenant_id_produces_valid_db_name() {
        let tid = TenantId::from_uuid("550e8400-e29b-41d4-a716-446655440000".parse().unwrap());
        let sanitized_id = tid.to_string().replace('-', "_");
        let db_name = format!("tenant_{}_{}_db", sanitized_id, "ar");
        assert!(validate_db_name(&db_name).is_ok());
    }
}
