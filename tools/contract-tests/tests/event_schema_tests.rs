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
fn test_all_event_schemas_have_valid_examples() {
    let contracts = contracts_dir();

    match validate_event_contracts(&contracts) {
        Ok(validated) => {
            println!("✓ Validated {} event schemas with examples:", validated.len());
            for (schema, example) in &validated {
                println!("  ✓ {} -> {}", schema, example);
            }
            assert!(!validated.is_empty(), "No event schemas were validated");
        }
        Err(e) => {
            panic!("Event contract validation failed: {}", e);
        }
    }
}

#[test]
fn test_payments_payment_succeeded_example() {
    let contracts = contracts_dir();
    let schema_path = contracts.join("events/payments-payment-succeeded.v1.json");
    let example_path = contracts.join("events/examples/payments-payment-succeeded.v1.example.json");

    let schema = load_schema(&schema_path).expect("Failed to load schema");
    let example = load_example(&example_path).expect("Failed to load example");

    validate_example(&schema, &example, "payments-payment-succeeded.v1.example.json")
        .expect("Validation failed");
}

#[test]
fn test_payments_payment_failed_example() {
    let contracts = contracts_dir();
    let schema_path = contracts.join("events/payments-payment-failed.v1.json");
    let example_path = contracts.join("events/examples/payments-payment-failed.v1.example.json");

    let schema = load_schema(&schema_path).expect("Failed to load schema");
    let example = load_example(&example_path).expect("Failed to load example");

    validate_example(&schema, &example, "payments-payment-failed.v1.example.json")
        .expect("Validation failed");
}

#[test]
fn test_payments_refund_succeeded_example() {
    let contracts = contracts_dir();
    let schema_path = contracts.join("events/payments-refund-succeeded.v1.json");
    let example_path = contracts.join("events/examples/payments-refund-succeeded.v1.example.json");

    let schema = load_schema(&schema_path).expect("Failed to load schema");
    let example = load_example(&example_path).expect("Failed to load example");

    validate_example(&schema, &example, "payments-refund-succeeded.v1.example.json")
        .expect("Validation failed");
}

#[test]
fn test_payments_refund_failed_example() {
    let contracts = contracts_dir();
    let schema_path = contracts.join("events/payments-refund-failed.v1.json");
    let example_path = contracts.join("events/examples/payments-refund-failed.v1.example.json");

    let schema = load_schema(&schema_path).expect("Failed to load schema");
    let example = load_example(&example_path).expect("Failed to load example");

    validate_example(&schema, &example, "payments-refund-failed.v1.example.json")
        .expect("Validation failed");
}

#[test]
fn test_notifications_delivery_succeeded_example() {
    let contracts = contracts_dir();
    let schema_path = contracts.join("events/notifications-delivery-succeeded.v1.json");
    let example_path = contracts.join("events/examples/notifications-delivery-succeeded.v1.example.json");

    let schema = load_schema(&schema_path).expect("Failed to load schema");
    let example = load_example(&example_path).expect("Failed to load example");

    validate_example(&schema, &example, "notifications-delivery-succeeded.v1.example.json")
        .expect("Validation failed");
}

#[test]
fn test_notifications_delivery_failed_example() {
    let contracts = contracts_dir();
    let schema_path = contracts.join("events/notifications-delivery-failed.v1.json");
    let example_path = contracts.join("events/examples/notifications-delivery-failed.v1.example.json");

    let schema = load_schema(&schema_path).expect("Failed to load schema");
    let example = load_example(&example_path).expect("Failed to load example");

    validate_example(&schema, &example, "notifications-delivery-failed.v1.example.json")
        .expect("Validation failed");
}

#[test]
fn test_subscriptions_created_example() {
    let contracts = contracts_dir();
    let schema_path = contracts.join("events/subscriptions-created.v1.json");
    let example_path = contracts.join("events/examples/subscriptions-created.v1.example.json");

    let schema = load_schema(&schema_path).expect("Failed to load schema");
    let example = load_example(&example_path).expect("Failed to load example");

    validate_example(&schema, &example, "subscriptions-created.v1.example.json")
        .expect("Validation failed");
}

#[test]
fn test_subscriptions_paused_example() {
    let contracts = contracts_dir();
    let schema_path = contracts.join("events/subscriptions-paused.v1.json");
    let example_path = contracts.join("events/examples/subscriptions-paused.v1.example.json");

    let schema = load_schema(&schema_path).expect("Failed to load schema");
    let example = load_example(&example_path).expect("Failed to load example");

    validate_example(&schema, &example, "subscriptions-paused.v1.example.json")
        .expect("Validation failed");
}

#[test]
fn test_subscriptions_resumed_example() {
    let contracts = contracts_dir();
    let schema_path = contracts.join("events/subscriptions-resumed.v1.json");
    let example_path = contracts.join("events/examples/subscriptions-resumed.v1.example.json");

    let schema = load_schema(&schema_path).expect("Failed to load schema");
    let example = load_example(&example_path).expect("Failed to load example");

    validate_example(&schema, &example, "subscriptions-resumed.v1.example.json")
        .expect("Validation failed");
}

#[test]
fn test_subscriptions_billrun_executed_example() {
    let contracts = contracts_dir();
    let schema_path = contracts.join("events/subscriptions-billrun-executed.v1.json");
    let example_path = contracts.join("events/examples/subscriptions-billrun-executed.v1.example.json");

    let schema = load_schema(&schema_path).expect("Failed to load schema");
    let example = load_example(&example_path).expect("Failed to load example");

    validate_example(&schema, &example, "subscriptions-billrun-executed.v1.example.json")
        .expect("Validation failed");
}

#[test]
fn test_gl_posting_request_example() {
    let contracts = contracts_dir();
    let schema_path = contracts.join("events/gl-posting-request.v1.json");
    let example_path = contracts.join("events/examples/gl-posting-request.v1.example.json");

    let schema = load_schema(&schema_path).expect("Failed to load schema");
    let example = load_example(&example_path).expect("Failed to load example");

    validate_example(&schema, &example, "gl-posting-request.v1.example.json")
        .expect("Validation failed");
}

#[test]
fn test_gl_posting_accepted_example() {
    let contracts = contracts_dir();
    let schema_path = contracts.join("events/gl-posting-accepted.v1.json");
    let example_path = contracts.join("events/examples/gl-posting-accepted.v1.example.json");

    let schema = load_schema(&schema_path).expect("Failed to load schema");
    let example = load_example(&example_path).expect("Failed to load example");

    validate_example(&schema, &example, "gl-posting-accepted.v1.example.json")
        .expect("Validation failed");
}

#[test]
fn test_gl_posting_rejected_example() {
    let contracts = contracts_dir();
    let schema_path = contracts.join("events/gl-posting-rejected.v1.json");
    let example_path = contracts.join("events/examples/gl-posting-rejected.v1.example.json");

    let schema = load_schema(&schema_path).expect("Failed to load schema");
    let example = load_example(&example_path).expect("Failed to load example");

    validate_example(&schema, &example, "gl-posting-rejected.v1.example.json")
        .expect("Validation failed");
}
