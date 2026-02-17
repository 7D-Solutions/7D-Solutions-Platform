//! E2E test: Version endpoints smoke test
//!
//! Phase 16: Verify all 5 modules expose /api/version endpoints
//!
//! This test verifies that each module:
//! 1. Responds to GET /api/version with 200 OK
//! 2. Returns JSON with module_name, module_version, schema_version
//! 3. Schema version matches expected format (YYYYMMDDNNNNNN)
//!
//! Modules tested:
//! - AR (port 8086)
//! - Payments (port 8088)
//! - Subscriptions (port 8087)
//! - Notifications (port 8089)
//! - GL (port 8085)

use serde_json::Value;

#[tokio::test]
#[serial_test::serial]
async fn test_version_endpoints_all_modules() -> Result<(), Box<dyn std::error::Error>> {
    // Module configurations: (name, port)
    // Note: exact schema_version values are not asserted here — they advance with each migration.
    // Format is validated below (14 digits: YYYYMMDDNNNNNN).
    let modules = vec![
        ("ar-rs", 8086),
        ("gl-rs", 8085),
        ("payments-rs", 8088),
        ("subscriptions-rs", 8087),
        ("notifications-rs", 8089),
    ];

    let client = reqwest::Client::new();

    for (module_name, port) in modules {
        let url = format!("http://localhost:{}/api/version", port);

        println!("Testing {} at {}", module_name, url);

        // Make request to version endpoint
        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to request {}: {}", url, e))?;

        // Verify 200 OK
        assert_eq!(
            response.status(),
            200,
            "{} version endpoint should return 200 OK",
            module_name
        );

        // Parse JSON response
        let json: Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse JSON from {}: {}", module_name, e))?;

        // Verify module_name field
        let actual_module_name = json["module_name"]
            .as_str()
            .ok_or_else(|| format!("{} missing module_name field", module_name))?;
        assert_eq!(
            actual_module_name, module_name,
            "{} module_name should match",
            module_name
        );

        // Verify module_version field exists
        let module_version = json["module_version"]
            .as_str()
            .ok_or_else(|| format!("{} missing module_version field", module_name))?;
        assert!(
            !module_version.is_empty(),
            "{} module_version should not be empty",
            module_name
        );

        // Verify schema_version field
        let schema_version = json["schema_version"]
            .as_str()
            .ok_or_else(|| format!("{} missing schema_version field", module_name))?;

        // Verify schema_version format (14 digits: YYYYMMDDNNNNNN)
        assert_eq!(
            schema_version.len(),
            14,
            "{} schema_version should be 14 characters",
            module_name
        );
        assert!(
            schema_version.chars().all(|c| c.is_ascii_digit()),
            "{} schema_version should be all digits",
            module_name
        );

        println!("✅ {} version endpoint OK: v{} (schema: {})",
            module_name, module_version, schema_version);
    }

    println!("\n✅ All 5 modules expose /api/version endpoints correctly");

    Ok(())
}
