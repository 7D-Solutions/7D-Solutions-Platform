//! Fleet list + status — query tenant registry for tenant inventory.
//!
//! `fleet status` — summary overview (counts by status, service health).
//! `fleet list` — list all tenants with status and metadata.

use anyhow::{Context, Result};
use serde::Serialize;
use sqlx::PgPool;

use crate::output::CommandOutput;

/// Tenant row from the registry.
#[derive(Debug, sqlx::FromRow, Serialize)]
struct TenantRow {
    tenant_id: uuid::Uuid,
    status: String,
    environment: String,
    app_id: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

/// Status count from the registry.
#[derive(Debug, sqlx::FromRow)]
struct StatusCount {
    status: String,
    count: i64,
}

/// Connect to the tenant registry database.
async fn registry_pool() -> Result<PgPool> {
    let url = std::env::var("TENANT_REGISTRY_DATABASE_URL")
        .context("TENANT_REGISTRY_DATABASE_URL not set")?;
    PgPool::connect(&url)
        .await
        .context("Failed to connect to tenant registry")
}

/// `tenantctl fleet status` — overall fleet state.
pub async fn fleet_status() -> Result<CommandOutput> {
    let pool = registry_pool().await?;

    let counts: Vec<StatusCount> = sqlx::query_as(
        "SELECT status, COUNT(*)::bigint AS count FROM tenants GROUP BY status ORDER BY status",
    )
    .fetch_all(&pool)
    .await
    .context("Querying tenant counts")?;

    let total: i64 = counts.iter().map(|c| c.count).sum();
    let active = counts.iter().find(|c| c.status == "active").map(|c| c.count).unwrap_or(0);
    let suspended = counts.iter().find(|c| c.status == "suspended").map(|c| c.count).unwrap_or(0);
    let provisioning = counts.iter().find(|c| c.status == "provisioning").map(|c| c.count).unwrap_or(0);
    let deleted = counts.iter().find(|c| c.status == "deleted").map(|c| c.count).unwrap_or(0);

    // Print human-readable summary
    eprintln!();
    eprintln!("Fleet Status");
    eprintln!("{}", "-".repeat(30));
    eprintln!("  Total tenants:    {}", total);
    eprintln!("  Active:           {}", active);
    eprintln!("  Suspended:        {}", suspended);
    eprintln!("  Provisioning:     {}", provisioning);
    eprintln!("  Deleted:          {}", deleted);
    eprintln!();

    let data = serde_json::json!({
        "total": total,
        "active": active,
        "suspended": suspended,
        "provisioning": provisioning,
        "deleted": deleted,
    });

    Ok(CommandOutput::ok("fleet-status", "-")
        .with_state("ok")
        .with_data(data))
}

/// `tenantctl fleet list` — list all tenants with state.
pub async fn fleet_list() -> Result<CommandOutput> {
    let pool = registry_pool().await?;

    let tenants: Vec<TenantRow> = sqlx::query_as(
        r#"SELECT tenant_id, status, environment, app_id, created_at, updated_at
           FROM tenants
           ORDER BY created_at ASC"#,
    )
    .fetch_all(&pool)
    .await
    .context("Querying tenants")?;

    // Print human-readable table
    eprintln!();
    eprintln!(
        "{:<38} {:<14} {:<10} {:<12} {}",
        "TENANT_ID", "STATUS", "ENV", "APP_ID", "UPDATED"
    );
    eprintln!("{}", "-".repeat(95));
    for t in &tenants {
        eprintln!(
            "{:<38} {:<14} {:<10} {:<12} {}",
            t.tenant_id,
            t.status,
            t.environment,
            t.app_id.as_deref().unwrap_or("-"),
            t.updated_at.format("%Y-%m-%d %H:%M"),
        );
    }
    eprintln!();

    let tenant_data: Vec<serde_json::Value> = tenants
        .iter()
        .map(|t| {
            serde_json::json!({
                "tenant_id": t.tenant_id.to_string(),
                "status": t.status,
                "environment": t.environment,
                "app_id": t.app_id,
                "created_at": t.created_at.to_rfc3339(),
                "updated_at": t.updated_at.to_rfc3339(),
            })
        })
        .collect();

    let data = serde_json::json!({
        "count": tenants.len(),
        "tenants": tenant_data,
    });

    Ok(CommandOutput::ok("fleet-list", "-")
        .with_data(data))
}

#[cfg(test)]
mod tests {
    #[test]
    fn module_compiles() {
        // Ensures the module structure is valid.
    }
}
