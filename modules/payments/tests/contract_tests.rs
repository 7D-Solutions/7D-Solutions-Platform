/// Contract tests for Payments module
///
/// These tests validate that the payments module's event schemas
/// are correct and that golden examples conform to those schemas.
///
/// This ensures that the events we produce will be consumable by
/// other modules (like AR) that depend on payments events.

use serde_json::Value;
use std::fs;
use std::path::PathBuf;

fn contracts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("contracts")
}

fn load_json_file(path: &PathBuf) -> Value {
    let contents = fs::read_to_string(path)
        .unwrap_or_else(|_| panic!("Failed to read file: {:?}", path));
    serde_json::from_str(&contents)
        .unwrap_or_else(|_| panic!("Failed to parse JSON: {:?}", path))
}

#[test]
fn test_payment_succeeded_example_has_valid_envelope() {
    let example_path = contracts_dir()
        .join("events/examples/payments-payment-succeeded.v1.example.json");

    let example: Value = load_json_file(&example_path);

    // Validate envelope fields are present
    assert!(example.get("event_id").is_some(), "Missing event_id");
    assert!(example.get("occurred_at").is_some(), "Missing occurred_at");
    assert!(example.get("tenant_id").is_some(), "Missing tenant_id");
    assert!(example.get("source_module").is_some(), "Missing source_module");
    assert!(example.get("source_version").is_some(), "Missing source_version");
    assert!(example.get("payload").is_some(), "Missing payload");

    // Validate source_module is correct
    assert_eq!(
        example.get("source_module").and_then(|v| v.as_str()),
        Some("payments"),
        "source_module should be 'payments'"
    );

    // Validate payload has required fields
    let payload = example.get("payload").unwrap();
    assert!(payload.get("payment_id").is_some(), "Missing payment_id");
    assert!(payload.get("invoice_id").is_some(), "Missing invoice_id");
    assert!(payload.get("amount_minor").is_some(), "Missing amount_minor");
    assert!(payload.get("currency").is_some(), "Missing currency");

    // Validate amount_minor is an integer
    assert!(
        payload.get("amount_minor").unwrap().is_i64(),
        "amount_minor should be an integer"
    );
}

#[test]
fn test_payment_failed_example_has_valid_envelope() {
    let example_path = contracts_dir()
        .join("events/examples/payments-payment-failed.v1.example.json");

    let example: Value = load_json_file(&example_path);

    // Validate envelope fields
    assert!(example.get("event_id").is_some(), "Missing event_id");
    assert!(example.get("occurred_at").is_some(), "Missing occurred_at");
    assert!(example.get("tenant_id").is_some(), "Missing tenant_id");
    assert_eq!(
        example.get("source_module").and_then(|v| v.as_str()),
        Some("payments")
    );

    // Validate failure fields
    let payload = example.get("payload").unwrap();
    assert!(payload.get("failure_code").is_some(), "Missing failure_code");
    assert!(payload.get("failure_message").is_some(), "Missing failure_message");
}

#[test]
fn test_refund_succeeded_example_has_valid_envelope() {
    let example_path = contracts_dir()
        .join("events/examples/payments-refund-succeeded.v1.example.json");

    let example: Value = load_json_file(&example_path);

    // Validate envelope
    assert!(example.get("event_id").is_some(), "Missing event_id");
    assert_eq!(
        example.get("source_module").and_then(|v| v.as_str()),
        Some("payments")
    );

    // Validate refund fields
    let payload = example.get("payload").unwrap();
    assert!(payload.get("refund_id").is_some(), "Missing refund_id");
    assert!(payload.get("payment_id").is_some(), "Missing payment_id");
    assert!(payload.get("amount_minor").is_some(), "Missing amount_minor");
}

#[test]
fn test_refund_failed_example_has_valid_envelope() {
    let example_path = contracts_dir()
        .join("events/examples/payments-refund-failed.v1.example.json");

    let example: Value = load_json_file(&example_path);

    // Validate envelope
    assert!(example.get("event_id").is_some(), "Missing event_id");
    assert_eq!(
        example.get("source_module").and_then(|v| v.as_str()),
        Some("payments")
    );

    // Validate failure fields
    let payload = example.get("payload").unwrap();
    assert!(payload.get("failure_code").is_some(), "Missing failure_code");
    assert!(payload.get("failure_message").is_some(), "Missing failure_message");
}

#[test]
fn test_all_payment_examples_use_minor_currency_units() {
    let examples = vec![
        "payments-payment-succeeded.v1.example.json",
        "payments-payment-failed.v1.example.json",
        "payments-refund-succeeded.v1.example.json",
        "payments-refund-failed.v1.example.json",
    ];

    for example_name in examples {
        let example_path = contracts_dir()
            .join("events/examples")
            .join(example_name);

        let example: Value = load_json_file(&example_path);
        let payload = example.get("payload").unwrap();

        // Verify amount_minor is integer
        assert!(
            payload.get("amount_minor").unwrap().is_i64(),
            "{} should use integer amount_minor",
            example_name
        );

        // Verify currency is 3-letter code
        let currency = payload.get("currency").unwrap().as_str().unwrap();
        assert_eq!(
            currency.len(),
            3,
            "{} currency should be 3 characters",
            example_name
        );
        assert!(
            currency.chars().all(|c| c.is_ascii_uppercase()),
            "{} currency should be uppercase",
            example_name
        );
    }
}
