//! Module health verification and tenant activation
//!
//! Provides HTTP-based readiness checks for all platform modules and
//! atomic tenant activation: status transition + provisioned outbox event
//! in a single database transaction.
//!
//! Activation contract:
//! - ALL modules must report Ready before activation proceeds
//! - Status update (→ active) and outbox event are committed in one transaction
//! - If any module is unavailable, tenant stays in provisioning state
//! - Outbox event type: tenant.provisioned

use crate::lifecycle::event_types;
use crate::summary::{ModuleReadiness, ModuleUrl, ReadinessStatus, MODULE_READINESS_TIMEOUT};
use serde_json::json;
use sqlx::PgPool;
use std::time::Instant;
use uuid::Uuid;

/// Result of checking all module readiness endpoints
#[derive(Debug)]
pub struct HealthCheckResult {
    /// True only if every module returned Ready
    pub all_ready: bool,
    /// Per-module readiness details
    pub module_results: Vec<ModuleReadiness>,
}

/// Errors that can occur during tenant activation
#[derive(Debug, thiserror::Error)]
pub enum ActivationError {
    #[error("Modules not ready: {}", format_unavailable(.0))]
    ModulesNotReady(Vec<ModuleReadiness>),

    #[error("Database error during activation: {0}")]
    DatabaseError(#[from] sqlx::Error),

    #[error("Tenant not found: {0}")]
    TenantNotFound(Uuid),
}

fn format_unavailable(modules: &[ModuleReadiness]) -> String {
    modules
        .iter()
        .filter(|m| m.status != ReadinessStatus::Ready)
        .map(|m| {
            format!(
                "{}({})",
                m.module,
                m.error.as_deref().unwrap_or("not ready")
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

// ============================================================================
// HTTP Readiness Checks
// ============================================================================

/// Check a single module's /api/ready endpoint
async fn check_module_ready(client: &reqwest::Client, module: &ModuleUrl) -> ModuleReadiness {
    let start = Instant::now();
    let ready_url = format!("{}/api/ready", module.base_url);

    let result =
        tokio::time::timeout(MODULE_READINESS_TIMEOUT, client.get(&ready_url).send()).await;

    let latency_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(Ok(resp)) if resp.status().is_success() => ModuleReadiness {
            module: module.name.clone(),
            status: ReadinessStatus::Ready,
            schema_version: None,
            latency_ms,
            error: None,
        },
        Ok(Ok(resp)) => ModuleReadiness {
            module: module.name.clone(),
            status: ReadinessStatus::Degraded,
            schema_version: None,
            latency_ms,
            error: Some(format!("HTTP {}", resp.status())),
        },
        Ok(Err(e)) => ModuleReadiness {
            module: module.name.clone(),
            status: ReadinessStatus::Unavailable,
            schema_version: None,
            latency_ms,
            error: Some(e.to_string()),
        },
        Err(_) => ModuleReadiness {
            module: module.name.clone(),
            status: ReadinessStatus::Unavailable,
            schema_version: None,
            latency_ms,
            error: Some(format!(
                "timeout after {}ms",
                MODULE_READINESS_TIMEOUT.as_millis()
            )),
        },
    }
}

/// Check all module /api/ready endpoints in parallel.
///
/// Returns HealthCheckResult with all_ready=true only if every module
/// returns a 2xx status within MODULE_READINESS_TIMEOUT.
/// An empty module_urls list is vacuously all_ready (useful in tests).
pub async fn check_all_modules_ready(
    client: &reqwest::Client,
    module_urls: &[ModuleUrl],
) -> HealthCheckResult {
    if module_urls.is_empty() {
        return HealthCheckResult {
            all_ready: true,
            module_results: vec![],
        };
    }

    let checks: Vec<_> = module_urls
        .iter()
        .map(|m| check_module_ready(client, m))
        .collect();

    let module_results = futures::future::join_all(checks).await;
    let all_ready = module_results
        .iter()
        .all(|m| m.status == ReadinessStatus::Ready);

    HealthCheckResult {
        all_ready,
        module_results,
    }
}

// ============================================================================
// Atomic Tenant Activation
// ============================================================================

/// Atomically transition tenant to active and emit tenant.provisioned outbox event.
///
/// Runs inside a single database transaction:
/// 1. UPDATE tenants SET status='active' WHERE tenant_id=? AND status='provisioning'
/// 2. INSERT INTO provisioning_outbox (event_type, payload) ...
///
/// Returns TenantNotFound if the tenant doesn't exist or is not in provisioning state.
pub async fn activate_tenant_atomic(
    registry_pool: &PgPool,
    tenant_id: Uuid,
) -> Result<(), ActivationError> {
    let mut tx = registry_pool.begin().await?;

    // Guard: only activate if currently in 'provisioning' state
    let rows_updated = sqlx::query(
        r#"
        UPDATE tenants
        SET status = 'active', updated_at = NOW()
        WHERE tenant_id = $1
          AND status = 'provisioning'
        "#,
    )
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    if rows_updated == 0 {
        tx.rollback().await?;
        return Err(ActivationError::TenantNotFound(tenant_id));
    }

    // Outbox: emit tenant.provisioned event in same transaction
    let payload = json!({
        "tenant_id": tenant_id.to_string(),
        "event_version": "1.0",
    });

    sqlx::query(
        r#"
        INSERT INTO provisioning_outbox (tenant_id, event_type, payload)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(tenant_id)
    .bind(event_types::TENANT_PROVISIONED)
    .bind(&payload)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

// ============================================================================
// Combined: Verify Readiness Then Activate
// ============================================================================

/// Check all module /api/ready endpoints, then atomically activate the tenant.
///
/// This is the main entry point for the activation step:
/// 1. HTTP fanout to all module /api/ready endpoints (parallel, with timeouts)
/// 2. If ALL modules are Ready → atomic DB transaction (status + outbox event)
/// 3. If ANY module is not Ready → return ActivationError::ModulesNotReady
///
/// The tenant remains in 'provisioning' state if health checks fail.
pub async fn verify_and_activate_tenant(
    registry_pool: &PgPool,
    http_client: &reqwest::Client,
    module_urls: &[ModuleUrl],
    tenant_id: Uuid,
) -> Result<HealthCheckResult, ActivationError> {
    // Step 1: HTTP health checks (outside DB transaction)
    let health = check_all_modules_ready(http_client, module_urls).await;

    if !health.all_ready {
        let unavailable: Vec<ModuleReadiness> = health
            .module_results
            .into_iter()
            .filter(|m| m.status != ReadinessStatus::Ready)
            .collect();
        return Err(ActivationError::ModulesNotReady(unavailable));
    }

    // Step 2: Atomic activation in DB
    activate_tenant_atomic(registry_pool, tenant_id).await?;

    Ok(health)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_unavailable_lists_non_ready_modules() {
        let modules = vec![
            ModuleReadiness {
                module: "ar".to_string(),
                status: ReadinessStatus::Ready,
                schema_version: None,
                latency_ms: 10,
                error: None,
            },
            ModuleReadiness {
                module: "gl".to_string(),
                status: ReadinessStatus::Unavailable,
                schema_version: None,
                latency_ms: 2001,
                error: Some("timeout after 2000ms".to_string()),
            },
            ModuleReadiness {
                module: "payments".to_string(),
                status: ReadinessStatus::Degraded,
                schema_version: None,
                latency_ms: 500,
                error: Some("HTTP 503".to_string()),
            },
        ];

        let msg = format_unavailable(&modules);
        assert!(msg.contains("gl"));
        assert!(msg.contains("payments"));
        assert!(!msg.contains("ar")); // ar is ready, should not appear
    }

    #[test]
    fn activation_error_displays_module_names() {
        let unavailable = vec![ModuleReadiness {
            module: "subscriptions".to_string(),
            status: ReadinessStatus::Unavailable,
            schema_version: None,
            latency_ms: 2001,
            error: Some("connection refused".to_string()),
        }];
        let err = ActivationError::ModulesNotReady(unavailable);
        assert!(err.to_string().contains("subscriptions"));
    }

    #[tokio::test]
    async fn empty_module_list_is_all_ready() {
        let client = reqwest::Client::new();
        let result = check_all_modules_ready(&client, &[]).await;
        assert!(result.all_ready);
        assert!(result.module_results.is_empty());
    }

    #[tokio::test]
    async fn unavailable_url_returns_not_ready() {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(100))
            .build()
            .unwrap();
        let module_urls = vec![ModuleUrl::new("test-module", "http://127.0.0.1:19999")];
        let result = check_all_modules_ready(&client, &module_urls).await;
        assert!(!result.all_ready);
        assert_eq!(result.module_results.len(), 1);
        assert_ne!(result.module_results[0].status, ReadinessStatus::Ready);
    }
}
