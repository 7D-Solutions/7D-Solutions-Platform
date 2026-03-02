/// E2E test for audit log append-only enforcement
///
/// Verifies that the audit_events table:
/// 1. Can be written to (INSERT)
/// 2. Rejects UPDATE operations
/// 3. Rejects DELETE operations
mod common;

use audit::{
    schema::{MutationClass, WriteAuditRequest},
    writer::AuditWriter,
};
use sqlx::PgPool;
use uuid::Uuid;

/// Helper to set up the audit database pool (delegates to common helper with fallback URL)
async fn get_audit_pool() -> PgPool {
    common::get_audit_pool().await
}

// Shared migration helper with pg_advisory_lock lives in common/mod.rs.
// Use common::run_audit_migrations(pool) to avoid 40P01 catalog deadlocks.

#[tokio::test]
async fn test_audit_write_success() {
    let pool = get_audit_pool().await;
    common::run_audit_migrations(&pool).await;

    let writer = AuditWriter::new(pool.clone());
    let test_id = Uuid::new_v4(); // unique per run — prevents stale-data count interference

    let actor_id = Uuid::new_v4();
    let correlation_id = Uuid::new_v4();
    let entity_id = format!("inv_{}", test_id);

    let request = WriteAuditRequest::new(
        actor_id,
        "System".to_string(),
        "CreateInvoice".to_string(),
        MutationClass::Create,
        "Invoice".to_string(),
        entity_id.clone(),
    )
    .with_correlation(None, Some(correlation_id), Some("trace-abc".to_string()))
    .with_snapshots(
        None,
        Some(serde_json::json!({
            "id": "inv_12345",
            "amount_cents": 10000,
            "status": "draft"
        })),
    );

    let audit_id = writer
        .write(request)
        .await
        .expect("Failed to write audit event");

    // Verify the event was written
    let events = writer
        .get_by_entity("Invoice", &entity_id)
        .await
        .expect("Failed to query audit events");

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].audit_id, audit_id);
    assert_eq!(events[0].action, "CreateInvoice");
    assert_eq!(events[0].entity_type, "Invoice");
    assert_eq!(events[0].entity_id, entity_id);
    assert_eq!(events[0].mutation_class, MutationClass::Create);
    assert_eq!(events[0].actor_id, actor_id);
    assert!(events[0].after_snapshot.is_some());
}

#[tokio::test]
async fn test_audit_update_rejected() {
    let pool = get_audit_pool().await;
    common::run_audit_migrations(&pool).await;

    let writer = AuditWriter::new(pool.clone());

    // First, write an audit event
    let request = WriteAuditRequest::new(
        Uuid::new_v4(),
        "User".to_string(),
        "UpdateCustomer".to_string(),
        MutationClass::Update,
        "Customer".to_string(),
        "cust_789".to_string(),
    );

    let audit_id = writer
        .write(request)
        .await
        .expect("Failed to write audit event");

    // Attempt to UPDATE the audit event (should fail)
    let result = sqlx::query(
        r#"
        UPDATE audit_events
        SET action = 'TamperedAction'
        WHERE audit_id = $1
        "#,
    )
    .bind(audit_id)
    .execute(&pool)
    .await;

    assert!(result.is_err(), "UPDATE should be rejected");

    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("Audit log is append-only")
            || error_msg.contains("UPDATE")
            || error_msg.contains("forbidden"),
        "Error should indicate append-only violation. Got: {}",
        error_msg
    );
}

#[tokio::test]
async fn test_audit_delete_rejected() {
    let pool = get_audit_pool().await;
    common::run_audit_migrations(&pool).await;

    let writer = AuditWriter::new(pool.clone());

    // First, write an audit event
    let request = WriteAuditRequest::new(
        Uuid::new_v4(),
        "System".to_string(),
        "DeletePayment".to_string(),
        MutationClass::Delete,
        "Payment".to_string(),
        "pmt_456".to_string(),
    );

    let audit_id = writer
        .write(request)
        .await
        .expect("Failed to write audit event");

    // Attempt to DELETE the audit event (should fail)
    let result = sqlx::query(
        r#"
        DELETE FROM audit_events
        WHERE audit_id = $1
        "#,
    )
    .bind(audit_id)
    .execute(&pool)
    .await;

    assert!(result.is_err(), "DELETE should be rejected");

    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("Audit log is append-only")
            || error_msg.contains("DELETE")
            || error_msg.contains("forbidden"),
        "Error should indicate append-only violation. Got: {}",
        error_msg
    );
}

#[tokio::test]
async fn test_audit_correlation_query() {
    let pool = get_audit_pool().await;
    common::run_audit_migrations(&pool).await;

    let writer = AuditWriter::new(pool.clone());

    let correlation_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();

    // Write multiple events with the same correlation_id
    for i in 1..=3 {
        let request = WriteAuditRequest::new(
            actor_id,
            "Service".to_string(),
            format!("Step{}", i),
            MutationClass::StateTransition,
            "Workflow".to_string(),
            format!("wf_{}", i),
        )
        .with_correlation(None, Some(correlation_id), None);

        writer
            .write(request)
            .await
            .expect("Failed to write audit event");
    }

    // Query by correlation_id
    let events = writer
        .get_by_correlation(correlation_id)
        .await
        .expect("Failed to query by correlation");

    assert_eq!(events.len(), 3);
    assert_eq!(events[0].action, "Step1");
    assert_eq!(events[1].action, "Step2");
    assert_eq!(events[2].action, "Step3");
    assert!(events
        .iter()
        .all(|e| e.correlation_id == Some(correlation_id)));
}

#[tokio::test]
async fn test_audit_write_in_transaction() {
    let pool = get_audit_pool().await;
    common::run_audit_migrations(&pool).await;

    let test_id = Uuid::new_v4();
    let entity_id = format!("acc_{}", test_id);
    let mut tx = pool.begin().await.expect("Failed to begin transaction");

    let request = WriteAuditRequest::new(
        Uuid::new_v4(),
        "User".to_string(),
        "TransactionalUpdate".to_string(),
        MutationClass::Update,
        "Account".to_string(),
        entity_id.clone(),
    );

    let audit_id = AuditWriter::write_in_tx(&mut tx, request)
        .await
        .expect("Failed to write in transaction");

    // Commit the transaction
    tx.commit().await.expect("Failed to commit transaction");

    // Verify the event was committed
    let writer = AuditWriter::new(pool);
    let events = writer
        .get_by_entity("Account", &entity_id)
        .await
        .expect("Failed to query audit events");

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].audit_id, audit_id);
}

#[tokio::test]
async fn test_audit_transaction_rollback() {
    let pool = get_audit_pool().await;
    common::run_audit_migrations(&pool).await;

    let test_id = Uuid::new_v4();
    let entity_id = format!("test_rollback_{}", test_id);
    let mut tx = pool.begin().await.expect("Failed to begin transaction");

    let request = WriteAuditRequest::new(
        Uuid::new_v4(),
        "User".to_string(),
        "RollbackTest".to_string(),
        MutationClass::Create,
        "TestEntity".to_string(),
        entity_id.clone(),
    );

    let _audit_id = AuditWriter::write_in_tx(&mut tx, request)
        .await
        .expect("Failed to write in transaction");

    // Rollback the transaction
    tx.rollback().await.expect("Failed to rollback transaction");

    // Verify the event was NOT committed
    let writer = AuditWriter::new(pool);
    let events = writer
        .get_by_entity("TestEntity", &entity_id)
        .await
        .expect("Failed to query audit events");

    assert_eq!(events.len(), 0, "Event should not exist after rollback");
}
