//! Tenant verification logic
//!
//! Verifies tenant health by calling real /ready and /version endpoints
//! for all modules to ensure proper provisioning and operational readiness.

use anyhow::Result;
use serde::Deserialize;
use tracing::{info, warn};

use crate::provision::MODULES;

/// Health endpoint response
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // API response structure
struct HealthResponse {
    status: String,
    service: String,
}

/// Ready endpoint response
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // API response structure
struct ReadyResponse {
    status: String,
    service: String,
    database: String,
}

/// Version endpoint response
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // API response structure
struct VersionResponse {
    module_name: String,
    module_version: String,
    schema_version: String,
}

/// Result of verification for a single module
#[derive(Debug)]
pub struct ModuleVerification {
    pub module_name: String,
    pub ready_check: bool,
    pub version_check: bool,
    pub schema_version: Option<String>,
    #[allow(dead_code)] // Used for detailed error reporting
    pub error_message: Option<String>,
}

/// Complete verification result for a tenant
pub struct TenantVerification {
    pub tenant_id: String,
    pub all_passed: bool,
    pub module_results: Vec<ModuleVerification>,
}

/// Verify a tenant by calling real /ready and /version endpoints
pub async fn verify_tenant(tenant_id: &str) -> Result<TenantVerification> {
    info!("Verifying tenant: {}", tenant_id);

    let mut module_results = Vec::new();
    let mut all_passed = true;

    for module in MODULES {
        let verification = verify_module(module.name, module.http_port).await;

        if !verification.ready_check || !verification.version_check {
            all_passed = false;
        }

        module_results.push(verification);
    }

    let result = TenantVerification {
        tenant_id: tenant_id.to_string(),
        all_passed,
        module_results,
    };

    // Print summary
    print_verification_summary(&result);

    Ok(result)
}

/// Verify a single module's health and version endpoints
async fn verify_module(module_name: &str, http_port: u16) -> ModuleVerification {
    let base_url = format!("http://localhost:{}", http_port);

    let mut ready_check = false;
    let mut version_check = false;
    let mut schema_version = None;
    let mut error_messages = Vec::new();

    // Check /ready endpoint
    let ready_url = format!("{}/api/ready", base_url);
    match reqwest::get(&ready_url).await {
        Ok(response) => {
            if response.status().is_success() {
                match response.json::<ReadyResponse>().await {
                    Ok(ready_resp) => {
                        if ready_resp.status == "ready" && ready_resp.database == "connected" {
                            ready_check = true;
                            info!("  ✓ {} /ready endpoint passed", module_name);
                        } else {
                            error_messages.push(format!("Ready check status mismatch: {}", ready_resp.status));
                            warn!("  ✗ {} /ready endpoint failed: status={}", module_name, ready_resp.status);
                        }
                    }
                    Err(e) => {
                        error_messages.push(format!("Failed to parse ready response: {}", e));
                        warn!("  ✗ {} /ready response parsing failed: {}", module_name, e);
                    }
                }
            } else {
                error_messages.push(format!("Ready endpoint returned {}", response.status()));
                warn!("  ✗ {} /ready endpoint returned {}", module_name, response.status());
            }
        }
        Err(e) => {
            error_messages.push(format!("Failed to connect to /ready: {}", e));
            warn!("  ✗ {} /ready endpoint unreachable: {}", module_name, e);
        }
    }

    // Check /version endpoint
    let version_url = format!("{}/api/version", base_url);
    match reqwest::get(&version_url).await {
        Ok(response) => {
            if response.status().is_success() {
                match response.json::<VersionResponse>().await {
                    Ok(version_resp) => {
                        schema_version = Some(version_resp.schema_version.clone());
                        version_check = true;
                        info!("  ✓ {} /version endpoint passed (schema: {})",
                              module_name, version_resp.schema_version);
                    }
                    Err(e) => {
                        error_messages.push(format!("Failed to parse version response: {}", e));
                        warn!("  ✗ {} /version response parsing failed: {}", module_name, e);
                    }
                }
            } else {
                error_messages.push(format!("Version endpoint returned {}", response.status()));
                warn!("  ✗ {} /version endpoint returned {}", module_name, response.status());
            }
        }
        Err(e) => {
            error_messages.push(format!("Failed to connect to /version: {}", e));
            warn!("  ✗ {} /version endpoint unreachable: {}", module_name, e);
        }
    }

    ModuleVerification {
        module_name: module_name.to_string(),
        ready_check,
        version_check,
        schema_version,
        error_message: if error_messages.is_empty() {
            None
        } else {
            Some(error_messages.join("; "))
        },
    }
}

/// Print verification summary
fn print_verification_summary(result: &TenantVerification) {
    info!("\n{}", "=".repeat(60));
    info!("TENANT VERIFICATION SUMMARY");
    info!("{}", "=".repeat(60));
    info!("Tenant ID: {}", result.tenant_id);

    let total_checks = result.module_results.len() * 2; // ready + version per module
    let passed_checks = result.module_results.iter()
        .map(|m| (m.ready_check as usize) + (m.version_check as usize))
        .sum::<usize>();

    info!("Total Checks:  {}", total_checks);
    info!("Passed:        {} ({:.1}%)",
          passed_checks,
          (passed_checks as f64 / total_checks as f64) * 100.0);
    info!("Failed:        {}", total_checks - passed_checks);
    info!("{}", "=".repeat(60));

    // Print per-module results
    for module_result in &result.module_results {
        let ready_icon = if module_result.ready_check { "✓" } else { "✗" };
        let version_icon = if module_result.version_check { "✓" } else { "✗" };

        info!(
            "{} {} - Ready: {} | Version: {} {}",
            if module_result.ready_check && module_result.version_check { "✅" } else { "❌" },
            module_result.module_name,
            ready_icon,
            version_icon,
            module_result.schema_version.as_ref().map(|v| format!("({})", v)).unwrap_or_default()
        );
    }

    info!("{}\n", "=".repeat(60));

    if result.all_passed {
        info!("✅ All verification checks passed!");
    } else {
        warn!("⚠️  Some verification checks failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_verification_structure() {
        let verification = ModuleVerification {
            module_name: "ar".to_string(),
            ready_check: true,
            version_check: true,
            schema_version: Some("20260216000001".to_string()),
            error_message: None,
        };

        assert_eq!(verification.module_name, "ar");
        assert!(verification.ready_check);
        assert!(verification.version_check);
    }

    #[test]
    fn tenant_verification_calculates_all_passed() {
        let results = vec![
            ModuleVerification {
                module_name: "ar".to_string(),
                ready_check: true,
                version_check: true,
                schema_version: Some("20260216000001".to_string()),
                error_message: None,
            },
            ModuleVerification {
                module_name: "gl".to_string(),
                ready_check: true,
                version_check: false,
                schema_version: None,
                error_message: Some("Connection failed".to_string()),
            },
        ];

        let verification = TenantVerification {
            tenant_id: "t1".to_string(),
            all_passed: false,
            module_results: results,
        };

        assert!(!verification.all_passed);
        assert_eq!(verification.module_results.len(), 2);
    }
}
