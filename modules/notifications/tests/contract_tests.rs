/// Contract tests for Notifications module
///
/// These tests validate that the notifications module's event schemas
/// are correct and that golden examples conform to those schemas.

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
fn test_delivery_succeeded_example_has_valid_envelope() {
    let example_path = contracts_dir()
        .join("events/examples/notifications-delivery-succeeded.v1.example.json");

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
        Some("notifications"),
        "source_module should be 'notifications'"
    );

    // Validate payload has required fields
    let payload = example.get("payload").unwrap();
    assert!(payload.get("notification_id").is_some(), "Missing notification_id");
    assert!(payload.get("channel").is_some(), "Missing channel");
    assert!(payload.get("status").is_some(), "Missing status");

    // Validate status is succeeded
    assert_eq!(
        payload.get("status").unwrap().as_str().unwrap(),
        "succeeded",
        "Status should be 'succeeded'"
    );
}

#[test]
fn test_delivery_failed_example_has_valid_envelope() {
    let example_path = contracts_dir()
        .join("events/examples/notifications-delivery-failed.v1.example.json");

    let example: Value = load_json_file(&example_path);

    // Validate envelope fields
    assert!(example.get("event_id").is_some(), "Missing event_id");
    assert!(example.get("occurred_at").is_some(), "Missing occurred_at");
    assert!(example.get("tenant_id").is_some(), "Missing tenant_id");
    assert_eq!(
        example.get("source_module").and_then(|v| v.as_str()),
        Some("notifications")
    );

    // Validate failure fields
    let payload = example.get("payload").unwrap();
    assert!(payload.get("failure_code").is_some(), "Missing failure_code");
    assert!(payload.get("failure_message").is_some(), "Missing failure_message");
    assert!(payload.get("status").is_some(), "Missing status");

    // Validate status is failed
    assert_eq!(
        payload.get("status").unwrap().as_str().unwrap(),
        "failed",
        "Status should be 'failed'"
    );
}

#[test]
fn test_delivery_succeeded_has_valid_channels() {
    let example_path = contracts_dir()
        .join("events/examples/notifications-delivery-succeeded.v1.example.json");

    let example: Value = load_json_file(&example_path);
    let payload = example.get("payload").unwrap();
    let channel = payload.get("channel").unwrap().as_str().unwrap();

    // Validate channel is one of the expected values
    let valid_channels = vec!["email", "sms", "push", "webhook"];
    assert!(
        valid_channels.contains(&channel),
        "Channel should be one of: {:?}",
        valid_channels
    );
}

#[test]
fn test_delivery_failed_has_valid_channels() {
    let example_path = contracts_dir()
        .join("events/examples/notifications-delivery-failed.v1.example.json");

    let example: Value = load_json_file(&example_path);
    let payload = example.get("payload").unwrap();
    let channel = payload.get("channel").unwrap().as_str().unwrap();

    // Validate channel is one of the expected values
    let valid_channels = vec!["email", "sms", "push", "webhook"];
    assert!(
        valid_channels.contains(&channel),
        "Channel should be one of: {:?}",
        valid_channels
    );
}

#[test]
fn test_all_notification_examples_have_unique_event_ids() {
    let examples = vec![
        "notifications-delivery-succeeded.v1.example.json",
        "notifications-delivery-failed.v1.example.json",
    ];

    let mut event_ids = Vec::new();

    for example_name in examples {
        let example_path = contracts_dir()
            .join("events/examples")
            .join(example_name);

        let example: Value = load_json_file(&example_path);
        let event_id = example.get("event_id").unwrap().as_str().unwrap().to_string();

        assert!(
            !event_ids.contains(&event_id),
            "Duplicate event_id found: {}",
            event_id
        );

        event_ids.push(event_id);
    }
}
