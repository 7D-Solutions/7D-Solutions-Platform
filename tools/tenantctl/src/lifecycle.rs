//! Tenant lifecycle operations: suspend and deprovision
//!
//! These operations manage tenant state transitions beyond initial provisioning:
//! - Suspend: Temporarily disable a tenant (retain data, disable access)
//! - Deprovision: Soft-delete a tenant (mark for cleanup, follow retention policy)
//!
//! All lifecycle transitions are auditable and atomic.

use anyhow::{Context, Result, bail};
use audit::schema::{MutationClass, WriteAuditRequest};
use audit::writer::AuditWriter;
use security::{RbacPolicy, Role, Operation};
use sqlx::{PgPool, Postgres, Transaction};
use tenant_registry::{TenantStatus, is_valid_state_transition};
use uuid::Uuid;

// ============================================================================
// Database Connection
// ============================================================================

/// Get connection pool for platform audit database
async fn get_audit_pool() -> Result<PgPool> {
    let database_url = std::env::var("PLATFORM_AUDIT_DATABASE_URL")
        .context("PLATFORM_AUDIT_DATABASE_URL not set")?;

    PgPool::connect(&database_url)
        .await
        .context("Failed to connect to platform audit database")
}

/// Get connection pool for tenant registry database
async fn get_registry_pool() -> Result<PgPool> {
    let database_url = std::env::var("TENANT_REGISTRY_DATABASE_URL")
        .context("TENANT_REGISTRY_DATABASE_URL not set")?;

    PgPool::connect(&database_url)
        .await
        .context("Failed to connect to tenant registry database")
}

// ============================================================================
// Suspend Tenant
// ============================================================================

/// Suspend a tenant (disable access, retain data)
///
/// This operation:
/// 1. Authorizes role can perform operation
/// 2. Verifies tenant exists and is Active
/// 3. Updates tenant status to Suspended
/// 4. Records audit log entry
///
/// Suspension is reversible - tenant can be reactivated later.
///
/// # Errors
/// - Insufficient permissions
/// - Tenant not found
/// - Tenant not in Active state
/// - Database connection/transaction errors
pub async fn suspend_tenant(role: Role, actor: &str, tenant_id: &str) -> Result<()> {
    // Authorize operation
    RbacPolicy::authorize(role, Operation::TenantSuspend, actor, tenant_id)?;

    tracing::info!(tenant_id, actor, role = ?role, "Suspending tenant");

    let registry_pool = get_registry_pool().await?;
    let audit_pool = get_audit_pool().await?;

    // Parse tenant ID
    let tenant_uuid = Uuid::parse_str(tenant_id)
        .context("Invalid tenant ID format")?;

    // Start transaction for atomic state transition
    let mut tx = registry_pool.begin().await?;

    // Fetch current tenant record
    let tenant = fetch_tenant(&mut tx, tenant_uuid).await?;

    tracing::debug!(
        tenant_id = %tenant_uuid,
        current_status = ?tenant.status,
        "Current tenant status"
    );

    // Verify valid state transition
    if !is_valid_state_transition(tenant.status(), TenantStatus::Suspended) {
        bail!(
            "Cannot suspend tenant {}: invalid state transition {} -> suspended",
            tenant_id,
            tenant.status
        );
    }

    // Update tenant status
    sqlx::query(
        r#"
        UPDATE tenants
        SET status = 'suspended',
            updated_at = CURRENT_TIMESTAMP
        WHERE tenant_id = $1
        "#
    )
    .bind(tenant_uuid)
    .execute(&mut *tx)
    .await
    .context("Failed to update tenant status to suspended")?;

    // Commit registry transaction
    tx.commit().await.context("Failed to commit tenant suspension")?;

    // Write audit log entry
    write_lifecycle_audit_entry(
        &audit_pool,
        tenant_uuid,
        "tenant.suspended",
        &tenant.status,
        "suspended",
    )
    .await?;

    tracing::info!(tenant_id, "Tenant suspended successfully");
    Ok(())
}

// ============================================================================
// Deprovision Tenant
// ============================================================================

/// Deprovision a tenant (soft delete, mark for cleanup)
///
/// This operation:
/// 1. Authorizes role can perform operation
/// 2. Verifies tenant exists and is Active or Suspended
/// 3. Updates tenant status to Deleted
/// 4. Sets deleted_at timestamp
/// 5. Records audit log entry
///
/// Deprovisioning is a soft delete - data is retained according to
/// retention policy. Physical deletion would be a separate cleanup process.
///
/// # Errors
/// - Insufficient permissions
/// - Tenant not found
/// - Tenant not in Active or Suspended state
/// - Database connection/transaction errors
pub async fn deprovision_tenant(role: Role, actor: &str, tenant_id: &str) -> Result<()> {
    // Authorize operation
    RbacPolicy::authorize(role, Operation::TenantDeprovision, actor, tenant_id)?;

    tracing::info!(tenant_id, actor, role = ?role, "Deprovisioning tenant");

    let registry_pool = get_registry_pool().await?;
    let audit_pool = get_audit_pool().await?;

    // Parse tenant ID
    let tenant_uuid = Uuid::parse_str(tenant_id)
        .context("Invalid tenant ID format")?;

    // Start transaction for atomic state transition
    let mut tx = registry_pool.begin().await?;

    // Fetch current tenant record
    let tenant = fetch_tenant(&mut tx, tenant_uuid).await?;

    tracing::debug!(
        tenant_id = %tenant_uuid,
        current_status = ?tenant.status,
        "Current tenant status"
    );

    // Verify valid state transition
    if !is_valid_state_transition(tenant.status(), TenantStatus::Deleted) {
        bail!(
            "Cannot deprovision tenant {}: invalid state transition {} -> deleted",
            tenant_id,
            tenant.status
        );
    }

    // Update tenant status and set deleted_at
    sqlx::query(
        r#"
        UPDATE tenants
        SET status = 'deleted',
            deleted_at = CURRENT_TIMESTAMP,
            updated_at = CURRENT_TIMESTAMP
        WHERE tenant_id = $1
        "#
    )
    .bind(tenant_uuid)
    .execute(&mut *tx)
    .await
    .context("Failed to update tenant status to deleted")?;

    // Commit registry transaction
    tx.commit().await.context("Failed to commit tenant deprovision")?;

    // Write audit log entry
    write_lifecycle_audit_entry(
        &audit_pool,
        tenant_uuid,
        "tenant.deprovisioned",
        &tenant.status,
        "deleted",
    )
    .await?;

    tracing::info!(tenant_id, "Tenant deprovisioned successfully");
    Ok(())
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Minimal tenant record for lifecycle operations
#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)]
struct TenantRecord {
    tenant_id: Uuid,
    status: String,
}

impl TenantRecord {
    fn status(&self) -> TenantStatus {
        match self.status.as_str() {
            "provisioning" => TenantStatus::Provisioning,
            "active" => TenantStatus::Active,
            "suspended" => TenantStatus::Suspended,
            "deleted" => TenantStatus::Deleted,
            _ => panic!("Unknown tenant status: {}", self.status),
        }
    }
}

/// Fetch tenant record from registry
async fn fetch_tenant(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
) -> Result<TenantRecord> {
    sqlx::query_as::<_, TenantRecord>(
        "SELECT tenant_id, status FROM tenants WHERE tenant_id = $1"
    )
    .bind(tenant_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| anyhow::anyhow!("Tenant not found: {}", tenant_id))
}

/// Write audit log entry for lifecycle transition
async fn write_lifecycle_audit_entry(
    audit_pool: &PgPool,
    tenant_id: Uuid,
    action: &str,
    before_status: &str,
    after_status: &str,
) -> Result<()> {
    let writer = AuditWriter::new(audit_pool.clone());

    let request = WriteAuditRequest::new(
        Uuid::nil(), // System actor (tenantctl CLI)
        "system".to_string(),
        action.to_string(),
        MutationClass::StateTransition,
        "tenant".to_string(),
        tenant_id.to_string(),
    )
    .with_snapshots(
        Some(serde_json::json!({ "status": before_status })),
        Some(serde_json::json!({ "status": after_status })),
    )
    .with_metadata(serde_json::json!({
        "source": "tenantctl",
        "operation": action,
    }));

    writer.write(request).await?;

    tracing::debug!(
        tenant_id = %tenant_id,
        action = action,
        "Audit entry written"
    );

    Ok(())
}

// ============================================================================
// Demo Reset
// ============================================================================

/// Result of a demo reset operation
#[derive(Debug)]
pub struct DemoResetResult {
    /// The resolved tenant UUID
    pub tenant_id: String,
    /// SHA256 digest of the seeded dataset
    pub dataset_digest: String,
}

/// Reset a tenant to a known demo state.
///
/// This operation:
/// 1. Drops all tenant module databases (irreversible!)
/// 2. Re-provisions the tenant (creates DBs, runs migrations)
/// 3. Re-seeds demo data via demo-seed binary with the given seed
/// 4. Returns the dataset digest for determinism verification
///
/// **Warning:** All existing tenant data is permanently deleted.
pub async fn demo_reset_tenant(
    tenant_id: &str,
    seed: u64,
    ar_base_url: &str,
) -> Result<DemoResetResult> {
    use crate::provision::{create_tenant, MODULES};
    use sqlx::Connection;

    tracing::info!(tenant_id, seed, "Starting demo reset");

    // Resolve tenant ID (same logic as create_tenant)
    let tid = if tenant_id.len() == 36 {
        
        tenant_id
            .parse::<Uuid>()
            .context("Invalid tenant UUID format")?
    } else {
        let namespace = Uuid::NAMESPACE_DNS;
        Uuid::new_v5(&namespace, tenant_id.as_bytes())
    };

    // 1. Drop all module databases for this tenant
    for module in MODULES {
        let db_name = format!("tenant_{}_{}_db", tid, module.name);
        let base_url = format!(
            "postgres://{}:{}@{}:{}/postgres",
            module.postgres_user,
            module.postgres_password,
            module.postgres_host,
            module.postgres_port
        );

        match sqlx::postgres::PgConnection::connect(&base_url).await {
            Ok(mut conn) => {
                // Terminate existing connections to the database before dropping
                let _ = sqlx::query(
                    "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1"
                )
                .bind(&db_name)
                .execute(&mut conn)
                .await;

                // Drop the database (ignore if it doesn't exist)
                let drop_sql = format!("DROP DATABASE IF EXISTS \"{}\"", db_name);
                match sqlx::query(&drop_sql).execute(&mut conn).await {
                    Ok(_) => tracing::info!(db_name, module = module.name, "Dropped database"),
                    Err(e) => tracing::warn!(
                        db_name,
                        module = module.name,
                        error = %e,
                        "Failed to drop database (continuing)"
                    ),
                }
            }
            Err(e) => {
                tracing::warn!(
                    module = module.name,
                    error = %e,
                    "Cannot connect to drop databases (continuing)"
                );
            }
        }
    }

    // 2. Re-provision the tenant
    let provision_result = create_tenant(tenant_id)
        .await
        .context("Failed to re-provision tenant after reset")?;

    if !provision_result.success {
        let err = provision_result
            .error_message
            .unwrap_or_else(|| "Unknown provisioning error".to_string());
        anyhow::bail!("Re-provisioning failed: {}", err);
    }

    tracing::info!(tenant_id, "Tenant re-provisioned successfully");

    // 3. Run demo-seed via subprocess
    //
    // We invoke the compiled demo-seed binary. In CI, it should already be
    // compiled. In dev, we use `cargo run -p demo-seed`.
    let demo_seed_binary = std::env::var("DEMO_SEED_BINARY")
        .unwrap_or_else(|_| "cargo".to_string());

    let mut cmd = if demo_seed_binary == "cargo" {
        let mut c = std::process::Command::new("cargo");
        c.args(["run", "-p", "demo-seed", "--"]);
        c
    } else {
        std::process::Command::new(&demo_seed_binary)
    };

    cmd.args([
        "--tenant", tenant_id,
        "--seed", &seed.to_string(),
        "--ar-url", ar_base_url,
    ]);

    tracing::info!(tenant_id, seed, ar_base_url, "Running demo-seed");

    let output = cmd.output().context("Failed to run demo-seed")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("demo-seed failed: {}", stderr);
    }

    let digest = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if digest.is_empty() {
        anyhow::bail!("demo-seed produced no output digest");
    }

    tracing::info!(tenant_id, seed, digest = %digest, "Demo reset complete");

    Ok(DemoResetResult {
        tenant_id: tid.to_string(),
        dataset_digest: digest,
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_record_status_parsing() {
        let record = TenantRecord {
            tenant_id: Uuid::new_v4(),
            status: "active".to_string(),
        };
        assert!(matches!(record.status(), TenantStatus::Active));

        let record = TenantRecord {
            tenant_id: Uuid::new_v4(),
            status: "suspended".to_string(),
        };
        assert!(matches!(record.status(), TenantStatus::Suspended));
    }

    #[test]
    #[should_panic(expected = "Unknown tenant status")]
    fn tenant_record_invalid_status_panics() {
        let record = TenantRecord {
            tenant_id: Uuid::new_v4(),
            status: "invalid".to_string(),
        };
        let _ = record.status();
    }
}
