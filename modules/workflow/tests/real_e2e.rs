//! Integration tests for workflow module (bd-iyfxs, bd-hv1v4, bd-ucz7d).
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
//! 15. Parallel routing — 2-of-3 threshold with dedup
//! 16. Parallel routing — duplicate actor rejection
//! 17. Conditional routing — branch by amount (gte)
//! 18. Conditional routing — default branch fallback
//! 19. Conditional routing — no match without default fails
//! 30. Escalation: pending step times out → single escalation fires (exactly-once)
//! 31. Escalation: re-tick on already-fired timer is no-op (idempotent)
//! 32. Escalation: cancel timers when instance advances
//! 33. Delegation: create rule + resolve delegatee
//! 34. Delegation: revoke rule + verify no longer resolves
//! 35. Delegation: delegatee can decide on behalf of delegator

use serial_test::serial;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use workflow::domain::definitions::{
    CreateDefinitionRequest, DefError, DefinitionRepo, ListDefinitionsQuery,
};
use workflow::domain::holds::{
    ApplyHoldRequest, HoldError, HoldRepo, ListHoldsQuery, ReleaseHoldRequest,
};
use workflow::domain::instances::{
    AdvanceInstanceRequest, InstanceError, InstanceRepo, ListInstancesQuery,
    StartInstanceRequest,
};
use workflow::domain::routing::{
    EvaluateConditionRequest, RecordDecisionRequest, RoutingEngine, RoutingError,
};
use workflow::domain::escalation::{
    CreateEscalationRuleRequest, EscalationRepo,
};
use workflow::domain::delegation::{
    CreateDelegationRequest, DelegationError, DelegationRepo, ResolveDelegationQuery,
    RevokeDelegationRequest,
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

// ============================================================================
// 15. Parallel routing — 2-of-3 threshold auto-advances after 2nd approval
// ============================================================================

fn parallel_steps() -> serde_json::Value {
    json!([
        {
            "step_id": "draft",
            "name": "Draft",
            "allowed_transitions": ["review"]
        },
        {
            "step_id": "review",
            "name": "Parallel Review",
            "routing_mode": { "mode": "parallel", "threshold": 2 },
            "allowed_transitions": ["approved"]
        },
        {
            "step_id": "approved",
            "name": "Approved",
            "allowed_transitions": []
        }
    ])
}

async fn create_parallel_definition(
    pool: &sqlx::PgPool,
    tid: &str,
) -> workflow::domain::definitions::WorkflowDefinition {
    DefinitionRepo::create(
        pool,
        &CreateDefinitionRequest {
            tenant_id: tid.to_string(),
            name: format!("parallel-def-{}", Uuid::new_v4().simple()),
            description: Some("Parallel threshold test".into()),
            steps: parallel_steps(),
            initial_step_id: "draft".into(),
        },
    )
    .await
    .expect("create parallel def failed")
}

#[tokio::test]
#[serial]
async fn test_parallel_threshold_auto_advance() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let def = create_parallel_definition(&pool, &tid).await;

    // Start instance at draft
    let instance = InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tid.clone(),
            definition_id: def.id,
            entity_type: "purchase_order".into(),
            entity_id: "PO-PAR-001".into(),
            context: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("start failed");

    // Advance to the parallel review step
    let (at_review, _) = InstanceRepo::advance(
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
            idempotency_key: None,
        },
    )
    .await
    .expect("advance to review failed");

    assert_eq!(at_review.current_step_id, "review");

    let actor_1 = Uuid::new_v4();
    let actor_2 = Uuid::new_v4();
    let actor_3 = Uuid::new_v4();

    // First approval — threshold NOT met (1 of 2)
    let r1 = RoutingEngine::record_decision(
        &pool,
        &RecordDecisionRequest {
            tenant_id: tid.clone(),
            instance_id: instance.id,
            step_id: "review".into(),
            actor_id: actor_1,
            actor_type: Some("user".into()),
            decision: "approve".into(),
            comment: None,
            metadata: None,
        },
    )
    .await
    .expect("decision 1 failed");

    assert!(!r1.threshold_met);
    assert_eq!(r1.current_count, 1);
    assert_eq!(r1.threshold, 2);
    assert!(r1.advanced_instance.is_none());

    // Second approval — threshold MET (2 of 2), auto-advances to "approved"
    let r2 = RoutingEngine::record_decision(
        &pool,
        &RecordDecisionRequest {
            tenant_id: tid.clone(),
            instance_id: instance.id,
            step_id: "review".into(),
            actor_id: actor_2,
            actor_type: Some("user".into()),
            decision: "approve".into(),
            comment: Some("LGTM".into()),
            metadata: None,
        },
    )
    .await
    .expect("decision 2 failed");

    assert!(r2.threshold_met);
    assert_eq!(r2.current_count, 2);
    let advanced = r2.advanced_instance.expect("should have auto-advanced");
    assert_eq!(advanced.current_step_id, "approved");

    // Verify transition audit trail includes the auto-advance
    let transitions = InstanceRepo::list_transitions(&pool, &tid, instance.id)
        .await
        .expect("list transitions failed");
    let actions: Vec<&str> = transitions.iter().map(|t| t.action.as_str()).collect();
    assert!(actions.contains(&"parallel_threshold_met"));

    // Verify outbox has threshold_met event
    let events: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM events_outbox WHERE aggregate_id = $1 ORDER BY created_at",
    )
    .bind(instance.id.to_string())
    .fetch_all(&pool)
    .await
    .expect("query failed");
    let types: Vec<&str> = events.iter().map(|e| e.0.as_str()).collect();
    assert!(types.contains(&"workflow.events.step.parallel_threshold_met"));

    // Third actor tries to decide — step already advanced, should fail with StepMismatch
    let r3 = RoutingEngine::record_decision(
        &pool,
        &RecordDecisionRequest {
            tenant_id: tid.clone(),
            instance_id: instance.id,
            step_id: "review".into(),
            actor_id: actor_3,
            actor_type: Some("user".into()),
            decision: "approve".into(),
            comment: None,
            metadata: None,
        },
    )
    .await;
    assert!(
        matches!(r3, Err(RoutingError::StepMismatch { .. })),
        "Expected StepMismatch after auto-advance, got: {:?}",
        r3
    );

    // Verify decisions list
    let decisions = RoutingEngine::list_decisions(&pool, &tid, instance.id, "review")
        .await
        .expect("list decisions failed");
    assert_eq!(decisions.len(), 2);
}

// ============================================================================
// 16. Parallel routing — duplicate actor decision is rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_parallel_duplicate_actor_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let def = create_parallel_definition(&pool, &tid).await;

    let instance = InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tid.clone(),
            definition_id: def.id,
            entity_type: "po".into(),
            entity_id: "PO-DUP-001".into(),
            context: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("start failed");

    InstanceRepo::advance(
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
            idempotency_key: None,
        },
    )
    .await
    .expect("advance to review failed");

    let actor = Uuid::new_v4();

    // First decision succeeds
    RoutingEngine::record_decision(
        &pool,
        &RecordDecisionRequest {
            tenant_id: tid.clone(),
            instance_id: instance.id,
            step_id: "review".into(),
            actor_id: actor,
            actor_type: Some("user".into()),
            decision: "approve".into(),
            comment: None,
            metadata: None,
        },
    )
    .await
    .expect("first decision should succeed");

    // Same actor, same step → DuplicateDecision
    let dup = RoutingEngine::record_decision(
        &pool,
        &RecordDecisionRequest {
            tenant_id: tid.clone(),
            instance_id: instance.id,
            step_id: "review".into(),
            actor_id: actor,
            actor_type: Some("user".into()),
            decision: "approve".into(),
            comment: None,
            metadata: None,
        },
    )
    .await;

    assert!(
        matches!(dup, Err(RoutingError::DuplicateDecision(_))),
        "Expected DuplicateDecision, got: {:?}",
        dup
    );

    // Verify only 1 decision was counted (not 2)
    let decisions = RoutingEngine::list_decisions(&pool, &tid, instance.id, "review")
        .await
        .expect("list decisions failed");
    assert_eq!(decisions.len(), 1, "duplicate must not be recorded");
}

// ============================================================================
// 17. Conditional routing — branch by amount (gte → director, default → manager)
// ============================================================================

fn conditional_steps() -> serde_json::Value {
    json!([
        {
            "step_id": "triage",
            "name": "Triage",
            "routing_mode": {
                "mode": "conditional",
                "conditions": [
                    {
                        "field": "amount",
                        "op": "gte",
                        "value": 10000,
                        "target_step": "director_review"
                    },
                    {
                        "default": true,
                        "target_step": "manager_review"
                    }
                ]
            },
            "allowed_transitions": ["director_review", "manager_review"]
        },
        {
            "step_id": "director_review",
            "name": "Director Review",
            "allowed_transitions": ["approved"]
        },
        {
            "step_id": "manager_review",
            "name": "Manager Review",
            "allowed_transitions": ["approved"]
        },
        {
            "step_id": "approved",
            "name": "Approved",
            "allowed_transitions": []
        }
    ])
}

async fn create_conditional_definition(
    pool: &sqlx::PgPool,
    tid: &str,
) -> workflow::domain::definitions::WorkflowDefinition {
    DefinitionRepo::create(
        pool,
        &CreateDefinitionRequest {
            tenant_id: tid.to_string(),
            name: format!("cond-def-{}", Uuid::new_v4().simple()),
            description: Some("Conditional routing test".into()),
            steps: conditional_steps(),
            initial_step_id: "triage".into(),
        },
    )
    .await
    .expect("create conditional def failed")
}

#[tokio::test]
#[serial]
async fn test_conditional_branch_high_amount() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let def = create_conditional_definition(&pool, &tid).await;

    // Start with amount = 25000 → should branch to director_review
    let instance = InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tid.clone(),
            definition_id: def.id,
            entity_type: "purchase_order".into(),
            entity_id: "PO-COND-HIGH".into(),
            context: Some(json!({ "amount": 25000, "department": "engineering" })),
            idempotency_key: None,
        },
    )
    .await
    .expect("start failed");

    assert_eq!(instance.current_step_id, "triage");

    let result = RoutingEngine::evaluate_condition(
        &pool,
        &EvaluateConditionRequest {
            tenant_id: tid.clone(),
            instance_id: instance.id,
        },
    )
    .await
    .expect("evaluate_condition failed");

    assert_eq!(result.target_step, "director_review");
    assert_eq!(result.matched_condition_index, Some(0));
    let advanced = result.advanced_instance.expect("should have advanced");
    assert_eq!(advanced.current_step_id, "director_review");

    // Verify the transition recorded the conditional branch
    let transitions = InstanceRepo::list_transitions(&pool, &tid, instance.id)
        .await
        .expect("list transitions failed");
    let last = transitions.last().expect("should have transitions");
    assert_eq!(last.action, "conditional_branch");
    assert_eq!(last.from_step_id, "triage");
    assert_eq!(last.to_step_id, "director_review");
}

// ============================================================================
// 18. Conditional routing — default branch fallback
// ============================================================================

#[tokio::test]
#[serial]
async fn test_conditional_branch_default_fallback() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let def = create_conditional_definition(&pool, &tid).await;

    // Start with amount = 500 → should fall through to manager_review (default)
    let instance = InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tid.clone(),
            definition_id: def.id,
            entity_type: "purchase_order".into(),
            entity_id: "PO-COND-LOW".into(),
            context: Some(json!({ "amount": 500 })),
            idempotency_key: None,
        },
    )
    .await
    .expect("start failed");

    let result = RoutingEngine::evaluate_condition(
        &pool,
        &EvaluateConditionRequest {
            tenant_id: tid.clone(),
            instance_id: instance.id,
        },
    )
    .await
    .expect("evaluate_condition failed");

    assert_eq!(result.target_step, "manager_review");
    assert_eq!(result.matched_condition_index, None); // default branch, no index
    let advanced = result.advanced_instance.expect("should have advanced");
    assert_eq!(advanced.current_step_id, "manager_review");
}

// ============================================================================
// 19. Conditional routing — no match without default fails
// ============================================================================

fn no_default_conditional_steps() -> serde_json::Value {
    json!([
        {
            "step_id": "check",
            "name": "Check",
            "routing_mode": {
                "mode": "conditional",
                "conditions": [
                    { "field": "priority", "op": "eq", "value": "critical", "target_step": "escalate" }
                ]
            },
            "allowed_transitions": ["escalate"]
        },
        {
            "step_id": "escalate",
            "name": "Escalate",
            "allowed_transitions": []
        }
    ])
}

#[tokio::test]
#[serial]
async fn test_conditional_no_match_no_default_fails() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let def = DefinitionRepo::create(
        &pool,
        &CreateDefinitionRequest {
            tenant_id: tid.clone(),
            name: format!("no-default-{}", Uuid::new_v4().simple()),
            description: None,
            steps: no_default_conditional_steps(),
            initial_step_id: "check".into(),
        },
    )
    .await
    .expect("create def failed");

    let instance = InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tid.clone(),
            definition_id: def.id,
            entity_type: "task".into(),
            entity_id: "TASK-NODEF".into(),
            context: Some(json!({ "priority": "low" })), // doesn't match "critical"
            idempotency_key: None,
        },
    )
    .await
    .expect("start failed");

    let err = RoutingEngine::evaluate_condition(
        &pool,
        &EvaluateConditionRequest {
            tenant_id: tid,
            instance_id: instance.id,
        },
    )
    .await;

    assert!(
        matches!(err, Err(RoutingError::NoConditionMatched)),
        "Expected NoConditionMatched, got: {:?}",
        err
    );
}

// ============================================================================
// 20. Apply hold — persists and enqueues outbox event
// ============================================================================

#[tokio::test]
#[serial]
async fn test_apply_hold_persists_and_emits_event() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let hold = HoldRepo::apply(
        &pool,
        &ApplyHoldRequest {
            tenant_id: tid.clone(),
            entity_type: "document".into(),
            entity_id: "DOC-001".into(),
            hold_type: "quality_hold".into(),
            reason: Some("Pending inspection".into()),
            applied_by: Some(Uuid::new_v4()),
            metadata: Some(json!({ "inspector": "QA-team" })),
            idempotency_key: None,
        },
    )
    .await
    .expect("apply hold failed");

    assert_eq!(hold.tenant_id, tid);
    assert_eq!(hold.entity_type, "document");
    assert_eq!(hold.entity_id, "DOC-001");
    assert_eq!(hold.hold_type, "quality_hold");
    assert_eq!(hold.reason.as_deref(), Some("Pending inspection"));
    assert!(hold.released_at.is_none());

    // Verify outbox event
    let event: Option<(String,)> =
        sqlx::query_as("SELECT event_type FROM events_outbox WHERE aggregate_id = $1")
            .bind(hold.id.to_string())
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert_eq!(event.unwrap().0, "workflow.events.hold.applied");
}

// ============================================================================
// 21. Release hold — sets released_at and enqueues outbox event
// ============================================================================

#[tokio::test]
#[serial]
async fn test_release_hold_sets_timestamp_and_emits_event() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let releaser = Uuid::new_v4();

    let hold = HoldRepo::apply(
        &pool,
        &ApplyHoldRequest {
            tenant_id: tid.clone(),
            entity_type: "part".into(),
            entity_id: "PART-42".into(),
            hold_type: "engineering_hold".into(),
            reason: Some("Design review pending".into()),
            applied_by: None,
            metadata: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("apply failed");

    let released = HoldRepo::release(
        &pool,
        hold.id,
        &ReleaseHoldRequest {
            tenant_id: tid.clone(),
            released_by: Some(releaser),
            release_reason: Some("Design approved".into()),
            idempotency_key: None,
        },
    )
    .await
    .expect("release failed");

    assert!(released.released_at.is_some());
    assert_eq!(released.released_by, Some(releaser));
    assert_eq!(released.release_reason.as_deref(), Some("Design approved"));

    // Verify both events in outbox
    let events: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM events_outbox WHERE aggregate_id = $1 ORDER BY created_at",
    )
    .bind(hold.id.to_string())
    .fetch_all(&pool)
    .await
    .unwrap();
    let types: Vec<&str> = events.iter().map(|e| e.0.as_str()).collect();
    assert!(types.contains(&"workflow.events.hold.applied"));
    assert!(types.contains(&"workflow.events.hold.released"));
}

// ============================================================================
// 22. Duplicate active hold — AlreadyHeld error
// ============================================================================

#[tokio::test]
#[serial]
async fn test_duplicate_active_hold_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    HoldRepo::apply(
        &pool,
        &ApplyHoldRequest {
            tenant_id: tid.clone(),
            entity_type: "batch".into(),
            entity_id: "BATCH-100".into(),
            hold_type: "material_hold".into(),
            reason: None,
            applied_by: None,
            metadata: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("first apply failed");

    // Same entity + hold_type → AlreadyHeld
    let err = HoldRepo::apply(
        &pool,
        &ApplyHoldRequest {
            tenant_id: tid.clone(),
            entity_type: "batch".into(),
            entity_id: "BATCH-100".into(),
            hold_type: "material_hold".into(),
            reason: None,
            applied_by: None,
            metadata: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, HoldError::AlreadyHeld),
        "Expected AlreadyHeld, got: {:?}",
        err
    );
}

// ============================================================================
// 23. Re-apply after release — allowed
// ============================================================================

#[tokio::test]
#[serial]
async fn test_reapply_after_release_allowed() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let hold1 = HoldRepo::apply(
        &pool,
        &ApplyHoldRequest {
            tenant_id: tid.clone(),
            entity_type: "order".into(),
            entity_id: "ORD-55".into(),
            hold_type: "customer_hold".into(),
            reason: Some("Credit check".into()),
            applied_by: None,
            metadata: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("apply failed");

    HoldRepo::release(
        &pool,
        hold1.id,
        &ReleaseHoldRequest {
            tenant_id: tid.clone(),
            released_by: None,
            release_reason: Some("Credit approved".into()),
            idempotency_key: None,
        },
    )
    .await
    .expect("release failed");

    // Re-apply same hold_type on same entity → should succeed
    let hold2 = HoldRepo::apply(
        &pool,
        &ApplyHoldRequest {
            tenant_id: tid.clone(),
            entity_type: "order".into(),
            entity_id: "ORD-55".into(),
            hold_type: "customer_hold".into(),
            reason: Some("Second credit check".into()),
            applied_by: None,
            metadata: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("re-apply failed");

    assert_ne!(hold1.id, hold2.id);
    assert!(hold2.released_at.is_none());
}

// ============================================================================
// 24. Release already-released hold — AlreadyReleased error
// ============================================================================

#[tokio::test]
#[serial]
async fn test_release_already_released_hold_fails() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let hold = HoldRepo::apply(
        &pool,
        &ApplyHoldRequest {
            tenant_id: tid.clone(),
            entity_type: "item".into(),
            entity_id: "ITEM-99".into(),
            hold_type: "quality_hold".into(),
            reason: None,
            applied_by: None,
            metadata: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("apply failed");

    HoldRepo::release(
        &pool,
        hold.id,
        &ReleaseHoldRequest {
            tenant_id: tid.clone(),
            released_by: None,
            release_reason: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("first release failed");

    let err = HoldRepo::release(
        &pool,
        hold.id,
        &ReleaseHoldRequest {
            tenant_id: tid.clone(),
            released_by: None,
            release_reason: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, HoldError::AlreadyReleased),
        "Expected AlreadyReleased, got: {:?}",
        err
    );
}

// ============================================================================
// 25. Release non-existent hold — NotFound
// ============================================================================

#[tokio::test]
#[serial]
async fn test_release_nonexistent_hold_fails() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let err = HoldRepo::release(
        &pool,
        Uuid::new_v4(),
        &ReleaseHoldRequest {
            tenant_id: tid,
            released_by: None,
            release_reason: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, HoldError::NotFound),
        "Expected NotFound, got: {:?}",
        err
    );
}

// ============================================================================
// 26. Idempotent apply — same key returns existing hold
// ============================================================================

#[tokio::test]
#[serial]
async fn test_idempotent_apply_hold() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let idem_key = format!("hold:apply:{}:DOC-IDEM", tid);

    let hold1 = HoldRepo::apply(
        &pool,
        &ApplyHoldRequest {
            tenant_id: tid.clone(),
            entity_type: "document".into(),
            entity_id: "DOC-IDEM".into(),
            hold_type: "quality_hold".into(),
            reason: Some("First apply".into()),
            applied_by: None,
            metadata: None,
            idempotency_key: Some(idem_key.clone()),
        },
    )
    .await
    .expect("first apply failed");

    // Same key → same hold returned
    let hold2 = HoldRepo::apply(
        &pool,
        &ApplyHoldRequest {
            tenant_id: tid.clone(),
            entity_type: "document".into(),
            entity_id: "DOC-IDEM".into(),
            hold_type: "quality_hold".into(),
            reason: Some("Second apply attempt".into()),
            applied_by: None,
            metadata: None,
            idempotency_key: Some(idem_key),
        },
    )
    .await
    .expect("idempotent apply failed");

    assert_eq!(hold1.id, hold2.id);
}

// ============================================================================
// 27. Idempotent release — same key returns existing release
// ============================================================================

#[tokio::test]
#[serial]
async fn test_idempotent_release_hold() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let idem_key = format!("hold:release:{}:ITEM-IDEM", tid);

    let hold = HoldRepo::apply(
        &pool,
        &ApplyHoldRequest {
            tenant_id: tid.clone(),
            entity_type: "item".into(),
            entity_id: "ITEM-IDEM".into(),
            hold_type: "engineering_hold".into(),
            reason: None,
            applied_by: None,
            metadata: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("apply failed");

    let released1 = HoldRepo::release(
        &pool,
        hold.id,
        &ReleaseHoldRequest {
            tenant_id: tid.clone(),
            released_by: None,
            release_reason: Some("Done".into()),
            idempotency_key: Some(idem_key.clone()),
        },
    )
    .await
    .expect("first release failed");

    // Same key → same result
    let released2 = HoldRepo::release(
        &pool,
        hold.id,
        &ReleaseHoldRequest {
            tenant_id: tid.clone(),
            released_by: None,
            release_reason: Some("Done".into()),
            idempotency_key: Some(idem_key),
        },
    )
    .await
    .expect("idempotent release failed");

    assert_eq!(released1.id, released2.id);
    assert!(released2.released_at.is_some());
}

// ============================================================================
// 28. List holds — active-only filter and entity filter
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_holds_with_filters() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let h1 = HoldRepo::apply(
        &pool,
        &ApplyHoldRequest {
            tenant_id: tid.clone(),
            entity_type: "doc".into(),
            entity_id: "D-LIST-1".into(),
            hold_type: "quality_hold".into(),
            reason: None,
            applied_by: None,
            metadata: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("apply h1 failed");

    HoldRepo::apply(
        &pool,
        &ApplyHoldRequest {
            tenant_id: tid.clone(),
            entity_type: "doc".into(),
            entity_id: "D-LIST-1".into(),
            hold_type: "engineering_hold".into(),
            reason: None,
            applied_by: None,
            metadata: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("apply h2 failed");

    // Release one
    HoldRepo::release(
        &pool,
        h1.id,
        &ReleaseHoldRequest {
            tenant_id: tid.clone(),
            released_by: None,
            release_reason: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("release failed");

    // List all for entity — should return 2
    let all = HoldRepo::list(
        &pool,
        &ListHoldsQuery {
            tenant_id: tid.clone(),
            entity_type: Some("doc".into()),
            entity_id: Some("D-LIST-1".into()),
            hold_type: None,
            active_only: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .expect("list all failed");
    assert_eq!(all.len(), 2);

    // List active only — should return 1
    let active = HoldRepo::list(
        &pool,
        &ListHoldsQuery {
            tenant_id: tid.clone(),
            entity_type: Some("doc".into()),
            entity_id: Some("D-LIST-1".into()),
            hold_type: None,
            active_only: Some(true),
            limit: None,
            offset: None,
        },
    )
    .await
    .expect("list active failed");
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].hold_type, "engineering_hold");

    // List by hold_type filter
    let quality = HoldRepo::list(
        &pool,
        &ListHoldsQuery {
            tenant_id: tid.clone(),
            entity_type: None,
            entity_id: None,
            hold_type: Some("quality_hold".into()),
            active_only: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .expect("list by type failed");
    assert_eq!(quality.len(), 1);
}

// ============================================================================
// 29. Tenant isolation — tenant B cannot see/release tenant A's holds
// ============================================================================

#[tokio::test]
#[serial]
async fn test_hold_tenant_isolation() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();

    let hold_a = HoldRepo::apply(
        &pool,
        &ApplyHoldRequest {
            tenant_id: tid_a.clone(),
            entity_type: "widget".into(),
            entity_id: "W-001".into(),
            hold_type: "quality_hold".into(),
            reason: None,
            applied_by: None,
            metadata: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("apply failed");

    // Tenant B cannot see tenant A's hold
    let result = HoldRepo::get(&pool, &tid_b, hold_a.id).await;
    assert!(matches!(result, Err(HoldError::NotFound)));

    // Tenant B cannot release tenant A's hold
    let err = HoldRepo::release(
        &pool,
        hold_a.id,
        &ReleaseHoldRequest {
            tenant_id: tid_b.clone(),
            released_by: None,
            release_reason: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, HoldError::NotFound));

    // Tenant B's list is empty
    let list = HoldRepo::list(
        &pool,
        &ListHoldsQuery {
            tenant_id: tid_b,
            entity_type: None,
            entity_id: None,
            hold_type: None,
            active_only: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .expect("list failed");
    assert!(list.is_empty());
}

// ============================================================================
// 30. Multiple hold types on same entity — independent
// ============================================================================

#[tokio::test]
#[serial]
async fn test_multiple_hold_types_independent() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let h_quality = HoldRepo::apply(
        &pool,
        &ApplyHoldRequest {
            tenant_id: tid.clone(),
            entity_type: "assembly".into(),
            entity_id: "ASM-001".into(),
            hold_type: "quality_hold".into(),
            reason: Some("Quality inspection".into()),
            applied_by: None,
            metadata: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("quality hold failed");

    let h_eng = HoldRepo::apply(
        &pool,
        &ApplyHoldRequest {
            tenant_id: tid.clone(),
            entity_type: "assembly".into(),
            entity_id: "ASM-001".into(),
            hold_type: "engineering_hold".into(),
            reason: Some("Design change pending".into()),
            applied_by: None,
            metadata: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("engineering hold failed");

    // Both active
    let active = HoldRepo::list(
        &pool,
        &ListHoldsQuery {
            tenant_id: tid.clone(),
            entity_type: Some("assembly".into()),
            entity_id: Some("ASM-001".into()),
            hold_type: None,
            active_only: Some(true),
            limit: None,
            offset: None,
        },
    )
    .await
    .expect("list failed");
    assert_eq!(active.len(), 2);

    // Release quality — engineering still active
    HoldRepo::release(
        &pool,
        h_quality.id,
        &ReleaseHoldRequest {
            tenant_id: tid.clone(),
            released_by: None,
            release_reason: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("release quality failed");

    let still_active = HoldRepo::list(
        &pool,
        &ListHoldsQuery {
            tenant_id: tid.clone(),
            entity_type: Some("assembly".into()),
            entity_id: Some("ASM-001".into()),
            hold_type: None,
            active_only: Some(true),
            limit: None,
            offset: None,
        },
    )
    .await
    .expect("list failed");
    assert_eq!(still_active.len(), 1);
    assert_eq!(still_active[0].id, h_eng.id);
}

// ============================================================================
// 30. Escalation: pending step times out → single escalation fires (exactly-once)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_escalation_timer_fires_once_when_due() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    // Create definition with a review step
    let def = create_test_definition(&pool, &tid).await;

    // Create escalation rule: review step times out after 5s
    let rule = EscalationRepo::create_rule(
        &pool,
        &CreateEscalationRuleRequest {
            tenant_id: tid.clone(),
            definition_id: def.id,
            step_id: "review".into(),
            timeout_seconds: 5,
            escalate_to_step: Some("approved".into()),
            notify_actor_ids: vec![Uuid::new_v4()],
            notify_template: Some("escalation_alert".into()),
            max_escalations: Some(1),
            metadata: None,
        },
    )
    .await
    .expect("create rule failed");

    assert_eq!(rule.timeout_seconds, 5);
    assert!(rule.is_active);

    // Start instance and advance to review step
    let instance = InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tid.clone(),
            definition_id: def.id,
            entity_type: "document".into(),
            entity_id: format!("DOC-ESC-{}", Uuid::new_v4().simple()),
            context: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("start failed");

    let (instance, _) = InstanceRepo::advance(
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
            idempotency_key: None,
        },
    )
    .await
    .expect("advance to review failed");

    assert_eq!(instance.current_step_id, "review");

    // Arm a timer that is already due (due_at in the past for testing)
    let past_due = chrono::Utc::now() - chrono::Duration::seconds(10);
    let timer = EscalationRepo::arm_timer_with_due_at(
        &pool,
        &tid,
        instance.id,
        &rule,
        past_due,
    )
    .await
    .expect("arm timer failed");

    assert!(timer.fired_at.is_none());
    assert!(timer.cancelled_at.is_none());

    // Tick (tenant-scoped) — should fire exactly one escalation
    let fired = EscalationRepo::tick_for_tenant(&pool, &tid, 10).await.expect("tick failed");
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].id, timer.id);
    assert!(fired[0].fired_at.is_some());
    assert_eq!(fired[0].escalation_count, 1);

    // Verify outbox event was enqueued
    let event: Option<(String,)> = sqlx::query_as(
        "SELECT event_type FROM events_outbox WHERE aggregate_id = $1",
    )
    .bind(timer.id.to_string())
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(
        event.unwrap().0,
        "workflow.events.escalation.fired"
    );

    // Tick again — should fire nothing (already fired)
    let fired2 = EscalationRepo::tick_for_tenant(&pool, &tid, 10).await.expect("tick2 failed");
    assert_eq!(fired2.len(), 0);
}

// ============================================================================
// 31. Escalation: arm_timer is idempotent — returns existing active timer
// ============================================================================

#[tokio::test]
#[serial]
async fn test_escalation_arm_timer_idempotent() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let def = create_test_definition(&pool, &tid).await;

    let rule = EscalationRepo::create_rule(
        &pool,
        &CreateEscalationRuleRequest {
            tenant_id: tid.clone(),
            definition_id: def.id,
            step_id: "review".into(),
            timeout_seconds: 60,
            escalate_to_step: None,
            notify_actor_ids: vec![],
            notify_template: None,
            max_escalations: None,
            metadata: None,
        },
    )
    .await
    .expect("create rule failed");

    let instance = InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tid.clone(),
            definition_id: def.id,
            entity_type: "document".into(),
            entity_id: format!("DOC-IDEM-{}", Uuid::new_v4().simple()),
            context: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("start failed");

    // Arm twice — should return same timer
    let t1 = EscalationRepo::arm_timer(&pool, &tid, instance.id, &rule)
        .await
        .expect("arm 1 failed");
    let t2 = EscalationRepo::arm_timer(&pool, &tid, instance.id, &rule)
        .await
        .expect("arm 2 failed");

    assert_eq!(t1.id, t2.id);
}

// ============================================================================
// 32. Escalation: cancel timers when instance advances past step
// ============================================================================

#[tokio::test]
#[serial]
async fn test_escalation_cancel_timers_on_advance() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let def = create_test_definition(&pool, &tid).await;

    let rule = EscalationRepo::create_rule(
        &pool,
        &CreateEscalationRuleRequest {
            tenant_id: tid.clone(),
            definition_id: def.id,
            step_id: "review".into(),
            timeout_seconds: 300,
            escalate_to_step: Some("approved".into()),
            notify_actor_ids: vec![],
            notify_template: None,
            max_escalations: None,
            metadata: None,
        },
    )
    .await
    .expect("create rule failed");

    let instance = InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tid.clone(),
            definition_id: def.id,
            entity_type: "document".into(),
            entity_id: format!("DOC-CANCEL-{}", Uuid::new_v4().simple()),
            context: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("start failed");

    let (instance, _) = InstanceRepo::advance(
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
            idempotency_key: None,
        },
    )
    .await
    .expect("advance failed");

    // Arm timer
    let timer = EscalationRepo::arm_timer(&pool, &tid, instance.id, &rule)
        .await
        .expect("arm failed");
    assert!(timer.cancelled_at.is_none());

    // Verify active timers
    let active = EscalationRepo::list_active_timers(&pool, &tid, instance.id)
        .await
        .expect("list failed");
    assert_eq!(active.len(), 1);

    // Instance advances past review — cancel timers
    let cancelled = EscalationRepo::cancel_timers_for_instance(&pool, &tid, instance.id)
        .await
        .expect("cancel failed");
    assert_eq!(cancelled.len(), 1);
    assert!(cancelled[0].cancelled_at.is_some());

    // No more active timers
    let remaining = EscalationRepo::list_active_timers(&pool, &tid, instance.id)
        .await
        .expect("list failed");
    assert_eq!(remaining.len(), 0);

    // Tick (tenant-scoped) should not fire anything (cancelled)
    let fired = EscalationRepo::tick_for_tenant(&pool, &tid, 10).await.expect("tick failed");
    assert_eq!(fired.len(), 0);
}

// ============================================================================
// 33. Delegation: create rule + resolve delegatee
// ============================================================================

#[tokio::test]
#[serial]
async fn test_delegation_create_and_resolve() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let def = create_test_definition(&pool, &tid).await;

    let delegator = Uuid::new_v4();
    let delegatee = Uuid::new_v4();

    // Create delegation: delegator → delegatee, scoped to this definition
    let delegation = DelegationRepo::create(
        &pool,
        &CreateDelegationRequest {
            tenant_id: tid.clone(),
            delegator_id: delegator,
            delegatee_id: delegatee,
            definition_id: Some(def.id),
            entity_type: Some("document".into()),
            reason: Some("Out of office".into()),
            valid_from: None,
            valid_until: None,
        },
    )
    .await
    .expect("create delegation failed");

    assert_eq!(delegation.delegator_id, delegator);
    assert_eq!(delegation.delegatee_id, delegatee);
    assert!(delegation.revoked_at.is_none());

    // Verify outbox event
    let event: Option<(String,)> = sqlx::query_as(
        "SELECT event_type FROM events_outbox WHERE aggregate_id = $1",
    )
    .bind(delegation.id.to_string())
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(event.unwrap().0, "workflow.events.delegation.created");

    // Resolve: delegator should resolve to delegatee
    let resolved = DelegationRepo::resolve_delegatee(
        &pool,
        &ResolveDelegationQuery {
            tenant_id: tid.clone(),
            actor_id: delegator,
            definition_id: Some(def.id),
            entity_type: Some("document".into()),
        },
    )
    .await
    .expect("resolve failed");
    assert!(resolved.is_some());
    assert_eq!(resolved.unwrap().delegatee_id, delegatee);

    // Non-delegated actor resolves to None
    let other_actor = Uuid::new_v4();
    let not_delegated = DelegationRepo::resolve_delegatee(
        &pool,
        &ResolveDelegationQuery {
            tenant_id: tid.clone(),
            actor_id: other_actor,
            definition_id: Some(def.id),
            entity_type: Some("document".into()),
        },
    )
    .await
    .expect("resolve failed");
    assert!(not_delegated.is_none());
}

// ============================================================================
// 34. Delegation: revoke rule → no longer resolves
// ============================================================================

#[tokio::test]
#[serial]
async fn test_delegation_revoke_stops_resolution() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let delegator = Uuid::new_v4();
    let delegatee = Uuid::new_v4();
    let revoker = Uuid::new_v4();

    let delegation = DelegationRepo::create(
        &pool,
        &CreateDelegationRequest {
            tenant_id: tid.clone(),
            delegator_id: delegator,
            delegatee_id: delegatee,
            definition_id: None,
            entity_type: None,
            reason: Some("Vacation".into()),
            valid_from: None,
            valid_until: None,
        },
    )
    .await
    .expect("create failed");

    // Revoke it
    let revoked = DelegationRepo::revoke(
        &pool,
        delegation.id,
        &RevokeDelegationRequest {
            tenant_id: tid.clone(),
            revoked_by: revoker,
            revoke_reason: Some("Returned from vacation".into()),
        },
    )
    .await
    .expect("revoke failed");
    assert!(revoked.revoked_at.is_some());
    assert_eq!(revoked.revoked_by, Some(revoker));

    // Verify outbox event
    let events: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM events_outbox WHERE aggregate_id = $1 ORDER BY created_at",
    )
    .bind(delegation.id.to_string())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].0, "workflow.events.delegation.created");
    assert_eq!(events[1].0, "workflow.events.delegation.revoked");

    // Resolve: should no longer return delegatee
    let resolved = DelegationRepo::resolve_delegatee(
        &pool,
        &ResolveDelegationQuery {
            tenant_id: tid.clone(),
            actor_id: delegator,
            definition_id: None,
            entity_type: None,
        },
    )
    .await
    .expect("resolve failed");
    assert!(resolved.is_none());

    // Double-revoke should fail
    let err = DelegationRepo::revoke(
        &pool,
        delegation.id,
        &RevokeDelegationRequest {
            tenant_id: tid.clone(),
            revoked_by: revoker,
            revoke_reason: None,
        },
    )
    .await;
    assert!(matches!(err, Err(DelegationError::AlreadyRevoked)));
}

// ============================================================================
// 35. Delegation: delegatee can record decision on behalf of delegator
// ============================================================================

#[tokio::test]
#[serial]
async fn test_delegation_delegatee_decides_for_delegator() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    // Create definition with a parallel review step (2-of-2 threshold)
    let steps = json!([
        {
            "step_id": "draft",
            "name": "Draft",
            "allowed_transitions": ["review"]
        },
        {
            "step_id": "review",
            "name": "Review",
            "routing_mode": { "mode": "parallel", "threshold": 2 },
            "allowed_transitions": ["approved"]
        },
        {
            "step_id": "approved",
            "name": "Approved",
            "allowed_transitions": []
        }
    ]);

    let def = DefinitionRepo::create(
        &pool,
        &CreateDefinitionRequest {
            tenant_id: tid.clone(),
            name: format!("deleg-test-{}", Uuid::new_v4().simple()),
            description: None,
            steps,
            initial_step_id: "draft".into(),
        },
    )
    .await
    .expect("create def failed");

    let delegator = Uuid::new_v4();
    let delegatee = Uuid::new_v4();
    let other_reviewer = Uuid::new_v4();

    // Create delegation: delegator → delegatee
    DelegationRepo::create(
        &pool,
        &CreateDelegationRequest {
            tenant_id: tid.clone(),
            delegator_id: delegator,
            delegatee_id: delegatee,
            definition_id: Some(def.id),
            entity_type: None,
            reason: Some("OOO coverage".into()),
            valid_from: None,
            valid_until: None,
        },
    )
    .await
    .expect("create delegation failed");

    // Start instance and advance to review
    let instance = InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tid.clone(),
            definition_id: def.id,
            entity_type: "document".into(),
            entity_id: format!("DOC-DELEG-{}", Uuid::new_v4().simple()),
            context: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("start failed");

    let (instance, _) = InstanceRepo::advance(
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
            idempotency_key: None,
        },
    )
    .await
    .expect("advance to review failed");

    // Resolve delegation: the system should check if delegator has a delegate
    let resolved = DelegationRepo::resolve_delegatee(
        &pool,
        &ResolveDelegationQuery {
            tenant_id: tid.clone(),
            actor_id: delegator,
            definition_id: Some(def.id),
            entity_type: None,
        },
    )
    .await
    .expect("resolve failed");
    assert!(resolved.is_some());
    let effective_actor = resolved.unwrap().delegatee_id;
    assert_eq!(effective_actor, delegatee);

    // Delegatee records decision on behalf of delegator
    let result1 = RoutingEngine::record_decision(
        &pool,
        &RecordDecisionRequest {
            tenant_id: tid.clone(),
            instance_id: instance.id,
            step_id: "review".into(),
            actor_id: delegatee, // delegatee acts
            actor_type: Some("delegatee".into()),
            decision: "approve".into(),
            comment: Some(format!("On behalf of {}", delegator)),
            metadata: Some(json!({
                "delegation_from": delegator.to_string()
            })),
        },
    )
    .await
    .expect("delegatee decision failed");

    assert!(!result1.threshold_met); // 1 of 2

    // Other reviewer decides normally
    let result2 = RoutingEngine::record_decision(
        &pool,
        &RecordDecisionRequest {
            tenant_id: tid.clone(),
            instance_id: instance.id,
            step_id: "review".into(),
            actor_id: other_reviewer,
            actor_type: None,
            decision: "approve".into(),
            comment: None,
            metadata: None,
        },
    )
    .await
    .expect("other reviewer decision failed");

    assert!(result2.threshold_met); // 2 of 2
    assert!(result2.advanced_instance.is_some());
    assert_eq!(
        result2.advanced_instance.unwrap().current_step_id,
        "approved"
    );

    // Verify delegatee cannot double-decide (dedup still works)
    let dup_err = RoutingEngine::record_decision(
        &pool,
        &RecordDecisionRequest {
            tenant_id: tid.clone(),
            instance_id: instance.id,
            step_id: "review".into(),
            actor_id: delegatee,
            actor_type: None,
            decision: "approve".into(),
            comment: None,
            metadata: None,
        },
    )
    .await;
    // The instance already advanced past review, so this should fail
    assert!(dup_err.is_err());
}
