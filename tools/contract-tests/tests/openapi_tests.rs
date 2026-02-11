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

#[test]
fn test_payments_openapi_spec_valid() {
    let spec_path = contracts_dir().join("payments/payments-v0.1.0.yaml");

    let spec = validate_openapi_spec(&spec_path)
        .expect("Failed to parse payments OpenAPI spec");

    println!("✓ Payments OpenAPI spec is valid YAML");

    // Check required paths
    let required_paths = vec![
        "/api/health",
        "/api/payment-methods",
        "/api/refunds",
    ];

    check_required_paths(&spec, &required_paths, "payments-v0.1.0.yaml")
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

    // AR module should have at minimum these paths
    let required_paths = vec![
        "/api/ar/customers",
        "/api/ar/invoices",
    ];

    check_required_paths(&spec, &required_paths, "ar-v1.yaml")
        .expect("AR spec missing required paths");

    println!("✓ AR spec contains all required paths");
}

#[test]
fn test_auth_openapi_spec_valid() {
    let spec_path = contracts_dir().join("auth/auth-v1.yaml");

    let spec = validate_openapi_spec(&spec_path)
        .expect("Failed to parse Auth OpenAPI spec");

    println!("✓ Auth OpenAPI spec is valid YAML");

    // Auth module should have at minimum these paths
    let required_paths = vec![
        "/health/live",
        "/api/auth/login",
    ];

    check_required_paths(&spec, &required_paths, "auth-v1.yaml")
        .expect("Auth spec missing required paths");

    println!("✓ Auth spec contains all required paths");
}
