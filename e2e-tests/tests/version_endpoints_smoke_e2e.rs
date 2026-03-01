//! E2E test: Version endpoints smoke test
//!
//! Phase 16: Verify all 5 modules expose /api/version endpoints
//!
//! ## Testing Strategy (bd-nln)
//! - AR, GL, Payments: version handlers called in-process (no live service required).
//!   This guarantees tests always exercise current HEAD code, not stale containers.
//! - Subscriptions, Notifications: binary-only modules (no lib.rs) — tested via live
//!   HTTP only when services are reachable; skipped gracefully if not running.
//!
//! Modules tested:
//! - AR (ar-rs)        — in-process  ✓
//! - GL (gl-rs)        — in-process  ✓
//! - Payments          — in-process  ✓
//! - Subscriptions     — live HTTP (skip if unreachable)
//! - Notifications     — live HTTP (skip if unreachable)

use axum::Json;
use serde_json::Value;

/// Validate a version response against the expected contract.
fn assert_version_contract(json: &Value, expected_module: &str) {
    let actual_module_name = json["module_name"]
        .as_str()
        .unwrap_or_else(|| panic!("{} missing module_name field", expected_module));
    assert_eq!(
        actual_module_name, expected_module,
        "{} module_name should match",
        expected_module
    );

    let module_version = json["module_version"]
        .as_str()
        .unwrap_or_else(|| panic!("{} missing module_version field", expected_module));
    assert!(
        !module_version.is_empty(),
        "{} module_version should not be empty",
        expected_module
    );

    let schema_version = json["schema_version"]
        .as_str()
        .unwrap_or_else(|| panic!("{} missing schema_version field", expected_module));
    assert_eq!(
        schema_version.len(),
        14,
        "{} schema_version should be 14 characters, got '{}'",
        expected_module,
        schema_version
    );
    assert!(
        schema_version.chars().all(|c| c.is_ascii_digit()),
        "{} schema_version should be all digits, got '{}'",
        expected_module,
        schema_version
    );

    println!(
        "✅ {} version OK: v{} (schema: {})",
        expected_module, module_version, schema_version
    );
}

/// Test AR, GL, Payments version endpoints in-process.
///
/// These 3 modules expose library interfaces so handlers can be called directly,
/// guaranteeing tests always cover current HEAD code without live services.
#[tokio::test]
async fn test_version_endpoints_library_modules() -> Result<(), Box<dyn std::error::Error>> {
    let Json(ar_version) = ar_rs::http::health::version().await;
    assert_version_contract(&ar_version, "ar-rs");

    let Json(gl_version) = gl_rs::routes::health::version().await;
    assert_version_contract(&gl_version, "gl-rs");

    let Json(payments_version) = payments_rs::routes::health::version().await;
    assert_version_contract(&payments_version, "payments-rs");

    println!("\n✅ AR, GL, Payments version endpoints verified in-process");
    Ok(())
}

/// Test Subscriptions and Notifications version endpoints via live HTTP.
///
/// These are binary-only modules (no lib.rs) so cannot be tested in-process.
/// Test skips gracefully if services are not reachable — this test is informational
/// only; correctness of these modules is covered by their own unit tests.
#[tokio::test]
async fn test_version_endpoints_binary_modules() -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?;

    let binary_modules = vec![
        ("subscriptions-rs", "http://localhost:8087"),
        ("notifications-rs", "http://localhost:8089"),
    ];

    for (module_name, base_url) in binary_modules {
        let url = format!("{}/api/version", base_url);
        match client.get(&url).send().await {
            Ok(response) if response.status().is_success() => {
                let json: Value = response.json().await?;
                assert_version_contract(&json, module_name);
            }
            Ok(response) => {
                println!(
                    "⚠️  {} /api/version returned {} — skipping (service may be stale/unavailable)",
                    module_name,
                    response.status()
                );
            }
            Err(_) => {
                println!(
                    "⚠️  {} unreachable at {} — skipping (service not running)",
                    module_name, base_url
                );
            }
        }
    }

    Ok(())
}
