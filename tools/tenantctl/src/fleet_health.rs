//! Fleet health — query /api/ready for every service in the platform.
//!
//! Uses the standardized health contract (docs/HEALTH-CONTRACT.md):
//! - `/healthz` → liveness (always 200 if process up)
//! - `/api/ready` → readiness (dependency-aware, JSON shape)

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::Instant;

use crate::output::CommandOutput;

/// Service definition with name and HTTP port.
#[derive(Debug, Clone)]
pub struct ServiceEndpoint {
    pub name: &'static str,
    pub port: u16,
}

/// All known services in the platform (matches docker-compose + verify script).
pub const SERVICES: &[ServiceEndpoint] = &[
    ServiceEndpoint { name: "ar", port: 8086 },
    ServiceEndpoint { name: "ap", port: 8093 },
    ServiceEndpoint { name: "gl", port: 8090 },
    ServiceEndpoint { name: "inventory", port: 8092 },
    ServiceEndpoint { name: "subscriptions", port: 8087 },
    ServiceEndpoint { name: "payments", port: 8088 },
    ServiceEndpoint { name: "notifications", port: 8089 },
    ServiceEndpoint { name: "treasury", port: 8094 },
    ServiceEndpoint { name: "fixed-assets", port: 8104 },
    ServiceEndpoint { name: "consolidation", port: 8105 },
    ServiceEndpoint { name: "timekeeping", port: 8097 },
    ServiceEndpoint { name: "party", port: 8098 },
    ServiceEndpoint { name: "integrations", port: 8099 },
    ServiceEndpoint { name: "ttp", port: 8100 },
    ServiceEndpoint { name: "reporting", port: 8096 },
    ServiceEndpoint { name: "maintenance", port: 8101 },
    ServiceEndpoint { name: "shipping-receiving", port: 8103 },
    ServiceEndpoint { name: "pdf-editor", port: 8106 },
    ServiceEndpoint { name: "identity-auth", port: 8080 },
];

/// Mirrors the canonical /api/ready response from HEALTH-CONTRACT.md.
#[derive(Debug, Deserialize, Serialize)]
pub struct ReadyResponse {
    pub service_name: String,
    pub version: String,
    pub status: String,
    pub degraded: bool,
    pub checks: Vec<ReadyCheck>,
    pub timestamp: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReadyCheck {
    pub name: String,
    pub status: String,
    pub latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Result of probing a single service.
#[derive(Debug, Serialize)]
pub struct ServiceHealthResult {
    pub name: String,
    pub port: u16,
    pub reachable: bool,
    pub status: String,
    pub latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checks: Option<Vec<ReadyCheck>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Query /api/ready for every service. Returns a CommandOutput with the
/// full results in `data`.
pub async fn fleet_health() -> Result<CommandOutput> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;

    let mut results = Vec::new();

    for svc in SERVICES {
        let result = probe_service(&client, svc).await;
        results.push(result);
    }

    let total = results.len();
    let ready = results.iter().filter(|r| r.status == "ready").count();
    let down = results.iter().filter(|r| r.status == "down").count();
    let unreachable = results.iter().filter(|r| !r.reachable).count();

    let overall = if unreachable > 0 || down > 0 {
        "degraded"
    } else {
        "healthy"
    };

    // Print human-readable table to stderr (visible even in --json mode)
    print_health_table(&results);

    let data = serde_json::json!({
        "total_services": total,
        "ready": ready,
        "down": down,
        "unreachable": unreachable,
        "overall": overall,
        "services": results,
    });

    let out = if overall == "healthy" {
        CommandOutput::ok("fleet-health", "-")
            .with_state(overall)
            .with_data(data)
    } else {
        CommandOutput::fail("fleet-health", "-", &format!(
            "{} down, {} unreachable of {} services",
            down, unreachable, total
        ))
        .with_state(overall)
        .with_data(data)
    };

    Ok(out)
}

async fn probe_service(client: &reqwest::Client, svc: &ServiceEndpoint) -> ServiceHealthResult {
    let url = format!("http://localhost:{}/api/ready", svc.port);
    let start = Instant::now();

    match client.get(&url).send().await {
        Ok(resp) => {
            let latency = start.elapsed().as_millis() as u64;
            match resp.json::<ReadyResponse>().await {
                Ok(ready) => ServiceHealthResult {
                    name: svc.name.to_string(),
                    port: svc.port,
                    reachable: true,
                    status: ready.status.clone(),
                    latency_ms: latency,
                    version: Some(ready.version),
                    checks: Some(ready.checks),
                    error: None,
                },
                Err(e) => ServiceHealthResult {
                    name: svc.name.to_string(),
                    port: svc.port,
                    reachable: true,
                    status: "down".to_string(),
                    latency_ms: latency,
                    version: None,
                    checks: None,
                    error: Some(format!("Invalid response: {}", e)),
                },
            }
        }
        Err(e) => ServiceHealthResult {
            name: svc.name.to_string(),
            port: svc.port,
            reachable: false,
            status: "unreachable".to_string(),
            latency_ms: start.elapsed().as_millis() as u64,
            version: None,
            checks: None,
            error: Some(e.to_string()),
        },
    }
}

fn print_health_table(results: &[ServiceHealthResult]) {
    eprintln!();
    eprintln!("{:<18} {:>5}  {:<12} {:>6}  {}", "SERVICE", "PORT", "STATUS", "MS", "VERSION");
    eprintln!("{}", "-".repeat(65));

    for r in results {
        let icon = match r.status.as_str() {
            "ready" => "✓",
            "degraded" => "~",
            _ => "✗",
        };
        let version = r.version.as_deref().unwrap_or("-");
        eprintln!(
            "{} {:<16} {:>5}  {:<12} {:>5}  {}",
            icon, r.name, r.port, r.status, r.latency_ms, version
        );
    }
    eprintln!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn services_list_complete() {
        assert_eq!(SERVICES.len(), 17);
        assert!(SERVICES.iter().any(|s| s.name == "ar"));
        assert!(SERVICES.iter().any(|s| s.name == "identity-auth"));
        assert!(SERVICES.iter().any(|s| s.name == "control-plane"));
    }

    #[test]
    fn service_ports_unique() {
        let mut ports: Vec<u16> = SERVICES.iter().map(|s| s.port).collect();
        ports.sort();
        ports.dedup();
        assert_eq!(ports.len(), SERVICES.len(), "Service ports must be unique");
    }

    #[test]
    fn service_health_result_serializes() {
        let r = ServiceHealthResult {
            name: "ar".to_string(),
            port: 8086,
            reachable: true,
            status: "ready".to_string(),
            latency_ms: 5,
            version: Some("0.1.0".to_string()),
            checks: Some(vec![ReadyCheck {
                name: "database".to_string(),
                status: "up".to_string(),
                latency_ms: 3,
                error: None,
            }]),
            error: None,
        };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["status"], "ready");
        assert!(!json.to_string().contains("\"error\""));
    }
}
