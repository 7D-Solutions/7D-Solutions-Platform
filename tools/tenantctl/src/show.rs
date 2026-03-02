//! tenantctl tenant show — display tenant state, mapping, and entitlements
//!
//! Queries the tenant registry for the full tenant record and formats
//! it as either human-readable text or JSON.

use anyhow::{Context, Result};
use sqlx::PgPool;
use uuid::Uuid;

use crate::output::CommandOutput;

/// Full tenant detail fetched from the registry.
#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)] // tenant_id used for display via query result
struct TenantDetailRow {
    tenant_id: Uuid,
    status: String,
    environment: String,
    product_code: Option<String>,
    plan_code: Option<String>,
    app_id: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Entitlement row for display.
#[derive(Debug, sqlx::FromRow)]
struct EntitlementDetail {
    plan_code: String,
    concurrent_user_limit: i32,
    effective_at: chrono::DateTime<chrono::Utc>,
}

/// Provisioning step row for display.
#[derive(Debug, sqlx::FromRow)]
struct ProvStepRow {
    step_name: String,
    step_order: i32,
    status: String,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
    error_message: Option<String>,
}

/// Execute `tenant show` and return a `CommandOutput`.
pub async fn show_tenant(tenant_id: &str) -> Result<CommandOutput> {
    let db_url = std::env::var("TENANT_REGISTRY_DATABASE_URL")
        .context("TENANT_REGISTRY_DATABASE_URL not set")?;

    let pool = PgPool::connect(&db_url)
        .await
        .context("Failed to connect to tenant registry")?;

    let tid = parse_tenant_id(tenant_id)?;

    // Fetch core tenant record
    let tenant: Option<TenantDetailRow> = sqlx::query_as(
        r#"SELECT tenant_id, status, environment, product_code, plan_code,
                  app_id, created_at, updated_at, deleted_at
           FROM tenants WHERE tenant_id = $1"#,
    )
    .bind(tid)
    .fetch_optional(&pool)
    .await
    .context("Querying tenant record")?;

    let tenant = match tenant {
        Some(t) => t,
        None => {
            return Ok(CommandOutput::fail("show", tenant_id, "Tenant not found"));
        }
    };

    // Fetch entitlements (optional)
    let entitlement: Option<EntitlementDetail> = sqlx::query_as(
        r#"SELECT plan_code, concurrent_user_limit, effective_at
           FROM cp_entitlements WHERE tenant_id = $1"#,
    )
    .bind(tid)
    .fetch_optional(&pool)
    .await
    .context("Querying entitlements")?;

    // Fetch provisioning steps (optional)
    let prov_steps: Vec<ProvStepRow> = sqlx::query_as(
        r#"SELECT step_name, step_order, status, completed_at, error_message
           FROM provisioning_steps WHERE tenant_id = $1
           ORDER BY step_order ASC"#,
    )
    .bind(tid)
    .fetch_all(&pool)
    .await
    .unwrap_or_default();

    // Build data payload
    let mut data = serde_json::json!({
        "environment": tenant.environment,
        "product_code": tenant.product_code,
        "plan_code": tenant.plan_code,
        "app_id": tenant.app_id,
        "created_at": tenant.created_at.to_rfc3339(),
        "updated_at": tenant.updated_at.to_rfc3339(),
    });

    if let Some(deleted) = tenant.deleted_at {
        data["deleted_at"] = serde_json::Value::String(deleted.to_rfc3339());
    }

    if let Some(ent) = entitlement {
        data["entitlement"] = serde_json::json!({
            "plan_code": ent.plan_code,
            "concurrent_user_limit": ent.concurrent_user_limit,
            "effective_at": ent.effective_at.to_rfc3339(),
        });
    }

    if !prov_steps.is_empty() {
        let steps: Vec<serde_json::Value> = prov_steps
            .iter()
            .map(|s| {
                let mut step = serde_json::json!({
                    "step_name": s.step_name,
                    "step_order": s.step_order,
                    "status": s.status,
                });
                if let Some(ref completed) = s.completed_at {
                    step["completed_at"] = serde_json::Value::String(completed.to_rfc3339());
                }
                if let Some(ref err) = s.error_message {
                    step["error_message"] = serde_json::Value::String(err.clone());
                }
                step
            })
            .collect();
        data["provisioning_steps"] = serde_json::Value::Array(steps);
    }

    Ok(CommandOutput::ok("show", &tid.to_string())
        .with_state(&tenant.status)
        .with_data(data))
}

fn parse_tenant_id(tenant_id: &str) -> Result<Uuid> {
    if tenant_id.len() == 36 {
        Uuid::parse_str(tenant_id).context("Invalid tenant UUID format")
    } else {
        Ok(Uuid::new_v5(&Uuid::NAMESPACE_DNS, tenant_id.as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tenant_id_deterministic() {
        let a = parse_tenant_id("acme").unwrap();
        let b = parse_tenant_id("acme").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn parse_tenant_id_uuid_passthrough() {
        let id = "00000000-0000-0000-0000-000000000001";
        let parsed = parse_tenant_id(id).unwrap();
        assert_eq!(parsed.to_string(), id);
    }
}
