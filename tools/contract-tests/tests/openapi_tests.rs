use contract_tests::*;
use std::path::PathBuf;

fn contracts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("contracts")
}

fn check_spec_version(spec: &serde_json::Value, expected_major: u64, spec_name: &str) {
    let version = spec
        .get("info")
        .and_then(|i| i.get("version"))
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.0");
    let major: u64 = version.split('.').next().unwrap_or("0").parse().unwrap_or(0);
    assert!(
        major >= expected_major,
        "{spec_name}: spec version {version} is below expected major {expected_major}"
    );
    println!("✓ {spec_name} version {version} (major >= {expected_major})");
}

#[test]
fn test_payments_openapi_spec_valid() {
    // payments-v1.0.0.yaml is the 1.0.0-proven spec (bead bd-1b1x)
    let spec_path = contracts_dir().join("payments/payments-v1.0.0.yaml");

    let spec = validate_openapi_spec(&spec_path)
        .expect("Failed to parse payments OpenAPI spec");

    println!("✓ Payments OpenAPI spec is valid YAML");
    check_spec_version(&spec, 1, "payments-v1.0.0.yaml");

    // Actual 1.0.0 endpoints: checkout sessions + webhook ingestion
    let required_paths = vec![
        "/api/health",
        "/api/payments/checkout-sessions",
        "/api/payments/checkout-sessions/{id}",
        "/api/payments/webhook/tilled",
    ];

    check_required_paths(&spec, &required_paths, "payments-v1.0.0.yaml")
        .expect("Payments spec missing required paths");

    println!("✓ Payments spec contains all required paths");
}

#[test]
fn test_notifications_openapi_spec_valid() {
    let spec_path = contracts_dir().join("notifications/notifications-v0.1.0.yaml");

    let spec = validate_openapi_spec(&spec_path)
        .expect("Failed to parse notifications OpenAPI spec");

    println!("✓ Notifications OpenAPI spec is valid YAML");

    // Check required paths
    let required_paths = vec![
        "/api/health",
        "/api/notifications/send",
        "/api/notifications/{notification_id}",
    ];

    check_required_paths(&spec, &required_paths, "notifications-v0.1.0.yaml")
        .expect("Notifications spec missing required paths");

    println!("✓ Notifications spec contains all required paths");
}

#[test]
fn test_subscriptions_openapi_spec_valid() {
    let spec_path = contracts_dir().join("subscriptions/subscriptions-v1.yaml");

    let spec = validate_openapi_spec(&spec_path)
        .expect("Failed to parse subscriptions OpenAPI spec");

    println!("✓ Subscriptions OpenAPI spec is valid YAML");

    // Check required paths
    let required_paths = vec![
        "/api/subscriptions",
        "/api/bill-runs/execute",
    ];

    check_required_paths(&spec, &required_paths, "subscriptions-v1.yaml")
        .expect("Subscriptions spec missing required paths");

    println!("✓ Subscriptions spec contains all required paths");
}

#[test]
fn test_ar_openapi_spec_valid() {
    let spec_path = contracts_dir().join("ar/ar-v1.yaml");

    let spec = validate_openapi_spec(&spec_path)
        .expect("Failed to parse AR OpenAPI spec");

    println!("✓ AR OpenAPI spec is valid YAML");
    check_spec_version(&spec, 1, "ar-v1.yaml");

    // 1.0.0 core paths (customers, invoices, aging, credit notes, write-offs, tax)
    let required_paths = vec![
        "/api/ar/customers",
        "/api/ar/invoices",
        "/api/ar/invoices/{id}/credit-notes",
        "/api/ar/invoices/{id}/write-off",
        "/api/ar/aging",
        "/api/ar/payments/allocate",
        "/api/ar/tax/config/jurisdictions",
        "/healthz",
    ];

    check_required_paths(&spec, &required_paths, "ar-v1.yaml")
        .expect("AR spec missing required paths");

    println!("✓ AR spec contains all required 1.0.0 paths");
}

#[test]
fn test_auth_openapi_spec_valid() {
    let spec_path = contracts_dir().join("auth/auth-v1.yaml");

    let spec = validate_openapi_spec(&spec_path)
        .expect("Failed to parse Auth OpenAPI spec");

    println!("✓ Auth OpenAPI spec is valid YAML");
    check_spec_version(&spec, 1, "auth-v1.yaml");

    // Auth module endpoints — login, healthz (1.1.0+), readiness
    let required_paths = vec![
        "/health/live",
        "/api/auth/login",
        "/healthz",
        "/api/ready",
    ];

    check_required_paths(&spec, &required_paths, "auth-v1.yaml")
        .expect("Auth spec missing required paths");

    println!("✓ Auth spec contains all required paths");
}
