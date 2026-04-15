//! Canonical health endpoint helpers for 7D Solutions Platform.
//!
//! Every module and service MUST expose:
//! - `GET /healthz` — liveness (process up, no dependency checks)
//! - `GET /api/ready` — readiness (dependency-aware, standardized JSON)
//!
//! See `docs/HEALTH-CONTRACT.md` for the full specification.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use axum::{http::StatusCode, Json};
use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

/// Status reported by a readiness check.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Up,
    Down,
}

/// Connection pool metrics for observability.
#[derive(Debug, Clone, Serialize)]
pub struct PoolMetrics {
    /// Total connections managed by the pool (active + idle).
    pub size: u32,
    /// Connections currently idle in the pool.
    pub idle: u32,
    /// Connections currently in use.
    pub active: u32,
}

/// Circuit breaker state snapshot for inclusion in `/api/ready` responses.
#[derive(Debug, Clone, Serialize)]
pub struct CircuitBreakerInfo {
    /// Identifies the downstream service (e.g. `"bom"`, `"production"`).
    pub service: String,
    /// One of `"closed"`, `"open"`, or `"half_open"`.
    pub state: String,
    /// Number of consecutive failures that triggered or are approaching the open threshold.
    pub consecutive_failures: u32,
    /// RFC-3339 timestamp of when the circuit opened, if currently open.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_since: Option<String>,
}

/// A single dependency check result.
#[derive(Debug, Clone, Serialize)]
pub struct HealthCheck {
    pub name: String,
    pub status: CheckStatus,
    pub latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pool: Option<PoolMetrics>,
}

/// Overall service status for the readiness response.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReadyStatus {
    Ready,
    Degraded,
    Down,
}

/// Status of a tenant-scoped readiness probe.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TenantReadyStatus {
    Up,
    Warming,
}

/// Per-tenant readiness result included in `/api/ready?tenant_id=` responses.
#[derive(Debug, Clone, Serialize)]
pub struct TenantReadiness {
    pub id: String,
    pub status: TenantReadyStatus,
}

/// Trait for tenant-scoped readiness probes.
///
/// Implement in modules that subscribe to `tenant.provisioned` events.
/// Call [`TenantReadinessRegistry::set_ready`] from the event consumer to
/// signal that this module has finished setting up for that tenant.
pub trait TenantReadinessCheck: Send + Sync {
    /// Returns `true` when the module has finished processing this tenant.
    fn is_ready(&self, tenant_id: Uuid) -> bool;
}

/// Thread-safe registry of tenant provisioning state.
///
/// Typical usage:
/// ```rust,ignore
/// let registry = Arc::new(TenantReadinessRegistry::new());
///
/// // In the tenant.provisioned consumer:
/// registry.set_ready(tenant_id);
///
/// // In the module builder:
/// ModuleBuilder::from_manifest("module.toml")
///     .tenant_readiness_check(registry.clone())
///     ...
/// ```
#[derive(Default, Clone)]
pub struct TenantReadinessRegistry {
    inner: Arc<Mutex<HashSet<Uuid>>>,
}

impl TenantReadinessRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a tenant as ready.
    ///
    /// Call this when the `tenant.provisioned` consumer successfully finishes
    /// all per-tenant setup for this module.
    pub fn set_ready(&self, tenant_id: Uuid) {
        self.inner.lock().expect("TenantReadinessRegistry lock poisoned").insert(tenant_id);
    }
}

impl TenantReadinessCheck for TenantReadinessRegistry {
    fn is_ready(&self, tenant_id: Uuid) -> bool {
        self.inner.lock().expect("TenantReadinessRegistry lock poisoned").contains(&tenant_id)
    }
}

/// Canonical `/api/ready` response body.
#[derive(Debug, Clone, Serialize)]
pub struct ReadyResponse {
    pub service_name: String,
    pub version: String,
    pub status: ReadyStatus,
    pub degraded: bool,
    pub checks: Vec<HealthCheck>,
    pub timestamp: String,
    /// Present only when `?tenant_id=` is supplied to `/api/ready`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant: Option<TenantReadiness>,
}

/// Canonical `/healthz` response body.
#[derive(Debug, Clone, Serialize)]
pub struct HealthzResponse {
    pub status: String,
}

/// GET /healthz — liveness probe. Always returns 200 if the process is up.
pub async fn healthz() -> Json<HealthzResponse> {
    Json(HealthzResponse {
        status: "alive".to_string(),
    })
}

/// Build a standardized readiness response from dependency check results.
///
/// Rules:
/// - All checks `Up` → status=ready, degraded=false, HTTP 200
/// - Any check `Down` → status=down, degraded=false, HTTP 503
pub fn build_ready_response(
    service_name: &str,
    version: &str,
    checks: Vec<HealthCheck>,
) -> ReadyResponse {
    let any_down = checks.iter().any(|c| c.status == CheckStatus::Down);
    let status = if any_down {
        ReadyStatus::Down
    } else {
        ReadyStatus::Ready
    };
    let degraded = status == ReadyStatus::Degraded;

    ReadyResponse {
        service_name: service_name.to_string(),
        version: version.to_string(),
        status,
        degraded,
        checks,
        timestamp: Utc::now().to_rfc3339(),
        tenant: None,
    }
}

/// Convert a `ReadyResponse` into an axum-compatible result.
///
/// Returns HTTP 200 for Ready/Degraded, HTTP 503 for Down.
pub fn ready_response_to_axum(
    resp: ReadyResponse,
) -> Result<Json<ReadyResponse>, (StatusCode, Json<ReadyResponse>)> {
    match resp.status {
        ReadyStatus::Ready | ReadyStatus::Degraded => Ok(Json(resp)),
        ReadyStatus::Down => Err((StatusCode::SERVICE_UNAVAILABLE, Json(resp))),
    }
}

/// Build a `HealthCheck` from a DB probe result.
pub fn db_check(latency_ms: u64, error: Option<String>) -> HealthCheck {
    HealthCheck {
        name: "database".to_string(),
        status: if error.is_none() {
            CheckStatus::Up
        } else {
            CheckStatus::Down
        },
        latency_ms,
        error,
        pool: None,
    }
}

/// Build a `HealthCheck` from a DB probe result with connection pool metrics.
pub fn db_check_with_pool(
    latency_ms: u64,
    error: Option<String>,
    pool_metrics: PoolMetrics,
) -> HealthCheck {
    HealthCheck {
        name: "database".to_string(),
        status: if error.is_none() {
            CheckStatus::Up
        } else {
            CheckStatus::Down
        },
        latency_ms,
        error,
        pool: Some(pool_metrics),
    }
}

/// Build a `HealthCheck` from a NATS connectivity probe.
pub fn nats_check(connected: bool, latency_ms: u64) -> HealthCheck {
    HealthCheck {
        name: "nats".to_string(),
        status: if connected {
            CheckStatus::Up
        } else {
            CheckStatus::Down
        },
        latency_ms,
        error: if connected {
            None
        } else {
            Some("not connected".to_string())
        },
        pool: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_response_all_up() {
        let checks = vec![db_check(5, None)];
        let resp = build_ready_response("test-svc", "0.1.0", checks);
        assert_eq!(resp.status, ReadyStatus::Ready);
        assert!(!resp.degraded);
        assert_eq!(resp.checks.len(), 1);
    }

    #[test]
    fn ready_response_db_down() {
        let checks = vec![db_check(0, Some("connection refused".into()))];
        let resp = build_ready_response("test-svc", "0.1.0", checks);
        assert_eq!(resp.status, ReadyStatus::Down);
    }

    #[test]
    fn healthz_serializes_correctly() {
        let resp = HealthzResponse {
            status: "alive".to_string(),
        };
        let json = serde_json::to_value(&resp).expect("serialization failed");
        assert_eq!(json["status"], "alive");
    }

    #[test]
    fn ready_response_serializes_correctly() {
        let checks = vec![db_check(3, None), nats_check(true, 1)];
        let resp = build_ready_response("identity-auth", "1.2.0", checks);
        let json = serde_json::to_value(&resp).expect("serialization failed");
        assert_eq!(json["service_name"], "identity-auth");
        assert_eq!(json["status"], "ready");
        assert_eq!(json["degraded"], false);
        assert_eq!(json["checks"].as_array().expect("checks must be array").len(), 2);
        assert!(json["timestamp"].as_str().is_some());
    }

    #[test]
    fn db_check_without_pool_omits_pool_field() {
        let c = db_check(5, None);
        let json = serde_json::to_value(&c).expect("serialization failed");
        assert!(json.get("pool").is_none());
    }

    #[test]
    fn db_check_with_pool_includes_metrics() {
        let metrics = PoolMetrics {
            size: 10,
            idle: 7,
            active: 3,
        };
        let c = db_check_with_pool(5, None, metrics);
        let json = serde_json::to_value(&c).expect("serialization failed");
        let pool = json.get("pool").expect("pool field must be present");
        assert_eq!(pool["size"], 10);
        assert_eq!(pool["idle"], 7);
        assert_eq!(pool["active"], 3);
    }
}
