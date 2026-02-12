//! Integration tests for GL posting request contract deserialization
//!
//! These tests verify that the contract types correctly deserialize
//! EventEnvelope<GlPostingRequestV1> from JSON matching the schema.

use event_bus::EventEnvelope;
use gl_rs::contracts::GlPostingRequestV1;
use gl_rs::validation::validate_gl_posting_request;

#[test]
fn test_deserialize_valid_event_envelope_from_example() {
    // This matches the schema from contracts/events/gl-posting-request.v1.json
    // Note: Using valid UUIDs (example JSON has ULIDs which don't parse as UUIDs)
    let json = r#"{
  "event_id": "550e8400-e29b-41d4-a716-446655440000",
  "occurred_at": "2024-02-11T16:00:00.000Z",
  "tenant_id": "tnt_01HPQW8YN4KJXR6S9TM2CP5V8H",
  "source_module": "ar",
  "source_version": "0.1.0",
  "correlation_id": "cor_01HPQZT7M2P9KY3W6V5R8XJ4N2T",
  "causation_id": "inv_01HPQW9K7J4M6N8P2R5T7V9W1X",
  "payload": {
    "posting_date": "2024-02-11",
    "currency": "USD",
    "source_doc_type": "AR_INVOICE",
    "source_doc_id": "inv_01HPQW9K7J4M6N8P2R5T7V9W1X",
    "description": "Invoice for customer services - February 2024",
    "lines": [
      {
        "account_ref": "1100",
        "debit": 2599.00,
        "credit": 0,
        "memo": "Accounts Receivable - Customer invoice",
        "dimensions": {
          "customer_id": "cus_01HPQW8Z5N7P9Q2R4T6V8W1X3Y"
        }
      },
      {
        "account_ref": "4000",
        "debit": 0,
        "credit": 2599.00,
        "memo": "Revenue - Professional services",
        "dimensions": {
          "customer_id": "cus_01HPQW8Z5N7P9Q2R4T6V8W1X3Y"
        }
      }
    ]
  }
}"#;

    // Deserialize the envelope
    let result: Result<EventEnvelope<GlPostingRequestV1>, _> = serde_json::from_str(json);
    assert!(
        result.is_ok(),
        "Failed to deserialize valid envelope: {:?}",
        result.err()
    );

    let envelope = result.unwrap();

    // Verify envelope fields
    assert_eq!(envelope.tenant_id, "tnt_01HPQW8YN4KJXR6S9TM2CP5V8H");
    assert_eq!(envelope.source_module, "ar");
    assert_eq!(envelope.source_version, "0.1.0");
    assert_eq!(
        envelope.correlation_id,
        Some("cor_01HPQZT7M2P9KY3W6V5R8XJ4N2T".to_string())
    );
    assert_eq!(
        envelope.causation_id,
        Some("inv_01HPQW9K7J4M6N8P2R5T7V9W1X".to_string())
    );

    // Verify payload
    let payload = &envelope.payload;
    assert_eq!(payload.posting_date, "2024-02-11");
    assert_eq!(payload.currency, "USD");
    assert_eq!(payload.source_doc_id, "inv_01HPQW9K7J4M6N8P2R5T7V9W1X");
    assert_eq!(
        payload.description,
        "Invoice for customer services - February 2024"
    );
    assert_eq!(payload.lines.len(), 2);

    // Verify line 1 (debit)
    assert_eq!(payload.lines[0].account_ref, "1100");
    assert_eq!(payload.lines[0].debit, 2599.00);
    assert_eq!(payload.lines[0].credit, 0.0);
    assert_eq!(
        payload.lines[0].memo,
        Some("Accounts Receivable - Customer invoice".to_string())
    );

    // Verify line 2 (credit)
    assert_eq!(payload.lines[1].account_ref, "4000");
    assert_eq!(payload.lines[1].debit, 0.0);
    assert_eq!(payload.lines[1].credit, 2599.00);

    // Validate the payload
    let validation_result = validate_gl_posting_request(&envelope.payload);
    assert!(
        validation_result.is_ok(),
        "Valid payload failed validation: {:?}",
        validation_result.err()
    );

    println!("✅ Successfully deserialized and validated EventEnvelope<GlPostingRequestV1>");
    println!("   Event ID: {}", envelope.event_id);
    println!("   Tenant: {}", envelope.tenant_id);
    println!("   Source: {}@{}", envelope.source_module, envelope.source_version);
    println!("   Posting Date: {}", payload.posting_date);
    println!("   Description: {}", payload.description);
    println!("   Lines: {} (balanced: {} debit, {} credit)",
        payload.lines.len(),
        payload.lines.iter().map(|l| l.debit).sum::<f64>(),
        payload.lines.iter().map(|l| l.credit).sum::<f64>()
    );
}

#[test]
fn test_reject_invalid_envelope_missing_required_field() {
    // Missing required field "description" in payload
    let json = r#"{
  "event_id": "550e8400-e29b-41d4-a716-446655440000",
  "occurred_at": "2024-02-11T16:00:00.000Z",
  "tenant_id": "tnt_01HPQW8YN4KJXR6S9TM2CP5V8H",
  "source_module": "ar",
  "source_version": "0.1.0",
  "payload": {
    "posting_date": "2024-02-11",
    "currency": "USD",
    "source_doc_type": "AR_INVOICE",
    "source_doc_id": "inv_123",
    "lines": [
      {
        "account_ref": "1100",
        "debit": 100.0,
        "credit": 0
      },
      {
        "account_ref": "4000",
        "debit": 0,
        "credit": 100.0
      }
    ]
  }
}"#;

    let result: Result<EventEnvelope<GlPostingRequestV1>, _> = serde_json::from_str(json);
    assert!(
        result.is_err(),
        "Should have failed: missing description field"
    );

    println!("✅ Correctly rejected invalid JSON (missing description):");
    println!("   Error: {:?}", result.err().unwrap());
}

#[test]
fn test_reject_invalid_currency() {
    let json = r#"{
  "event_id": "550e8400-e29b-41d4-a716-446655440001",
  "occurred_at": "2024-02-11T16:00:00.000Z",
  "tenant_id": "tnt_01HPQW8YN4KJXR6S9TM2CP5V8H",
  "source_module": "ar",
  "source_version": "0.1.0",
  "payload": {
    "posting_date": "2024-02-11",
    "currency": "usd",
    "source_doc_type": "AR_INVOICE",
    "source_doc_id": "inv_123",
    "description": "Test invoice",
    "lines": [
      {
        "account_ref": "1100",
        "debit": 100.0,
        "credit": 0
      },
      {
        "account_ref": "4000",
        "debit": 0,
        "credit": 100.0
      }
    ]
  }
}"#;

    let result: Result<EventEnvelope<GlPostingRequestV1>, _> = serde_json::from_str(json);
    assert!(result.is_ok(), "Deserialization should succeed");

    let envelope = result.unwrap();
    let validation_result = validate_gl_posting_request(&envelope.payload);

    assert!(
        validation_result.is_err(),
        "Should have failed validation: invalid currency"
    );

    println!("✅ Correctly rejected invalid currency (lowercase 'usd'):");
    println!("   Validation Error: {:?}", validation_result.err().unwrap());
}

#[test]
fn test_reject_unbalanced_entry() {
    let json = r#"{
  "event_id": "550e8400-e29b-41d4-a716-446655440002",
  "occurred_at": "2024-02-11T16:00:00.000Z",
  "tenant_id": "tnt_01HPQW8YN4KJXR6S9TM2CP5V8H",
  "source_module": "ar",
  "source_version": "0.1.0",
  "payload": {
    "posting_date": "2024-02-11",
    "currency": "USD",
    "source_doc_type": "AR_INVOICE",
    "source_doc_id": "inv_123",
    "description": "Test invoice",
    "lines": [
      {
        "account_ref": "1100",
        "debit": 100.0,
        "credit": 0
      },
      {
        "account_ref": "4000",
        "debit": 0,
        "credit": 50.0
      }
    ]
  }
}"#;

    let result: Result<EventEnvelope<GlPostingRequestV1>, _> = serde_json::from_str(json);
    assert!(result.is_ok(), "Deserialization should succeed");

    let envelope = result.unwrap();
    let validation_result = validate_gl_posting_request(&envelope.payload);

    assert!(
        validation_result.is_err(),
        "Should have failed validation: unbalanced entry"
    );

    println!("✅ Correctly rejected unbalanced entry (100 debit vs 50 credit):");
    println!("   Validation Error: {:?}", validation_result.err().unwrap());
}

#[test]
fn test_reject_empty_account_ref() {
    let json = r#"{
  "event_id": "550e8400-e29b-41d4-a716-446655440003",
  "occurred_at": "2024-02-11T16:00:00.000Z",
  "tenant_id": "tnt_01HPQW8YN4KJXR6S9TM2CP5V8H",
  "source_module": "ar",
  "source_version": "0.1.0",
  "payload": {
    "posting_date": "2024-02-11",
    "currency": "USD",
    "source_doc_type": "AR_INVOICE",
    "source_doc_id": "inv_123",
    "description": "Test invoice",
    "lines": [
      {
        "account_ref": "",
        "debit": 100.0,
        "credit": 0
      },
      {
        "account_ref": "4000",
        "debit": 0,
        "credit": 100.0
      }
    ]
  }
}"#;

    let result: Result<EventEnvelope<GlPostingRequestV1>, _> = serde_json::from_str(json);
    assert!(result.is_ok(), "Deserialization should succeed");

    let envelope = result.unwrap();
    let validation_result = validate_gl_posting_request(&envelope.payload);

    assert!(
        validation_result.is_err(),
        "Should have failed validation: empty account_ref"
    );

    println!("✅ Correctly rejected empty account_ref:");
    println!("   Validation Error: {:?}", validation_result.err().unwrap());
}
