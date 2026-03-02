//! Integration tests for workflow module (bd-iyfxs).
//!
//! Covers:
//! 1. Create a workflow definition — guard validation, DB persistence, outbox event
//! 2. Start an instance — definition lookup, initial transition, outbox event
//! 3. Advance through steps — Guard→Mutation→Outbox, transition audit trail
//! 4. Complete a workflow via __completed__ terminal pseudo-step
//! 5. Cancel a workflow via __cancelled__ terminal pseudo-step
//! 6. Reject advance on non-active instance
//! 7. Reject invalid step_id on advance
//! 8. Idempotent start — same key returns existing instance
//! 9. Idempotent advance — same key returns existing transition
//! 10. Tenant isolation — tenant B cannot see/advance tenant A's instances
//! 11. Duplicate definition name+version — returns Duplicate error

use serial_test::serial;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use workflow::domain::definitions::{
    CreateDefinitionRequest, DefError, DefinitionRepo, ListDefinitionsQuery,
};
use workflow::domain::instances::{
    AdvanceInstanceRequest, InstanceError, InstanceRepo, ListInstancesQuery,
    StartInstanceRequest,
};

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://workflow_user:workflow_pass@localhost:5457/workflow_db".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to workflow test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run workflow migrations");

    pool
}

fn unique_tenant() -> String {
    format!("wf-test-{}", Uuid::new_v4().simple())
}

fn sample_steps() -> serde_json::Value {
    json!([
        { "step_id": "draft", "name": "Draft", "allowed_transitions": ["review"] },
        { "step_id": "review", "name": "Review", "allowed_transitions": ["approved", "rejected"] },
        { "step_id": "approved", "name": "Approved", "allowed_transitions": [] },
        { "step_id": "rejected", "name": "Rejected", "allowed_transitions": ["draft"] }
    ])
}

async fn create_test_definition(pool: &sqlx::PgPool, tid: &str) -> workflow::domain::definitions::WorkflowDefinition {
    DefinitionRepo::create(
        pool,
        &CreateDefinitionRequest {
            tenant_id: tid.to_string(),
            name: format!("test-def-{}", Uuid::new_v4().simple()),
            description: Some("Test definition".into()),
            steps: sample_steps(),
            initial_step_id: "draft".into(),
        },
    )
    .await
    .unwrap()
}

// ============================================================================
// 1. Create definition — guard validation + persistence + outbox
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_definition_persists_and_enqueues_event() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let def = create_test_definition(&pool, &tid).await;

    assert_eq!(def.tenant_id, tid);
    assert_eq!(def.initial_step_id, "draft");
    assert!(def.is_active);
    assert_eq!(def.version, 1);

    // Verify outbox event was enqueued
    let event: Option<(String,)> =
        sqlx::query_as("SELECT event_type FROM events_outbox WHERE aggregate_id = $1")
            .bind(def.id.to_string())
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert_eq!(
        event.unwrap().0,
        "workflow.events.definition.created"
    );
}

// ============================================================================
// 2. Definition guard — rejects empty steps, missing initial_step_id
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_definition_rejects_empty_steps() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let result = DefinitionRepo::create(
        &pool,
        &CreateDefinitionRequest {
            tenant_id: tid,
            name: "empty-steps".into(),
            description: None,
            steps: json!([]),
            initial_step_id: "draft".into(),
        },
    )
    .await;

    assert!(matches!(result, Err(DefError::Validation(_))));
}

#[tokio::test]
#[serial]
async fn test_create_definition_rejects_missing_initial_step() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let result = DefinitionRepo::create(
        &pool,
        &CreateDefinitionRequest {
            tenant_id: tid,
            name: "bad-initial".into(),
            description: None,
            steps: json!([{ "step_id": "review" }]),
            initial_step_id: "nonexistent".into(),
        },
    )
    .await;

    assert!(matches!(result, Err(DefError::Validation(_))));
}

// ============================================================================
// 3. Start instance — definition lookup, initial transition, outbox
// ============================================================================

#[tokio::test]
#[serial]
async fn test_start_instance_creates_at_initial_step() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let def = create_test_definition(&pool, &tid).await;

    let instance = InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tid.clone(),
            definition_id: def.id,
            entity_type: "invoice".into(),
            entity_id: "INV-001".into(),
            context: Some(json!({ "amount": 100 })),
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(instance.tenant_id, tid);
    assert_eq!(instance.definition_id, def.id);
    assert_eq!(instance.current_step_id, "draft");
    assert_eq!(instance.status.to_string(), "active");
    assert_eq!(instance.entity_type, "invoice");
    assert_eq!(instance.entity_id, "INV-001");

    // Verify initial transition recorded
    let transitions = InstanceRepo::list_transitions(&pool, &tid, instance.id)
        .await
        .unwrap();
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].from_step_id, "__start__");
    assert_eq!(transitions[0].to_step_id, "draft");
    assert_eq!(transitions[0].action, "start");

    // Verify outbox event
    let event: Option<(String,)> =
        sqlx::query_as("SELECT event_type FROM events_outbox WHERE aggregate_id = $1")
            .bind(instance.id.to_string())
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert_eq!(
        event.unwrap().0,
        "workflow.events.instance.started"
    );
}

// ============================================================================
// 4. Advance through steps — transition audit trail
// ============================================================================

#[tokio::test]
#[serial]
async fn test_advance_through_steps_records_transitions() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let def = create_test_definition(&pool, &tid).await;

    let instance = InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tid.clone(),
            definition_id: def.id,
            entity_type: "po".into(),
            entity_id: "PO-001".into(),
            context: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    // Advance: draft → review
    let (inst2, tr1) = InstanceRepo::advance(
        &pool,
        instance.id,
        &AdvanceInstanceRequest {
            tenant_id: tid.clone(),
            to_step_id: "review".into(),
            action: "submit".into(),
            actor_id: None,
            actor_type: None,
            comment: Some("Submitting for review".into()),
            metadata: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(inst2.current_step_id, "review");
    assert_eq!(inst2.status.to_string(), "active");
    assert_eq!(tr1.from_step_id, "draft");
    assert_eq!(tr1.to_step_id, "review");
    assert_eq!(tr1.action, "submit");
    assert_eq!(tr1.comment.as_deref(), Some("Submitting for review"));

    // Advance: review → approved
    let (inst3, tr2) = InstanceRepo::advance(
        &pool,
        instance.id,
        &AdvanceInstanceRequest {
            tenant_id: tid.clone(),
            to_step_id: "approved".into(),
            action: "approve".into(),
            actor_id: Some(Uuid::new_v4()),
            actor_type: Some("user".into()),
            comment: None,
            metadata: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(inst3.current_step_id, "approved");
    assert_eq!(tr2.from_step_id, "review");
    assert_eq!(tr2.to_step_id, "approved");

    // Verify full transition audit trail
    let transitions = InstanceRepo::list_transitions(&pool, &tid, instance.id)
        .await
        .unwrap();
    assert_eq!(transitions.len(), 3); // __start__→draft, draft→review, review→approved
}

// ============================================================================
// 5. Complete workflow via __completed__ terminal pseudo-step
// ============================================================================

#[tokio::test]
#[serial]
async fn test_complete_workflow_via_terminal_step() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let def = create_test_definition(&pool, &tid).await;

    let instance = InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tid.clone(),
            definition_id: def.id,
            entity_type: "task".into(),
            entity_id: "TASK-001".into(),
            context: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    let (completed, _) = InstanceRepo::advance(
        &pool,
        instance.id,
        &AdvanceInstanceRequest {
            tenant_id: tid.clone(),
            to_step_id: "__completed__".into(),
            action: "complete".into(),
            actor_id: None,
            actor_type: None,
            comment: None,
            metadata: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(completed.status.to_string(), "completed");
    assert!(completed.completed_at.is_some());

    // Verify completion outbox event
    let events: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM events_outbox WHERE aggregate_id = $1 ORDER BY created_at",
    )
    .bind(instance.id.to_string())
    .fetch_all(&pool)
    .await
    .unwrap();
    let types: Vec<&str> = events.iter().map(|e| e.0.as_str()).collect();
    assert!(types.contains(&"workflow.events.instance.completed"));
}

// ============================================================================
// 6. Cancel workflow via __cancelled__ terminal pseudo-step
// ============================================================================

#[tokio::test]
#[serial]
async fn test_cancel_workflow_via_terminal_step() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let def = create_test_definition(&pool, &tid).await;

    let instance = InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tid.clone(),
            definition_id: def.id,
            entity_type: "task".into(),
            entity_id: "TASK-002".into(),
            context: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    let (cancelled, _) = InstanceRepo::advance(
        &pool,
        instance.id,
        &AdvanceInstanceRequest {
            tenant_id: tid.clone(),
            to_step_id: "__cancelled__".into(),
            action: "cancel".into(),
            actor_id: None,
            actor_type: None,
            comment: Some("No longer needed".into()),
            metadata: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(cancelled.status.to_string(), "cancelled");
    assert!(cancelled.cancelled_at.is_some());
}

// ============================================================================
// 7. Reject advance on non-active instance
// ============================================================================

#[tokio::test]
#[serial]
async fn test_reject_advance_on_completed_instance() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let def = create_test_definition(&pool, &tid).await;

    let instance = InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tid.clone(),
            definition_id: def.id,
            entity_type: "task".into(),
            entity_id: "TASK-003".into(),
            context: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    // Complete it
    InstanceRepo::advance(
        &pool,
        instance.id,
        &AdvanceInstanceRequest {
            tenant_id: tid.clone(),
            to_step_id: "__completed__".into(),
            action: "complete".into(),
            actor_id: None,
            actor_type: None,
            comment: None,
            metadata: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    // Try to advance again — should fail
    let err = InstanceRepo::advance(
        &pool,
        instance.id,
        &AdvanceInstanceRequest {
            tenant_id: tid,
            to_step_id: "review".into(),
            action: "reopen".into(),
            actor_id: None,
            actor_type: None,
            comment: None,
            metadata: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, InstanceError::NotActive(_)));
}

// ============================================================================
// 8. Reject invalid step_id on advance
// ============================================================================

#[tokio::test]
#[serial]
async fn test_reject_invalid_step_id() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let def = create_test_definition(&pool, &tid).await;

    let instance = InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tid.clone(),
            definition_id: def.id,
            entity_type: "task".into(),
            entity_id: "TASK-004".into(),
            context: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    let err = InstanceRepo::advance(
        &pool,
        instance.id,
        &AdvanceInstanceRequest {
            tenant_id: tid,
            to_step_id: "nonexistent_step".into(),
            action: "move".into(),
            actor_id: None,
            actor_type: None,
            comment: None,
            metadata: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, InstanceError::InvalidTransition(_)));
}

// ============================================================================
// 9. Idempotent start — same key returns existing instance
// ============================================================================

#[tokio::test]
#[serial]
async fn test_idempotent_start() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let def = create_test_definition(&pool, &tid).await;
    let idem_key = format!("workflow:start:{}:INV-IDEM", tid);

    let inst1 = InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tid.clone(),
            definition_id: def.id,
            entity_type: "invoice".into(),
            entity_id: "INV-IDEM".into(),
            context: None,
            idempotency_key: Some(idem_key.clone()),
        },
    )
    .await
    .unwrap();

    // Same key → same instance
    let inst2 = InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tid,
            definition_id: def.id,
            entity_type: "invoice".into(),
            entity_id: "INV-IDEM".into(),
            context: None,
            idempotency_key: Some(idem_key),
        },
    )
    .await
    .unwrap();

    assert_eq!(inst1.id, inst2.id);
}

// ============================================================================
// 10. Idempotent advance — same key returns existing transition
// ============================================================================

#[tokio::test]
#[serial]
async fn test_idempotent_advance() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let def = create_test_definition(&pool, &tid).await;

    let instance = InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tid.clone(),
            definition_id: def.id,
            entity_type: "invoice".into(),
            entity_id: "INV-IDEM2".into(),
            context: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    let idem_key = format!("workflow:advance:{}:review", instance.id);

    let (inst1, tr1) = InstanceRepo::advance(
        &pool,
        instance.id,
        &AdvanceInstanceRequest {
            tenant_id: tid.clone(),
            to_step_id: "review".into(),
            action: "submit".into(),
            actor_id: None,
            actor_type: None,
            comment: None,
            metadata: None,
            idempotency_key: Some(idem_key.clone()),
        },
    )
    .await
    .unwrap();

    // Same key → same result
    let (inst2, tr2) = InstanceRepo::advance(
        &pool,
        instance.id,
        &AdvanceInstanceRequest {
            tenant_id: tid,
            to_step_id: "review".into(),
            action: "submit".into(),
            actor_id: None,
            actor_type: None,
            comment: None,
            metadata: None,
            idempotency_key: Some(idem_key),
        },
    )
    .await
    .unwrap();

    assert_eq!(inst1.id, inst2.id);
    assert_eq!(tr1.id, tr2.id);
}

// ============================================================================
// 11. Tenant isolation — tenant B cannot see/advance tenant A's instances
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_isolation() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();

    let def_a = create_test_definition(&pool, &tid_a).await;

    let inst_a = InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tid_a.clone(),
            definition_id: def_a.id,
            entity_type: "invoice".into(),
            entity_id: "INV-ISO".into(),
            context: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    // Tenant B cannot see tenant A's instance
    let result = InstanceRepo::get(&pool, &tid_b, inst_a.id).await;
    assert!(matches!(result, Err(InstanceError::NotFound)));

    // Tenant B cannot advance tenant A's instance
    let err = InstanceRepo::advance(
        &pool,
        inst_a.id,
        &AdvanceInstanceRequest {
            tenant_id: tid_b.clone(),
            to_step_id: "review".into(),
            action: "submit".into(),
            actor_id: None,
            actor_type: None,
            comment: None,
            metadata: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, InstanceError::NotFound));

    // Tenant B cannot see tenant A's definitions
    let result = DefinitionRepo::get(&pool, &tid_b, def_a.id).await;
    assert!(matches!(result, Err(DefError::NotFound)));

    // Tenant B cannot list tenant A's instances
    let list = InstanceRepo::list(
        &pool,
        &ListInstancesQuery {
            tenant_id: tid_b,
            entity_type: None,
            entity_id: None,
            status: None,
            definition_id: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert!(list.is_empty());
}

// ============================================================================
// 12. Duplicate definition name+version — returns Duplicate error
// ============================================================================

#[tokio::test]
#[serial]
async fn test_duplicate_definition_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let name = format!("dup-def-{}", Uuid::new_v4().simple());

    DefinitionRepo::create(
        &pool,
        &CreateDefinitionRequest {
            tenant_id: tid.clone(),
            name: name.clone(),
            description: None,
            steps: sample_steps(),
            initial_step_id: "draft".into(),
        },
    )
    .await
    .unwrap();

    // Same name + same tenant → version defaults to 1 → duplicate
    let err = DefinitionRepo::create(
        &pool,
        &CreateDefinitionRequest {
            tenant_id: tid,
            name,
            description: None,
            steps: sample_steps(),
            initial_step_id: "draft".into(),
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, DefError::Duplicate));
}

// ============================================================================
// 13. List definitions — active_only filter
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_definitions_with_filter() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    create_test_definition(&pool, &tid).await;
    create_test_definition(&pool, &tid).await;

    let all = DefinitionRepo::list(
        &pool,
        &ListDefinitionsQuery {
            tenant_id: tid.clone(),
            active_only: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert!(all.len() >= 2);

    let active = DefinitionRepo::list(
        &pool,
        &ListDefinitionsQuery {
            tenant_id: tid,
            active_only: Some(true),
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert!(active.len() >= 2);
}

// ============================================================================
// 14. List instances with filters
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_instances_with_filters() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let def = create_test_definition(&pool, &tid).await;

    InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tid.clone(),
            definition_id: def.id,
            entity_type: "invoice".into(),
            entity_id: "INV-F1".into(),
            context: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tid.clone(),
            definition_id: def.id,
            entity_type: "po".into(),
            entity_id: "PO-F1".into(),
            context: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    // Filter by entity_type
    let invoices = InstanceRepo::list(
        &pool,
        &ListInstancesQuery {
            tenant_id: tid.clone(),
            entity_type: Some("invoice".into()),
            entity_id: None,
            status: None,
            definition_id: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0].entity_type, "invoice");

    // Filter by status
    let active = InstanceRepo::list(
        &pool,
        &ListInstancesQuery {
            tenant_id: tid,
            entity_type: None,
            entity_id: None,
            status: Some("active".into()),
            definition_id: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert!(active.len() >= 2);
}
