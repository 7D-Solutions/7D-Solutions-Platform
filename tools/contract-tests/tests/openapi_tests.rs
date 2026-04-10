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
    let major: u64 = version
        .split('.')
        .next()
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);
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

    let spec = validate_openapi_spec(&spec_path).expect("Failed to parse payments OpenAPI spec");

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

    let spec =
        validate_openapi_spec(&spec_path).expect("Failed to parse notifications OpenAPI spec");

    println!("✓ Notifications OpenAPI spec is valid YAML");

    // Check required paths
    let required_paths = vec![
        "/api/health",
        "/api/notifications/send",
        "/api/notifications/{id}",
    ];

    check_required_paths(&spec, &required_paths, "notifications-v0.1.0.yaml")
        .expect("Notifications spec missing required paths");

    println!("✓ Notifications spec contains all required paths");
}

#[test]
fn test_subscriptions_openapi_spec_valid() {
    let spec_path = contracts_dir().join("subscriptions/subscriptions-v1.yaml");

    let spec =
        validate_openapi_spec(&spec_path).expect("Failed to parse subscriptions OpenAPI spec");

    println!("✓ Subscriptions OpenAPI spec is valid YAML");

    // Check required paths
    let required_paths = vec!["/api/subscriptions", "/api/bill-runs/execute"];

    check_required_paths(&spec, &required_paths, "subscriptions-v1.yaml")
        .expect("Subscriptions spec missing required paths");

    println!("✓ Subscriptions spec contains all required paths");
}

#[test]
fn test_ar_openapi_spec_valid() {
    let spec_path = contracts_dir().join("ar/ar-v1.yaml");

    let spec = validate_openapi_spec(&spec_path).expect("Failed to parse AR OpenAPI spec");

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

    let spec = validate_openapi_spec(&spec_path).expect("Failed to parse Auth OpenAPI spec");

    println!("✓ Auth OpenAPI spec is valid YAML");
    check_spec_version(&spec, 1, "auth-v1.yaml");

    // Auth module endpoints — login, healthz (1.1.0+), readiness
    let required_paths = vec!["/health/live", "/api/auth/login", "/healthz", "/api/ready"];

    check_required_paths(&spec, &required_paths, "auth-v1.yaml")
        .expect("Auth spec missing required paths");

    println!("✓ Auth spec contains all required paths");
}

#[test]
fn test_ttp_openapi_spec_valid() {
    let spec_path = contracts_dir().join("ttp/ttp-v1.0.0.yaml");

    let spec = validate_openapi_spec(&spec_path).expect("Failed to parse TTP OpenAPI spec");

    println!("✓ TTP OpenAPI spec is valid YAML");
    check_spec_version(&spec, 1, "ttp-v1.0.0.yaml");

    // All public TTP routes
    let required_paths = vec![
        "/healthz",
        "/api/health",
        "/api/ready",
        "/api/version",
        "/metrics",
        "/api/ttp/billing-runs",
        "/api/metering/events",
        "/api/metering/trace",
        "/api/ttp/service-agreements",
    ];

    check_required_paths(&spec, &required_paths, "ttp-v1.0.0.yaml")
        .expect("TTP spec missing required paths");

    println!("✓ TTP spec contains all required paths");
}

#[test]
fn test_control_plane_openapi_spec_valid() {
    let spec_path = contracts_dir().join("control-plane/control-plane-v1.0.0.yaml");

    let spec =
        validate_openapi_spec(&spec_path).expect("Failed to parse control-plane OpenAPI spec");

    println!("✓ Control-plane OpenAPI spec is valid YAML");
    check_spec_version(&spec, 1, "control-plane-v1.0.0.yaml");

    // Core control-plane routes + merged tenant-registry routes
    let required_paths = vec![
        "/healthz",
        "/api/ready",
        "/api/control/tenants",
        "/api/control/tenants/{tenant_id}/summary",
        "/api/control/tenants/{tenant_id}/retention",
        "/api/control/tenants/{tenant_id}/tombstone",
        "/api/control/platform-billing-runs",
        "/api/tenants/{tenant_id}/entitlements",
        "/api/tenants/{tenant_id}/app-id",
        "/api/tenants/{tenant_id}/status",
        "/api/ttp/plans",
        "/api/tenants",
        "/api/tenants/{tenant_id}",
    ];

    check_required_paths(&spec, &required_paths, "control-plane-v1.0.0.yaml")
        .expect("Control-plane spec missing required paths");

    println!("✓ Control-plane spec contains all required paths");
}

#[test]
fn test_tenant_registry_openapi_spec_valid() {
    let spec_path = contracts_dir().join("tenant-registry/tenant-registry-v1.0.2.yaml");

    let spec =
        validate_openapi_spec(&spec_path).expect("Failed to parse tenant-registry OpenAPI spec");

    println!("✓ Tenant-registry OpenAPI spec is valid YAML");
    check_spec_version(&spec, 1, "tenant-registry-v1.0.2.yaml");

    // All routes contributed by the tenant-registry library
    let required_paths = vec![
        "/api/control/tenants/{tenant_id}/summary",
        "/api/tenants/{tenant_id}/entitlements",
        "/api/tenants/{tenant_id}/app-id",
        "/api/tenants/{tenant_id}/status",
        "/api/ttp/plans",
        "/api/tenants",
        "/api/tenants/{tenant_id}",
    ];

    check_required_paths(&spec, &required_paths, "tenant-registry-v1.0.2.yaml")
        .expect("Tenant-registry spec missing required paths");

    println!("✓ Tenant-registry spec contains all required paths");
}

#[test]
fn test_inventory_openapi_spec_valid() {
    let spec_path = contracts_dir().join("inventory/inventory-v0.1.0.yaml");

    let spec = validate_openapi_spec(&spec_path).expect("Failed to parse inventory OpenAPI spec");

    println!("✓ Inventory OpenAPI spec is valid YAML");

    // Core inventory endpoints: ops, item master, movements, locations
    let required_paths = vec![
        "/healthz",
        "/api/health",
        "/api/ready",
        "/api/version",
        "/api/inventory/items",
        "/api/inventory/items/{id}",
        "/api/inventory/items/{id}/deactivate",
        "/api/inventory/receipts",
        "/api/inventory/issues",
        "/api/inventory/adjustments",
        "/api/inventory/transfers",
        "/api/inventory/reservations/reserve",
        "/api/inventory/reservations/release",
        "/api/inventory/locations",
        "/api/inventory/locations/{id}",
        "/api/inventory/uoms",
        "/api/inventory/valuation-snapshots",
        "/api/inventory/cycle-count-tasks",
    ];

    check_required_paths(&spec, &required_paths, "inventory-v0.1.0.yaml")
        .expect("Inventory spec missing required paths");

    println!("✓ Inventory spec contains all required paths");
}

#[test]
fn test_party_openapi_spec_valid() {
    let spec_path = contracts_dir().join("party/party-v0.1.0.yaml");

    let spec = validate_openapi_spec(&spec_path).expect("Failed to parse party OpenAPI spec");

    println!("✓ Party OpenAPI spec is valid YAML");

    // Core party endpoints: ops, parties, contacts, addresses
    let required_paths = vec![
        "/healthz",
        "/api/health",
        "/api/ready",
        "/api/version",
        "/api/party/companies",
        "/api/party/individuals",
        "/api/party/parties",
        "/api/party/parties/search",
        "/api/party/parties/{id}",
        "/api/party/parties/{id}/deactivate",
        "/api/party/parties/{party_id}/contacts",
        "/api/party/contacts/{id}",
        "/api/party/parties/{party_id}/addresses",
        "/api/party/addresses/{id}",
    ];

    check_required_paths(&spec, &required_paths, "party-v0.1.0.yaml")
        .expect("Party spec missing required paths");

    println!("✓ Party spec contains all required paths");
}

#[test]
fn test_integrations_hub_openapi_spec_valid() {
    let spec_path = contracts_dir().join("integrations-hub/integrations-hub-v0.1.0.yaml");

    let spec =
        validate_openapi_spec(&spec_path).expect("Failed to parse integrations-hub OpenAPI spec");

    println!("✓ Integrations-hub OpenAPI spec is valid YAML");

    // Core integrations-hub endpoints: ops, webhooks, external-refs, connectors
    let required_paths = vec![
        "/healthz",
        "/api/health",
        "/api/ready",
        "/api/version",
        "/api/webhooks/inbound/{system}",
        "/api/integrations/external-refs",
        "/api/integrations/external-refs/by-entity",
        "/api/integrations/external-refs/by-system",
        "/api/integrations/external-refs/{id}",
        "/api/integrations/connectors/types",
        "/api/integrations/connectors",
        "/api/integrations/connectors/{id}",
        "/api/integrations/connectors/{id}/test",
    ];

    check_required_paths(&spec, &required_paths, "integrations-hub-v0.1.0.yaml")
        .expect("Integrations-hub spec missing required paths");

    println!("✓ Integrations-hub spec contains all required paths");
}

#[test]
fn test_pdf_editor_openapi_spec_valid() {
    let spec_path = contracts_dir().join("pdf-editor/pdf-editor-v0.1.0.yaml");

    let spec = validate_openapi_spec(&spec_path).expect("Failed to parse pdf-editor OpenAPI spec");

    println!("✓ PDF Editor OpenAPI spec is valid YAML");

    // All implemented pdf-editor endpoints
    let required_paths = vec![
        // Ops
        "/healthz",
        "/api/health",
        "/api/ready",
        "/api/version",
        "/metrics",
        // PDF processing (stateless)
        "/api/pdf/render-annotations",
        "/api/pdf/forms/submissions/{id}/generate",
        // Form templates
        "/api/pdf/forms/templates",
        "/api/pdf/forms/templates/{id}",
        // Form fields
        "/api/pdf/forms/templates/{id}/fields",
        "/api/pdf/forms/templates/{tid}/fields/{fid}",
        "/api/pdf/forms/templates/{id}/fields/reorder",
        // Form submissions
        "/api/pdf/forms/submissions",
        "/api/pdf/forms/submissions/{id}",
        "/api/pdf/forms/submissions/{id}/submit",
    ];

    check_required_paths(&spec, &required_paths, "pdf-editor-v0.1.0.yaml")
        .expect("PDF Editor spec missing required paths");

    println!("✓ PDF Editor spec contains all required paths");
}

#[test]
fn test_ap_openapi_spec_valid() {
    let spec_path = contracts_dir().join("ap/openapi.json");

    let spec = validate_openapi_spec_json(&spec_path).expect("Failed to parse AP OpenAPI spec");

    println!("✓ AP OpenAPI spec is valid JSON");
    check_spec_version(&spec, 2, "ap/openapi.json");

    let required_paths = vec![
        "/api/ap/vendors",
        "/api/ap/vendors/{vendor_id}",
        "/api/ap/vendors/{vendor_id}/deactivate",
        "/api/ap/pos",
        "/api/ap/pos/{po_id}",
        "/api/ap/pos/{po_id}/approve",
        "/api/ap/bills",
        "/api/ap/bills/{bill_id}",
        "/api/ap/bills/{bill_id}/approve",
        "/api/ap/bills/{bill_id}/void",
        "/api/ap/payment-runs",
        "/api/ap/payment-runs/{run_id}",
        "/api/ap/payment-terms",
        "/api/ap/aging",
    ];

    check_required_paths(&spec, &required_paths, "ap/openapi.json")
        .expect("AP spec missing required paths");

    check_no_empty_schemas(&spec, "ap/openapi.json").expect("AP spec has empty schemas");

    println!("✓ AP spec contains all required paths with no empty schemas");
}

#[test]
fn test_bom_openapi_spec_valid() {
    let spec_path = contracts_dir().join("bom/openapi.json");

    let spec = validate_openapi_spec_json(&spec_path).expect("Failed to parse BOM OpenAPI spec");

    println!("✓ BOM OpenAPI spec is valid JSON");
    check_spec_version(&spec, 2, "bom/openapi.json");

    let required_paths = vec![
        "/api/bom",
        "/api/bom/{bom_id}",
        "/api/eco",
        "/api/eco/{eco_id}",
        "/api/eco/{eco_id}/submit",
        "/api/eco/{eco_id}/approve",
        "/api/eco/{eco_id}/reject",
        "/api/eco/{eco_id}/apply",
    ];

    check_required_paths(&spec, &required_paths, "bom/openapi.json")
        .expect("BOM spec missing required paths");

    check_no_empty_schemas(&spec, "bom/openapi.json").expect("BOM spec has empty schemas");

    println!("✓ BOM spec contains all required paths with no empty schemas");
}

#[test]
fn test_production_openapi_spec_valid() {
    let spec_path = contracts_dir().join("production/openapi.json");

    let spec =
        validate_openapi_spec_json(&spec_path).expect("Failed to parse Production OpenAPI spec");

    println!("✓ Production OpenAPI spec is valid JSON");
    check_spec_version(&spec, 2, "production/openapi.json");

    let required_paths = vec![
        "/api/production/workcenters",
        "/api/production/workcenters/{id}",
        "/api/production/work-orders",
        "/api/production/work-orders/{id}",
        "/api/production/work-orders/{id}/release",
        "/api/production/work-orders/{id}/close",
        "/api/production/routings",
        "/api/production/routings/{id}",
        "/api/production/time-entries/start",
        "/api/production/workcenters/{id}/downtime/start",
    ];

    check_required_paths(&spec, &required_paths, "production/openapi.json")
        .expect("Production spec missing required paths");

    check_no_empty_schemas(&spec, "production/openapi.json")
        .expect("Production spec has empty schemas");

    println!("✓ Production spec contains all required paths with no empty schemas");
}

#[test]
fn test_integrations_openapi_spec_valid() {
    let spec_path = contracts_dir().join("integrations/openapi.json");

    let spec = validate_openapi_spec_json(&spec_path)
        .expect("Failed to parse Integrations OpenAPI spec");

    println!("✓ Integrations OpenAPI spec is valid JSON");
    check_spec_version(&spec, 2, "integrations/openapi.json");

    let required_paths = vec![
        "/api/integrations/external-refs",
        "/api/integrations/external-refs/{id}",
        "/api/integrations/connectors/types",
        "/api/integrations/connectors",
        "/api/integrations/connectors/{id}",
        "/api/webhooks/inbound/{system}",
        "/api/integrations/oauth/connect/{provider}",
        "/api/integrations/oauth/status/{provider}",
    ];

    check_required_paths(&spec, &required_paths, "integrations/openapi.json")
        .expect("Integrations spec missing required paths");

    check_no_empty_schemas(&spec, "integrations/openapi.json")
        .expect("Integrations spec has empty schemas");

    println!("✓ Integrations spec contains all required paths with no empty schemas");
}

#[test]
fn test_shipping_receiving_openapi_spec_valid() {
    let spec_path = contracts_dir().join("shipping-receiving/openapi.json");

    let spec = validate_openapi_spec_json(&spec_path)
        .expect("Failed to parse Shipping & Receiving OpenAPI spec");

    println!("✓ Shipping & Receiving OpenAPI spec is valid JSON");
    check_spec_version(&spec, 3, "shipping-receiving/openapi.json");

    let required_paths = vec![
        "/api/shipping-receiving/shipments",
        "/api/shipping-receiving/shipments/{id}",
        "/api/shipping-receiving/shipments/{id}/lines",
        "/api/shipping-receiving/shipments/{id}/close",
        "/api/shipping-receiving/shipments/{id}/ship",
        "/api/shipping-receiving/po/{po_id}/shipments",
        "/api/shipping-receiving/shipments/{id}/routings",
    ];

    check_required_paths(&spec, &required_paths, "shipping-receiving/openapi.json")
        .expect("Shipping & Receiving spec missing required paths");

    check_no_empty_schemas(&spec, "shipping-receiving/openapi.json")
        .expect("Shipping & Receiving spec has empty schemas");

    println!("✓ Shipping & Receiving spec contains all required paths with no empty schemas");
}
