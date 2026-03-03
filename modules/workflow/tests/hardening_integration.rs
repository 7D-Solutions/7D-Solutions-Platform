//! Phase 58 Gate A: workflow safety, tenant, and auth hardening (bd-15gse)
//!
//! Five required integration test categories against real Postgres on port 5457:
//!
//! 1. **Migration safety** — apply all migrations forward, verify schema tables
//! 2. **Tenant boundary** — tenant_A data invisible to tenant_B (definitions + instances)
//! 3. **AuthZ denial** — mutation endpoints reject requests without valid JWT claims
//! 4. **Guard→Mutation→Outbox atomicity** — write + outbox row in same transaction
//! 5. **Concurrent tenant isolation** — parallel requests from different tenants

use axum::{
    body::Body,
    extract::DefaultBodyLimit,
    http::Request as HttpRequest,
    http::StatusCode,
    routing::{get, patch, post},
    Extension, Router,
};
use security::{
    middleware::{
        default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
    },
    RequirePermissionsLayer, permissions,
};
use serial_test::serial;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

use workflow::domain::definitions::{
    CreateDefinitionRequest, DefinitionRepo, ListDefinitionsQuery,
};
use workflow::domain::instances::{
    InstanceRepo, ListInstancesQuery, StartInstanceRequest,
};
use workflow::{http, metrics, AppState};

// ============================================================================
// Test DB helpers
// ============================================================================

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://workflow_user:workflow_pass@localhost:5457/workflow_db".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(10)
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
    format!("harden-{}", Uuid::new_v4().simple())
}

fn sample_steps() -> serde_json::Value {
    json!([
        { "step_id": "draft", "name": "Draft", "allowed_transitions": ["review"] },
        { "step_id": "review", "name": "Review", "allowed_transitions": ["approved", "rejected"] },
        { "step_id": "approved", "name": "Approved", "allowed_transitions": [] },
        { "step_id": "rejected", "name": "Rejected", "allowed_transitions": ["draft"] }
    ])
}

async fn create_test_definition(
    pool: &sqlx::PgPool,
    tid: &str,
) -> workflow::domain::definitions::WorkflowDefinition {
    DefinitionRepo::create(
        pool,
        &CreateDefinitionRequest {
            tenant_id: tid.to_string(),
            name: format!("harden-def-{}", Uuid::new_v4().simple()),
            description: Some("Hardening test definition".into()),
            steps: sample_steps(),
            initial_step_id: "draft".into(),
        },
    )
    .await
    .unwrap()
}

/// Build the workflow HTTP router without JWT verification.
/// Without a JwtVerifier, the `optional_claims_mw` inserts no claims,
/// so RequirePermissionsLayer will reject mutation requests with 401.
fn build_test_router(pool: sqlx::PgPool) -> Router {
    let wf_metrics = Arc::new(
        metrics::WorkflowMetrics::new().expect("metrics"),
    );
    let app_state = Arc::new(AppState {
        pool,
        metrics: wf_metrics,
    });

    // No JWT verifier — simulates unauthenticated caller
    let maybe_verifier: Option<Arc<security::JwtVerifier>> = None;

    Router::new()
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(http::health::health))
        .route("/api/ready", get(http::health::ready))
        .route("/api/version", get(http::health::version))
        .merge(
            Router::new()
                .route(
                    "/api/workflow/definitions",
                    post(http::definitions::create_definition)
                        .get(http::definitions::list_definitions),
                )
                .route(
                    "/api/workflow/definitions/{def_id}",
                    get(http::definitions::get_definition),
                )
                .route(
                    "/api/workflow/instances",
                    post(http::instances::start_instance)
                        .get(http::instances::list_instances),
                )
                .route(
                    "/api/workflow/instances/{instance_id}",
                    get(http::instances::get_instance),
                )
                .route(
                    "/api/workflow/instances/{instance_id}/advance",
                    patch(http::instances::advance_instance),
                )
                .route(
                    "/api/workflow/instances/{instance_id}/transitions",
                    get(http::instances::list_transitions),
                )
                .route_layer(RequirePermissionsLayer::new(&[
                    permissions::WORKFLOW_MUTATE,
                ])),
        )
        .with_state(app_state)
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(axum::middleware::from_fn(timeout_middleware))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(Extension(default_rate_limiter()))
        .layer(axum::middleware::from_fn_with_state(
            maybe_verifier,
            security::optional_claims_mw,
        ))
}

// ============================================================================
// 1. Migration safety — apply forward, verify all expected tables exist
// ============================================================================

#[tokio::test]
#[serial]
async fn test_migration_safety_all_tables_present() {
    let pool = setup_db().await;

    // All expected tables from the 5 migration files
    let expected_tables = vec![
        "events_outbox",
        "processed_events",
        "workflow_definitions",
        "workflow_instances",
        "workflow_transitions",
        "workflow_idempotency_keys",
        "workflow_step_decisions",
        "workflow_holds",
        "workflow_escalation_rules",
        "workflow_escalation_timers",
        "workflow_delegation_rules",
    ];

    for table in &expected_tables {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.tables
                WHERE table_schema = 'public' AND table_name = $1
            )",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert!(exists, "Expected table '{}' missing after migrations", table);
    }

    // Verify key columns exist
    let column_checks = vec![
        ("workflow_definitions", "tenant_id"),
        ("workflow_definitions", "steps"),
        ("workflow_definitions", "initial_step_id"),
        ("workflow_instances", "tenant_id"),
        ("workflow_instances", "definition_id"),
        ("workflow_instances", "current_step_id"),
        ("workflow_instances", "status"),
        ("workflow_transitions", "tenant_id"),
        ("workflow_transitions", "idempotency_key"),
        ("workflow_holds", "hold_type"),
        ("workflow_holds", "released_at"),
        ("workflow_escalation_rules", "timeout_seconds"),
        ("workflow_delegation_rules", "delegator_id"),
        ("workflow_delegation_rules", "delegatee_id"),
    ];

    for (table, column) in &column_checks {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.columns
                WHERE table_name = $1 AND column_name = $2
            )",
        )
        .bind(table)
        .bind(column)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert!(
            exists,
            "Expected column '{}.{}' missing after migrations",
            table, column
        );
    }

    // Verify key unique indexes exist (tenant isolation boundaries)
    let has_def_unique: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM pg_indexes
            WHERE tablename = 'workflow_definitions'
              AND indexdef LIKE '%tenant_id%name%version%'
        )",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        has_def_unique,
        "workflow_definitions must have unique constraint on (tenant_id, name, version)"
    );

    // Verify migration version tracking
    let migration_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = true")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        migration_count >= 5,
        "Expected at least 5 successful migrations, found {}",
        migration_count
    );

    // ── Rollback/forward-fix documentation ──
    // Workflow uses append-only migrations (no DROP/ALTER destructive).
    // Rollback strategy:
    //   1. Migration 5 (escalation+delegation): DROP TABLE workflow_delegation_rules,
    //      workflow_escalation_timers, workflow_escalation_rules;
    //   2. Migration 4 (holds): DROP TABLE workflow_holds;
    //   3. Migration 3 (routing): DROP TABLE workflow_step_decisions;
    //   4. Migration 2 (core): DROP TABLE workflow_idempotency_keys,
    //      workflow_transitions, workflow_instances, workflow_definitions;
    //   5. Migration 1 (outbox): DROP TABLE processed_events, events_outbox;
    //
    // Forward-fix preferred: if a migration fails mid-apply, fix and re-run.
    // SQLx tracks per-migration success so partial state is recoverable.
}

// ============================================================================
// 2. Tenant boundary — definitions and instances invisible across tenants
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_boundary_definitions() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    // Create definition under tenant A
    let def_a = create_test_definition(&pool, &tenant_a).await;

    // Tenant B lists definitions — must see zero rows
    let b_defs = DefinitionRepo::list(
        &pool,
        &ListDefinitionsQuery {
            tenant_id: tenant_b.clone(),
            active_only: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        b_defs.len(),
        0,
        "Tenant B must not see tenant A's definitions"
    );

    // Tenant B cannot get tenant A's definition by ID
    let b_get = DefinitionRepo::get(&pool, &tenant_b, def_a.id).await;
    assert!(
        b_get.is_err(),
        "Tenant B must not access tenant A's definition by ID"
    );

    // Tenant A sees their own definition
    let a_defs = DefinitionRepo::list(
        &pool,
        &ListDefinitionsQuery {
            tenant_id: tenant_a.clone(),
            active_only: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert!(
        a_defs.iter().any(|d| d.id == def_a.id),
        "Tenant A must see their own definition"
    );
}

#[tokio::test]
#[serial]
async fn test_tenant_boundary_instances() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    // Create definition and instance under tenant A
    let def_a = create_test_definition(&pool, &tenant_a).await;

    let instance_a = InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tenant_a.clone(),
            definition_id: def_a.id,
            entity_type: "document".into(),
            entity_id: format!("doc-{}", Uuid::new_v4().simple()),
            context: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    // Tenant B cannot see tenant A's instance
    let b_get = InstanceRepo::get(&pool, &tenant_b, instance_a.id).await;
    assert!(
        b_get.is_err(),
        "Tenant B must not access tenant A's instance by ID"
    );

    // Tenant B list instances — zero rows
    let b_instances = InstanceRepo::list(
        &pool,
        &ListInstancesQuery {
            tenant_id: tenant_b.clone(),
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
    assert_eq!(
        b_instances.len(),
        0,
        "Tenant B must not see tenant A's instances"
    );

    // Tenant B cannot list tenant A's transitions
    let b_transitions =
        InstanceRepo::list_transitions(&pool, &tenant_b, instance_a.id).await.unwrap();
    assert_eq!(
        b_transitions.len(),
        0,
        "Tenant B must not see tenant A's transitions"
    );

    // Tenant A can see their own instance
    let a_get = InstanceRepo::get(&pool, &tenant_a, instance_a.id).await.unwrap();
    assert_eq!(a_get.id, instance_a.id);
}

// ============================================================================
// 3. AuthZ denial — mutation endpoints reject unauthenticated requests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_authz_create_definition_denied_without_jwt() {
    let pool = setup_db().await;
    let app = build_test_router(pool);

    let body = json!({
        "name": "test-def",
        "steps": [{"step_id": "draft", "name": "Draft"}],
        "initial_step_id": "draft"
    });

    let req = HttpRequest::builder()
        .method("POST")
        .uri("/api/workflow/definitions")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "POST /definitions must reject without JWT"
    );
}

#[tokio::test]
#[serial]
async fn test_authz_start_instance_denied_without_jwt() {
    let pool = setup_db().await;
    let app = build_test_router(pool);

    let body = json!({
        "definition_id": Uuid::new_v4(),
        "entity_type": "document",
        "entity_id": "doc-123"
    });

    let req = HttpRequest::builder()
        .method("POST")
        .uri("/api/workflow/instances")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "POST /instances must reject without JWT"
    );
}

#[tokio::test]
#[serial]
async fn test_authz_advance_instance_denied_without_jwt() {
    let pool = setup_db().await;
    let app = build_test_router(pool);

    let body = json!({
        "to_step_id": "review",
        "action": "submit"
    });

    let req = HttpRequest::builder()
        .method("PATCH")
        .uri(format!(
            "/api/workflow/instances/{}/advance",
            Uuid::new_v4()
        ))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "PATCH /instances/:id/advance must reject without JWT"
    );
}

// ============================================================================
// 4. Guard→Mutation→Outbox atomicity
// ============================================================================

#[tokio::test]
#[serial]
async fn test_guard_mutation_outbox_definition_created() {
    let pool = setup_db().await;
    let tenant = unique_tenant();

    let def = create_test_definition(&pool, &tenant).await;

    // The outbox row must exist (written in the same transaction as the definition)
    let outbox_event: Option<(String, String)> = sqlx::query_as(
        "SELECT event_type, aggregate_id FROM events_outbox WHERE aggregate_id = $1",
    )
    .bind(def.id.to_string())
    .fetch_optional(&pool)
    .await
    .unwrap();

    let (event_type, agg_id) =
        outbox_event.expect("Outbox event must exist after definition creation");
    assert_eq!(event_type, "workflow.events.definition.created");
    assert_eq!(agg_id, def.id.to_string());

    // Verify outbox payload contains envelope fields
    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM events_outbox WHERE aggregate_id = $1",
    )
    .bind(def.id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(payload["tenant_id"], tenant);
    assert_eq!(payload["event_type"], "workflow.events.definition.created");
    assert!(payload["event_id"].is_string(), "envelope must have event_id");
    assert!(payload["source_version"].is_string(), "envelope must have source_version");
}

#[tokio::test]
#[serial]
async fn test_guard_mutation_outbox_instance_started() {
    let pool = setup_db().await;
    let tenant = unique_tenant();

    let def = create_test_definition(&pool, &tenant).await;

    let instance = InstanceRepo::start(
        &pool,
        &StartInstanceRequest {
            tenant_id: tenant.clone(),
            definition_id: def.id,
            entity_type: "document".into(),
            entity_id: format!("doc-{}", Uuid::new_v4().simple()),
            context: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    // Outbox event for the instance start
    let outbox_event: Option<(String,)> = sqlx::query_as(
        "SELECT event_type FROM events_outbox WHERE aggregate_id = $1 AND event_type = $2",
    )
    .bind(instance.id.to_string())
    .bind("workflow.events.instance.started")
    .fetch_optional(&pool)
    .await
    .unwrap();

    assert_eq!(
        outbox_event.unwrap().0,
        "workflow.events.instance.started",
        "Outbox must contain instance.started event"
    );

    // Verify the transition record was also created (part of atomic tx)
    let transitions = InstanceRepo::list_transitions(&pool, &tenant, instance.id)
        .await
        .unwrap();
    assert_eq!(
        transitions.len(),
        1,
        "Initial transition must be created atomically with instance"
    );
    assert_eq!(transitions[0].from_step_id, "__start__");
    assert_eq!(transitions[0].to_step_id, "draft");
}

// ============================================================================
// 5. Concurrent tenant isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_concurrent_tenant_isolation_definitions() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    let mut handles = Vec::new();

    // Spawn concurrent definition creates from both tenants
    for i in 0..5u32 {
        let p = pool.clone();
        let a = tenant_a.clone();
        handles.push(tokio::spawn(async move {
            DefinitionRepo::create(
                &p,
                &CreateDefinitionRequest {
                    tenant_id: a,
                    name: format!("concurrent-def-a-{}", i),
                    description: Some(format!("Tenant A def {}", i)),
                    steps: json!([
                        { "step_id": "start", "name": "Start" },
                        { "step_id": "end", "name": "End" }
                    ]),
                    initial_step_id: "start".into(),
                },
            )
            .await
            .expect("Tenant A definition create should succeed")
        }));

        let p = pool.clone();
        let b = tenant_b.clone();
        handles.push(tokio::spawn(async move {
            DefinitionRepo::create(
                &p,
                &CreateDefinitionRequest {
                    tenant_id: b,
                    name: format!("concurrent-def-b-{}", i),
                    description: Some(format!("Tenant B def {}", i)),
                    steps: json!([
                        { "step_id": "init", "name": "Init" },
                        { "step_id": "done", "name": "Done" }
                    ]),
                    initial_step_id: "init".into(),
                },
            )
            .await
            .expect("Tenant B definition create should succeed")
        }));
    }

    for h in handles {
        h.await.expect("join");
    }

    // Verify tenant A sees only their definitions
    let a_defs = DefinitionRepo::list(
        &pool,
        &ListDefinitionsQuery {
            tenant_id: tenant_a.clone(),
            active_only: None,
            limit: Some(100),
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(a_defs.len(), 5, "Tenant A should have 5 definitions");

    // Verify tenant B sees only their definitions
    let b_defs = DefinitionRepo::list(
        &pool,
        &ListDefinitionsQuery {
            tenant_id: tenant_b.clone(),
            active_only: None,
            limit: Some(100),
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(b_defs.len(), 5, "Tenant B should have 5 definitions");

    // Cross-tenant: no leaks
    for d in &a_defs {
        assert_eq!(d.tenant_id, tenant_a, "Tenant A def has wrong tenant_id");
    }
    for d in &b_defs {
        assert_eq!(d.tenant_id, tenant_b, "Tenant B def has wrong tenant_id");
    }
}

#[tokio::test]
#[serial]
async fn test_concurrent_tenant_isolation_instances() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    // Each tenant needs their own definition
    let def_a = create_test_definition(&pool, &tenant_a).await;
    let def_b = create_test_definition(&pool, &tenant_b).await;

    let mut handles = Vec::new();

    // Spawn concurrent instance starts from both tenants
    for i in 0..5u32 {
        let p = pool.clone();
        let a = tenant_a.clone();
        let da = def_a.id;
        handles.push(tokio::spawn(async move {
            InstanceRepo::start(
                &p,
                &StartInstanceRequest {
                    tenant_id: a,
                    definition_id: da,
                    entity_type: "document".into(),
                    entity_id: format!("doc-a-{}", i),
                    context: None,
                    idempotency_key: None,
                },
            )
            .await
            .expect("Tenant A instance start should succeed")
        }));

        let p = pool.clone();
        let b = tenant_b.clone();
        let db = def_b.id;
        handles.push(tokio::spawn(async move {
            InstanceRepo::start(
                &p,
                &StartInstanceRequest {
                    tenant_id: b,
                    definition_id: db,
                    entity_type: "order".into(),
                    entity_id: format!("order-b-{}", i),
                    context: None,
                    idempotency_key: None,
                },
            )
            .await
            .expect("Tenant B instance start should succeed")
        }));
    }

    for h in handles {
        h.await.expect("join");
    }

    // Verify tenant A sees only their instances
    let a_instances = InstanceRepo::list(
        &pool,
        &ListInstancesQuery {
            tenant_id: tenant_a.clone(),
            entity_type: None,
            entity_id: None,
            status: None,
            definition_id: Some(def_a.id),
            limit: Some(100),
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(a_instances.len(), 5, "Tenant A should have 5 instances");

    // Verify tenant B sees only their instances
    let b_instances = InstanceRepo::list(
        &pool,
        &ListInstancesQuery {
            tenant_id: tenant_b.clone(),
            entity_type: None,
            entity_id: None,
            status: None,
            definition_id: Some(def_b.id),
            limit: Some(100),
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(b_instances.len(), 5, "Tenant B should have 5 instances");

    // Cross-tenant: no leaks in instance data
    for inst in &a_instances {
        assert_eq!(inst.tenant_id, tenant_a, "Tenant A instance has wrong tenant_id");
    }
    for inst in &b_instances {
        assert_eq!(inst.tenant_id, tenant_b, "Tenant B instance has wrong tenant_id");
    }

    // Verify outbox events are per-tenant (no cross-contamination)
    let a_outbox: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox
         WHERE aggregate_type = 'workflow_instance'
           AND payload->>'tenant_id' = $1",
    )
    .bind(&tenant_a)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(a_outbox, 5, "Tenant A should have 5 instance outbox events");

    let b_outbox: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox
         WHERE aggregate_type = 'workflow_instance'
           AND payload->>'tenant_id' = $1",
    )
    .bind(&tenant_b)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(b_outbox, 5, "Tenant B should have 5 instance outbox events");
}
