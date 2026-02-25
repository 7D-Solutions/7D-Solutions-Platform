//! Integration tests for the audit trail writer.
//!
//! Verifies that AuditWriter records mutations with correct actor, entity,
//! timestamp, snapshots, and correlation metadata against a real Postgres DB.

mod helpers;

use audit::{
    actor::Actor,
    diff::Diff,
    schema::{MutationClass, WriteAuditRequest},
    writer::AuditWriter,
};
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn writer_records_actor_and_entity_correctly() {
    let pool = helpers::get_audit_pool().await;
    helpers::run_audit_migrations(&pool).await;

    let writer = AuditWriter::new(pool.clone());
    let actor = Actor::user(Uuid::new_v4());
    let entity_id = format!("order_{}", Uuid::new_v4());

    let request = WriteAuditRequest::new(
        actor.id,
        actor.actor_type_str(),
        "CreateOrder".to_string(),
        MutationClass::Create,
        "Order".to_string(),
        entity_id.clone(),
    );

    let audit_id = writer.write(request).await.expect("write failed");

    let events = writer
        .get_by_entity("Order", &entity_id)
        .await
        .expect("query failed");

    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.audit_id, audit_id);
    assert_eq!(ev.actor_id, actor.id);
    assert_eq!(ev.actor_type, "User");
    assert_eq!(ev.action, "CreateOrder");
    assert_eq!(ev.mutation_class, MutationClass::Create);
    assert_eq!(ev.entity_type, "Order");
    assert_eq!(ev.entity_id, entity_id);
}

#[tokio::test]
async fn writer_records_occurred_at_timestamp() {
    let pool = helpers::get_audit_pool().await;
    helpers::run_audit_migrations(&pool).await;

    let writer = AuditWriter::new(pool.clone());
    let entity_id = format!("ts_{}", Uuid::new_v4());
    let before_write = chrono::Utc::now();

    let request = WriteAuditRequest::new(
        Uuid::new_v4(),
        "System".to_string(),
        "TimestampCheck".to_string(),
        MutationClass::Create,
        "TimestampTest".to_string(),
        entity_id.clone(),
    );

    writer.write(request).await.expect("write failed");
    let after_write = chrono::Utc::now();

    let events = writer
        .get_by_entity("TimestampTest", &entity_id)
        .await
        .expect("query failed");

    assert_eq!(events.len(), 1);
    let ts = events[0].occurred_at;
    assert!(
        ts >= before_write && ts <= after_write,
        "occurred_at {ts} should be between {before_write} and {after_write}"
    );
}

#[tokio::test]
async fn writer_stores_before_after_snapshots() {
    let pool = helpers::get_audit_pool().await;
    helpers::run_audit_migrations(&pool).await;

    let writer = AuditWriter::new(pool.clone());
    let entity_id = format!("snap_{}", Uuid::new_v4());

    let before = json!({"status": "draft", "amount": 100});
    let after = json!({"status": "finalized", "amount": 100});

    let request = WriteAuditRequest::new(
        Uuid::new_v4(),
        "Service".to_string(),
        "FinalizeInvoice".to_string(),
        MutationClass::StateTransition,
        "Invoice".to_string(),
        entity_id.clone(),
    )
    .with_snapshots(Some(before.clone()), Some(after.clone()));

    writer.write(request).await.expect("write failed");

    let events = writer
        .get_by_entity("Invoice", &entity_id)
        .await
        .expect("query failed");

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].before_snapshot, Some(before));
    assert_eq!(events[0].after_snapshot, Some(after));
}

#[tokio::test]
async fn writer_stores_correlation_metadata() {
    let pool = helpers::get_audit_pool().await;
    helpers::run_audit_migrations(&pool).await;

    let writer = AuditWriter::new(pool.clone());
    let entity_id = format!("corr_{}", Uuid::new_v4());
    let causation_id = Uuid::new_v4();
    let correlation_id = Uuid::new_v4();
    let trace_id = "trace-abc-123";

    let request = WriteAuditRequest::new(
        Uuid::new_v4(),
        "User".to_string(),
        "UpdateCustomer".to_string(),
        MutationClass::Update,
        "Customer".to_string(),
        entity_id.clone(),
    )
    .with_correlation(
        Some(causation_id),
        Some(correlation_id),
        Some(trace_id.to_string()),
    );

    writer.write(request).await.expect("write failed");

    let events = writer
        .get_by_entity("Customer", &entity_id)
        .await
        .expect("query failed");

    assert_eq!(events[0].causation_id, Some(causation_id));
    assert_eq!(events[0].correlation_id, Some(correlation_id));
    assert_eq!(events[0].trace_id, Some(trace_id.to_string()));
}

#[tokio::test]
async fn writer_stores_field_diff_in_metadata() {
    let pool = helpers::get_audit_pool().await;
    helpers::run_audit_migrations(&pool).await;

    let writer = AuditWriter::new(pool.clone());
    let entity_id = format!("diff_{}", Uuid::new_v4());

    let before = json!({"name": "Alice", "email": "old@test.com"});
    let after = json!({"name": "Alice", "email": "new@test.com"});
    let diff = Diff::new(Some(before.clone()), Some(after.clone()));

    let request = WriteAuditRequest::new(
        Uuid::new_v4(),
        "User".to_string(),
        "UpdateProfile".to_string(),
        MutationClass::Update,
        "Profile".to_string(),
        entity_id.clone(),
    )
    .with_snapshots(Some(before), Some(after))
    .with_metadata(json!({
        "field_changes": diff.field_changes,
        "changed_field_count": diff.changed_field_count()
    }));

    writer.write(request).await.expect("write failed");

    let events = writer
        .get_by_entity("Profile", &entity_id)
        .await
        .expect("query failed");

    let meta = events[0].metadata.as_ref().expect("metadata missing");
    assert_eq!(meta["changed_field_count"], 1);
    let changes = meta["field_changes"].as_array().unwrap();
    assert_eq!(changes[0]["field"], "email");
    assert_eq!(changes[0]["old_value"], "old@test.com");
    assert_eq!(changes[0]["new_value"], "new@test.com");
}

#[tokio::test]
async fn writer_in_tx_commits_on_success() {
    let pool = helpers::get_audit_pool().await;
    helpers::run_audit_migrations(&pool).await;

    let entity_id = format!("tx_ok_{}", Uuid::new_v4());
    let mut tx = pool.begin().await.expect("begin failed");

    let request = WriteAuditRequest::new(
        Uuid::new_v4(),
        "User".to_string(),
        "TxCommit".to_string(),
        MutationClass::Create,
        "TxTest".to_string(),
        entity_id.clone(),
    );

    let audit_id = AuditWriter::write_in_tx(&mut tx, request)
        .await
        .expect("write_in_tx failed");
    tx.commit().await.expect("commit failed");

    let writer = AuditWriter::new(pool);
    let events = writer
        .get_by_entity("TxTest", &entity_id)
        .await
        .expect("query failed");

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].audit_id, audit_id);
}

#[tokio::test]
async fn writer_in_tx_rolls_back_on_abort() {
    let pool = helpers::get_audit_pool().await;
    helpers::run_audit_migrations(&pool).await;

    let entity_id = format!("tx_rb_{}", Uuid::new_v4());
    let mut tx = pool.begin().await.expect("begin failed");

    let request = WriteAuditRequest::new(
        Uuid::new_v4(),
        "User".to_string(),
        "TxRollback".to_string(),
        MutationClass::Create,
        "TxTest".to_string(),
        entity_id.clone(),
    );

    AuditWriter::write_in_tx(&mut tx, request)
        .await
        .expect("write_in_tx failed");
    tx.rollback().await.expect("rollback failed");

    let writer = AuditWriter::new(pool);
    let events = writer
        .get_by_entity("TxTest", &entity_id)
        .await
        .expect("query failed");

    assert_eq!(events.len(), 0, "rolled-back event must not be visible");
}

#[tokio::test]
async fn writer_service_actor_has_deterministic_id() {
    let pool = helpers::get_audit_pool().await;
    helpers::run_audit_migrations(&pool).await;

    let writer = AuditWriter::new(pool.clone());
    let actor = Actor::service("billing-scheduler");
    let entity_id = format!("svc_{}", Uuid::new_v4());

    let request = WriteAuditRequest::new(
        actor.id,
        actor.actor_type_str(),
        "ScheduledBill".to_string(),
        MutationClass::Create,
        "Bill".to_string(),
        entity_id.clone(),
    );

    writer.write(request).await.expect("write failed");

    let events = writer
        .get_by_entity("Bill", &entity_id)
        .await
        .expect("query failed");

    // Same service name must always produce same actor_id
    let expected_id = Actor::service("billing-scheduler").id;
    assert_eq!(events[0].actor_id, expected_id);
    assert_eq!(events[0].actor_type, "Service");
}
