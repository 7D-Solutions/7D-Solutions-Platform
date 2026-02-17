/// E2E test for audit field-level diff tracking
///
/// Verifies that:
/// 1. Field-level diffs are captured for mutable_with_audit entities
/// 2. Diffs are deterministic (stable ordering across runs)
/// 3. Audit entries include actor + trace/correlation metadata
/// 4. Field changes are correctly recorded in audit events

mod common;

use audit::{
    actor::Actor,
    diff::Diff,
    schema::{MutationClass, WriteAuditRequest},
    writer::AuditWriter,
};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// Helper to set up the audit database pool (delegates to common helper with fallback URL)
async fn get_audit_pool() -> PgPool {
    common::get_audit_pool().await
}

// Shared migration helper with pg_advisory_lock lives in common/mod.rs.
// Use common::run_audit_migrations(pool) to avoid 40P01 catalog deadlocks.

#[tokio::test]
async fn test_field_diff_single_update() {
    let pool = get_audit_pool().await;
    common::run_audit_migrations(&pool).await;

    let writer = AuditWriter::new(pool.clone());
    let test_id = Uuid::new_v4(); // unique per run — prevents stale-data count interference

    let actor = Actor::user(Uuid::new_v4());
    let correlation_id = Uuid::new_v4();
    let trace_id = "trace-single-update";
    let entity_id = format!("cust_{}", test_id);

    // Simulate a customer name update
    let before = json!({
        "customer_id": "cust_123",
        "name": "John Doe",
        "email": "john@example.com",
        "status": "active"
    });

    let after = json!({
        "customer_id": "cust_123",
        "name": "Jane Doe",
        "email": "john@example.com",
        "status": "active"
    });

    // Compute diff
    let diff = Diff::new(Some(before.clone()), Some(after.clone()));

    // Create audit request with field diff
    let request = WriteAuditRequest::new(
        actor.id,
        actor.actor_type_str(),
        "UpdateCustomer".to_string(),
        MutationClass::Update,
        "Customer".to_string(),
        entity_id.clone(),
    )
    .with_snapshots(Some(before), Some(after))
    .with_correlation(None, Some(correlation_id), Some(trace_id.to_string()))
    .with_metadata(json!({
        "field_changes": diff.field_changes,
        "changed_field_count": diff.changed_field_count()
    }));

    let audit_id = writer
        .write(request)
        .await
        .expect("Failed to write audit event");

    // Verify the audit event was written with correct metadata
    let events = writer
        .get_by_entity("Customer", &entity_id)
        .await
        .expect("Failed to query audit events");

    assert_eq!(events.len(), 1);
    let event = &events[0];

    assert_eq!(event.audit_id, audit_id);
    assert_eq!(event.action, "UpdateCustomer");
    assert_eq!(event.mutation_class, MutationClass::Update);
    assert_eq!(event.actor_id, actor.id);
    assert_eq!(event.actor_type, actor.actor_type_str());
    assert_eq!(event.correlation_id, Some(correlation_id));
    assert_eq!(event.trace_id, Some(trace_id.to_string()));

    // Verify metadata contains field changes
    let metadata = event.metadata.as_ref().expect("Metadata should exist");
    let field_changes = metadata["field_changes"]
        .as_array()
        .expect("field_changes should be an array");
    let changed_count = metadata["changed_field_count"]
        .as_u64()
        .expect("changed_field_count should be a number");

    assert_eq!(changed_count, 1);
    assert_eq!(field_changes.len(), 1);

    // Verify the specific field change
    let change = &field_changes[0];
    assert_eq!(change["field"], "name");
    assert_eq!(change["old_value"], "John Doe");
    assert_eq!(change["new_value"], "Jane Doe");
}

#[tokio::test]
async fn test_field_diff_multiple_updates() {
    let pool = get_audit_pool().await;
    common::run_audit_migrations(&pool).await;

    let writer = AuditWriter::new(pool.clone());
    let test_id = Uuid::new_v4(); // unique per run — prevents stale-data count interference

    let actor = Actor::service("billing-service");
    let correlation_id = Uuid::new_v4();
    let entity_id = format!("inv_{}", test_id);

    let before = json!({
        "invoice_id": "inv_456",
        "status": "draft",
        "amount_cents": 10000,
        "due_date": "2026-03-01",
        "customer_id": "cust_789"
    });

    let after = json!({
        "invoice_id": "inv_456",
        "status": "finalized",
        "amount_cents": 12000,
        "due_date": "2026-03-01",
        "customer_id": "cust_789"
    });

    let diff = Diff::new(Some(before.clone()), Some(after.clone()));

    let request = WriteAuditRequest::new(
        actor.id,
        actor.actor_type_str(),
        "FinalizeInvoice".to_string(),
        MutationClass::StateTransition,
        "Invoice".to_string(),
        entity_id.clone(),
    )
    .with_snapshots(Some(before), Some(after))
    .with_correlation(None, Some(correlation_id), None)
    .with_metadata(json!({
        "field_changes": diff.field_changes,
        "changed_field_count": diff.changed_field_count()
    }));

    writer
        .write(request)
        .await
        .expect("Failed to write audit event");

    let events = writer
        .get_by_entity("Invoice", &entity_id)
        .await
        .expect("Failed to query audit events");

    assert_eq!(events.len(), 1);
    let event = &events[0];

    let metadata = event.metadata.as_ref().expect("Metadata should exist");
    let field_changes = metadata["field_changes"].as_array().unwrap();
    let changed_count = metadata["changed_field_count"].as_u64().unwrap();

    // Two fields changed: status and amount_cents
    assert_eq!(changed_count, 2);
    assert_eq!(field_changes.len(), 2);

    // Verify deterministic ordering (alphabetical: amount_cents, status)
    assert_eq!(field_changes[0]["field"], "amount_cents");
    assert_eq!(field_changes[0]["old_value"], 10000);
    assert_eq!(field_changes[0]["new_value"], 12000);

    assert_eq!(field_changes[1]["field"], "status");
    assert_eq!(field_changes[1]["old_value"], "draft");
    assert_eq!(field_changes[1]["new_value"], "finalized");
}

#[tokio::test]
async fn test_field_diff_deterministic_ordering() {
    // Test that field diffs are deterministically ordered across multiple runs
    let before = json!({
        "zebra": "old_z",
        "apple": "old_a",
        "middle": "old_m",
        "banana": "old_b"
    });

    let after = json!({
        "zebra": "new_z",
        "apple": "new_a",
        "middle": "new_m",
        "banana": "new_b"
    });

    // Run diff computation 5 times to ensure consistent ordering
    for _ in 0..5 {
        let diff = Diff::new(Some(before.clone()), Some(after.clone()));

        assert_eq!(diff.changed_field_count(), 4);

        let field_names: Vec<String> = diff
            .field_changes
            .iter()
            .map(|c| c.field.clone())
            .collect();

        // Should be alphabetically ordered
        assert_eq!(
            field_names,
            vec!["apple", "banana", "middle", "zebra"],
            "Field ordering should be deterministic and alphabetical"
        );
    }
}

#[tokio::test]
async fn test_field_diff_with_field_addition_and_removal() {
    let pool = get_audit_pool().await;
    common::run_audit_migrations(&pool).await;

    let writer = AuditWriter::new(pool.clone());
    let test_id = Uuid::new_v4(); // unique per run — prevents stale-data count interference

    let actor = Actor::system();
    let trace_id = "trace-field-mutation";
    let entity_id = format!("entity_{}", test_id);

    let before = json!({
        "id": "entity_1",
        "old_field": "will_be_removed",
        "unchanged": "stays"
    });

    let after = json!({
        "id": "entity_1",
        "unchanged": "stays",
        "new_field": "was_added"
    });

    let diff = Diff::new(Some(before.clone()), Some(after.clone()));

    let request = WriteAuditRequest::new(
        actor.id,
        actor.actor_type_str(),
        "MigrateEntity".to_string(),
        MutationClass::Update,
        "Entity".to_string(),
        entity_id.clone(),
    )
    .with_snapshots(Some(before), Some(after))
    .with_correlation(None, None, Some(trace_id.to_string()))
    .with_metadata(json!({
        "field_changes": diff.field_changes,
        "changed_field_count": diff.changed_field_count()
    }));

    writer
        .write(request)
        .await
        .expect("Failed to write audit event");

    let events = writer
        .get_by_entity("Entity", &entity_id)
        .await
        .expect("Failed to query audit events");

    assert_eq!(events.len(), 1);
    let event = &events[0];

    let metadata = event.metadata.as_ref().unwrap();
    let field_changes = metadata["field_changes"].as_array().unwrap();
    let changed_count = metadata["changed_field_count"].as_u64().unwrap();

    // Two changes: new_field added, old_field removed
    assert_eq!(changed_count, 2);
    assert_eq!(field_changes.len(), 2);

    // Deterministic ordering: new_field, old_field (alphabetical)
    assert_eq!(field_changes[0]["field"], "new_field");
    assert!(field_changes[0]["old_value"].is_null());
    assert_eq!(field_changes[0]["new_value"], "was_added");

    assert_eq!(field_changes[1]["field"], "old_field");
    assert_eq!(field_changes[1]["old_value"], "will_be_removed");
    assert!(field_changes[1]["new_value"].is_null());
}

#[tokio::test]
async fn test_field_diff_with_transaction() {
    let pool = get_audit_pool().await;
    common::run_audit_migrations(&pool).await;

    let test_id = Uuid::new_v4(); // unique per run — prevents stale-data count interference
    let entity_id = format!("acc_{}", test_id);
    let mut tx = pool.begin().await.expect("Failed to begin transaction");

    let actor = Actor::user(Uuid::new_v4());
    let correlation_id = Uuid::new_v4();

    let before = json!({
        "account_id": "acc_999",
        "balance_cents": 50000
    });

    let after = json!({
        "account_id": "acc_999",
        "balance_cents": 75000
    });

    let diff = Diff::new(Some(before.clone()), Some(after.clone()));

    let request = WriteAuditRequest::new(
        actor.id,
        actor.actor_type_str(),
        "UpdateBalance".to_string(),
        MutationClass::Update,
        "Account".to_string(),
        entity_id.clone(),
    )
    .with_snapshots(Some(before), Some(after))
    .with_correlation(None, Some(correlation_id), None)
    .with_metadata(json!({
        "field_changes": diff.field_changes,
        "changed_field_count": diff.changed_field_count()
    }));

    let audit_id = AuditWriter::write_in_tx(&mut tx, request)
        .await
        .expect("Failed to write in transaction");

    tx.commit().await.expect("Failed to commit transaction");

    // Verify the event was committed with field diff
    let writer = AuditWriter::new(pool);
    let events = writer
        .get_by_entity("Account", &entity_id)
        .await
        .expect("Failed to query audit events");

    assert_eq!(events.len(), 1);
    let event = &events[0];

    assert_eq!(event.audit_id, audit_id);

    let metadata = event.metadata.as_ref().unwrap();
    let changed_count = metadata["changed_field_count"].as_u64().unwrap();

    assert_eq!(changed_count, 1);
}

#[tokio::test]
async fn test_field_diff_no_changes() {
    // Test that no field changes are recorded when before == after
    let before = json!({
        "id": "unchanged_1",
        "status": "active"
    });

    let after = json!({
        "id": "unchanged_1",
        "status": "active"
    });

    let diff = Diff::new(Some(before), Some(after));

    assert_eq!(diff.changed_field_count(), 0);
    assert!(diff.is_modification());
    assert_eq!(diff.field_changes.len(), 0);
}
