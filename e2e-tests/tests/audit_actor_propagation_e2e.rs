/// E2E test for actor identity propagation into audit log
///
/// Verifies that:
/// 1. Actor identity is present in audit entries for mutations
/// 2. Service actors use deterministic IDs
/// 3. User actors preserve user identity
/// 4. System actors use well-known identity

mod common;

use audit::{
    actor::Actor,
    schema::{MutationClass, WriteAuditRequest},
    writer::AuditWriter,
};
use event_bus::EventEnvelope;
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
async fn test_user_actor_in_audit() {
    let pool = get_audit_pool().await;
    common::run_audit_migrations(&pool).await;

    let writer = AuditWriter::new(pool.clone());

    // Simulate a user action (e.g., from HTTP request with authenticated user)
    let user_id = Uuid::new_v4();
    let actor = Actor::user(user_id);

    let request = WriteAuditRequest::new(
        actor.id,
        actor.actor_type_str(),
        "CreateCustomer".to_string(),
        MutationClass::Create,
        "Customer".to_string(),
        "cust_123".to_string(),
    );

    let audit_id = writer
        .write(request)
        .await
        .expect("Failed to write audit event");

    // Verify the actor identity is preserved
    let events = writer
        .get_by_entity("Customer", "cust_123")
        .await
        .expect("Failed to query audit events");

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].audit_id, audit_id);
    assert_eq!(events[0].actor_id, user_id);
    assert_eq!(events[0].actor_type, "User");
    assert_eq!(events[0].action, "CreateCustomer");
}

#[tokio::test]
async fn test_service_actor_deterministic_id() {
    let pool = get_audit_pool().await;
    common::run_audit_migrations(&pool).await;

    let writer = AuditWriter::new(pool.clone());

    // Simulate a scheduled job (e.g., billing scheduler)
    let actor1 = Actor::service("billing-scheduler");
    let actor2 = Actor::service("billing-scheduler");

    // Verify deterministic IDs
    assert_eq!(
        actor1.id, actor2.id,
        "Service actors should have deterministic IDs"
    );

    let request = WriteAuditRequest::new(
        actor1.id,
        actor1.actor_type_str(),
        "GenerateInvoices".to_string(),
        MutationClass::Create,
        "InvoiceBatch".to_string(),
        "batch_456".to_string(),
    );

    let audit_id = writer
        .write(request)
        .await
        .expect("Failed to write audit event");

    // Verify the service actor is recorded
    let events = writer
        .get_by_entity("InvoiceBatch", "batch_456")
        .await
        .expect("Failed to query audit events");

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].audit_id, audit_id);
    assert_eq!(events[0].actor_id, actor1.id);
    assert_eq!(events[0].actor_type, "Service");
}

#[tokio::test]
async fn test_system_actor() {
    let pool = get_audit_pool().await;
    common::run_audit_migrations(&pool).await;

    let writer = AuditWriter::new(pool.clone());

    // Simulate a system operation (e.g., migration, maintenance)
    let actor = Actor::system();

    let request = WriteAuditRequest::new(
        actor.id,
        actor.actor_type_str(),
        "MigrateData".to_string(),
        MutationClass::Update,
        "System".to_string(),
        "migration_v2".to_string(),
    );

    let audit_id = writer
        .write(request)
        .await
        .expect("Failed to write audit event");

    // Verify the system actor uses nil UUID
    let events = writer
        .get_by_entity("System", "migration_v2")
        .await
        .expect("Failed to query audit events");

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].audit_id, audit_id);
    assert_eq!(events[0].actor_id, Uuid::nil());
    assert_eq!(events[0].actor_type, "System");
}

#[tokio::test]
async fn test_actor_propagation_to_event_envelope() {
    // Simulate actor propagation from HTTP request to event envelope
    let user_id = Uuid::new_v4();
    let actor = Actor::user(user_id);

    // Create an event envelope with actor identity
    let envelope = EventEnvelope::new(
        "tenant-123".to_string(),
        "test-module".to_string(),
        "customer.created".to_string(),
        json!({"customer_id": "cust_789"}),
    )
    .with_actor(actor.id, actor.actor_type_str())
    .with_mutation_class(Some("DATA_MUTATION".to_string()));

    // Verify actor fields are populated
    assert_eq!(envelope.actor_id, Some(user_id));
    assert_eq!(envelope.actor_type, Some("User".to_string()));
    assert_eq!(envelope.mutation_class, Some("DATA_MUTATION".to_string()));
}

#[tokio::test]
async fn test_multiple_actor_types_in_correlation() {
    let pool = get_audit_pool().await;
    common::run_audit_migrations(&pool).await;

    let writer = AuditWriter::new(pool.clone());
    let correlation_id = Uuid::new_v4();

    // User creates a resource
    let user_actor = Actor::user(Uuid::new_v4());
    let request1 = WriteAuditRequest::new(
        user_actor.id,
        user_actor.actor_type_str(),
        "CreateOrder".to_string(),
        MutationClass::Create,
        "Order".to_string(),
        "order_123".to_string(),
    )
    .with_correlation(None, Some(correlation_id), None);

    writer
        .write(request1)
        .await
        .expect("Failed to write user audit event");

    // Service processes the order
    let service_actor = Actor::service("order-processor");
    let request2 = WriteAuditRequest::new(
        service_actor.id,
        service_actor.actor_type_str(),
        "ProcessOrder".to_string(),
        MutationClass::StateTransition,
        "Order".to_string(),
        "order_123".to_string(),
    )
    .with_correlation(None, Some(correlation_id), None);

    writer
        .write(request2)
        .await
        .expect("Failed to write service audit event");

    // Query by correlation_id
    let events = writer
        .get_by_correlation(correlation_id)
        .await
        .expect("Failed to query by correlation");

    assert_eq!(events.len(), 2);

    // Verify first event is from user
    assert_eq!(events[0].action, "CreateOrder");
    assert_eq!(events[0].actor_type, "User");
    assert_eq!(events[0].actor_id, user_actor.id);

    // Verify second event is from service
    assert_eq!(events[1].action, "ProcessOrder");
    assert_eq!(events[1].actor_type, "Service");
    assert_eq!(events[1].actor_id, service_actor.id);
}

#[tokio::test]
async fn test_actor_required_for_mutations() {
    let pool = get_audit_pool().await;
    common::run_audit_migrations(&pool).await;

    let writer = AuditWriter::new(pool.clone());

    // Every mutation must have an actor (cannot be nil unless System actor)
    let actor = Actor::system(); // Using System actor with nil UUID is valid

    let request = WriteAuditRequest::new(
        actor.id,
        actor.actor_type_str(),
        "SystemMutation".to_string(),
        MutationClass::Create,
        "Config".to_string(),
        "config_123".to_string(),
    );

    let result = writer.write(request).await;
    assert!(
        result.is_ok(),
        "System actor with nil UUID should be allowed"
    );
}
