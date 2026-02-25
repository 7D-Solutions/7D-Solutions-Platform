//! Tenant summary aggregation
//!
//! Fetches tenant record from DB and module readiness via parallel HTTP fanout.
//! No direct cross-module DB reads — HTTP only.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::time::{Duration, Instant};
use thiserror::Error;
use uuid::Uuid;

/// Timeout for each module readiness HTTP check
pub const MODULE_READINESS_TIMEOUT: Duration = Duration::from_secs(2);

/// Module URL configuration for HTTP fanout
#[derive(Debug, Clone)]
pub struct ModuleUrl {
    pub name: String,
    pub base_url: String,
}

impl ModuleUrl {
    pub fn new(name: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            base_url: base_url.into(),
        }
    }

    /// Default local module URLs matching docker-compose ports
    pub fn default_local() -> Vec<ModuleUrl> {
        vec![
            ModuleUrl::new("ar", "http://localhost:8086"),
            ModuleUrl::new("payments", "http://localhost:8088"),
            ModuleUrl::new("subscriptions", "http://localhost:8087"),
            ModuleUrl::new("gl", "http://localhost:8090"),
            ModuleUrl::new("notifications", "http://localhost:8089"),
        ]
    }
}

/// Readiness status for a single module
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ReadinessStatus {
    Ready,
    Degraded,
    Unavailable,
}

/// Per-module readiness result from HTTP fanout
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleReadiness {
    pub module: String,
    pub status: ReadinessStatus,
    pub schema_version: Option<String>,
    pub latency_ms: u64,
    pub error: Option<String>,
}

/// Aggregated tenant summary response
#[derive(Debug, Serialize, Deserialize)]
pub struct TenantSummary {
    pub tenant_id: Uuid,
    pub status: String,
    pub environment: String,
    pub created_at: DateTime<Utc>,
    pub modules: Vec<ModuleReadiness>,
    pub overall_ready: bool,
}

/// Errors during summary fetch
#[derive(Debug, Error)]
pub enum SummaryError {
    #[error("Tenant not found: {0}")]
    TenantNotFound(Uuid),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// Fetch schema version from /api/version endpoint
async fn fetch_schema_version(
    client: &reqwest::Client,
    base_url: &str,
) -> Option<String> {
    #[derive(Deserialize)]
    struct VersionResponse {
        schema_version: Option<String>,
    }

    let url = format!("{}/api/version", base_url);
    let resp = tokio::time::timeout(
        MODULE_READINESS_TIMEOUT,
        client.get(&url).send(),
    )
    .await
    .ok()?
    .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    resp.json::<VersionResponse>().await.ok()?.schema_version
}

/// Fetch module readiness with schema version in parallel
async fn check_module_readiness_full(
    client: &reqwest::Client,
    module: &ModuleUrl,
) -> ModuleReadiness {
    let start = Instant::now();
    let ready_url = format!("{}/api/ready", module.base_url);

    let ready_result = tokio::time::timeout(
        MODULE_READINESS_TIMEOUT,
        client.get(&ready_url).send(),
    )
    .await;

    let latency_ms = start.elapsed().as_millis() as u64;

    match ready_result {
        Ok(Ok(resp)) if resp.status().is_success() => {
            let schema_version = fetch_schema_version(client, &module.base_url).await;
            ModuleReadiness {
                module: module.name.clone(),
                status: ReadinessStatus::Ready,
                schema_version,
                latency_ms,
                error: None,
            }
        }
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
            error: Some(format!("timeout after {}ms", MODULE_READINESS_TIMEOUT.as_millis())),
        },
    }
}

/// Fetch aggregated tenant summary: registry record + parallel module readiness checks
pub async fn fetch_tenant_summary(
    pool: &PgPool,
    client: &reqwest::Client,
    module_urls: &[ModuleUrl],
    tenant_id: Uuid,
) -> Result<TenantSummary, SummaryError> {
    // 1. Fetch tenant record from registry DB
    let row = sqlx::query_as::<_, (String, String, DateTime<Utc>)>(
        "SELECT status, environment, created_at FROM tenants WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    let (status, environment, created_at) = match row {
        Some(r) => r,
        None => return Err(SummaryError::TenantNotFound(tenant_id)),
    };

    // 2. Parallel HTTP fanout to all module readiness endpoints
    let checks: Vec<_> = module_urls
        .iter()
        .map(|m| check_module_readiness_full(client, m))
        .collect();

    let modules = futures::future::join_all(checks).await;

    // 3. Compute overall_ready: all modules must be Ready
    let overall_ready = modules.iter().all(|m| m.status == ReadinessStatus::Ready);

    Ok(TenantSummary {
        tenant_id,
        status,
        environment,
        created_at,
        modules,
        overall_ready,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_url_default_local_has_five_modules() {
        let urls = ModuleUrl::default_local();
        assert_eq!(urls.len(), 5);
        let names: Vec<&str> = urls.iter().map(|u| u.name.as_str()).collect();
        assert!(names.contains(&"ar"));
        assert!(names.contains(&"payments"));
        assert!(names.contains(&"subscriptions"));
        assert!(names.contains(&"gl"));
        assert!(names.contains(&"notifications"));
    }

    #[test]
    fn readiness_status_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&ReadinessStatus::Ready).unwrap(),
            r#""ready""#
        );
        assert_eq!(
            serde_json::to_string(&ReadinessStatus::Unavailable).unwrap(),
            r#""unavailable""#
        );
    }

    #[test]
    fn tenant_summary_serializes_correctly() {
        let summary = TenantSummary {
            tenant_id: Uuid::nil(),
            status: "active".to_string(),
            environment: "development".to_string(),
            created_at: Utc::now(),
            modules: vec![ModuleReadiness {
                module: "ar".to_string(),
                status: ReadinessStatus::Ready,
                schema_version: Some("20260216000001".to_string()),
                latency_ms: 42,
                error: None,
            }],
            overall_ready: true,
        };

        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("overall_ready"));
        assert!(json.contains("modules"));
        assert!(json.contains("latency_ms"));
    }

    #[test]
    fn overall_ready_false_when_any_module_unavailable() {
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
        ];
        let overall_ready = modules.iter().all(|m| m.status == ReadinessStatus::Ready);
        assert!(!overall_ready);
    }

    #[test]
    fn summary_error_not_found_message() {
        let id = Uuid::new_v4();
        let err = SummaryError::TenantNotFound(id);
        assert!(err.to_string().contains(&id.to_string()));
    }
}
