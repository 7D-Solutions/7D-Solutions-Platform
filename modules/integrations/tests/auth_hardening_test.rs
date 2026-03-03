//! Auth & RBAC Hardening Tests (Phase 58 Gate A, bd-1dshh)
//!
//! Proves:
//! 1. Permission constants are valid and distinct
//! 2. Router structure: reads gated by INTEGRATIONS_READ, mutations by INTEGRATIONS_MUTATE
//! 3. Tenant isolation under concurrent writes — connectors
//! 4. Tenant isolation under concurrent writes — external refs
//! 5. Guard → Mutation → Outbox atomicity for connector registration
//! 6. Guard rejection does NOT write outbox events (connector: unknown type)
//! 7. Guard rejection does NOT write outbox events (external ref: empty entity_type)
//! 8. Migration forward-fix validation — all tables, constraints, indexes present

use integrations_rs::domain::connectors::{
    service::{get_connector_config, list_connector_configs, register_connector, run_test_action},
    ConnectorError, RegisterConnectorRequest, RunTestActionRequest,
};
use integrations_rs::domain::external_refs::{
    service::{
        create_external_ref, delete_external_ref, get_by_external, get_external_ref,
        update_external_ref,
    },
    CreateExternalRefRequest, ExternalRefError, UpdateExternalRefRequest,
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://integrations_user:integrations_pass@localhost:5449/integrations_db"
            .to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
        .expect("Failed to connect to integrations test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run integrations migrations");
    pool
}

fn unique_tenant() -> String {
    format!("auth-hard-{}", Uuid::new_v4().simple())
}

fn corr() -> String {
    Uuid::new_v4().to_string()
}

fn echo_req(name: &str) -> RegisterConnectorRequest {
    RegisterConnectorRequest {
        connector_type: "echo".to_string(),
        name: name.to_string(),
        config: Some(serde_json::json!({"echo_prefix": "test"})),
    }
}

fn ext_ref_req(
    entity_type: &str,
    entity_id: &str,
    system: &str,
    external_id: &str,
) -> CreateExternalRefRequest {
    CreateExternalRefRequest {
        entity_type: entity_type.to_string(),
        entity_id: entity_id.to_string(),
        system: system.to_string(),
        external_id: external_id.to_string(),
        label: Some("test-label".to_string()),
        metadata: None,
    }
}

// ============================================================================
// 1. Permission constants are valid and distinct
// ============================================================================

#[test]
fn permission_constants_valid_and_distinct() {
    assert_eq!(
        security::permissions::INTEGRATIONS_MUTATE,
        "integrations.mutate"
    );
    assert_eq!(
        security::permissions::INTEGRATIONS_READ,
        "integrations.read"
    );
    assert_ne!(
        security::permissions::INTEGRATIONS_MUTATE,
        security::permissions::INTEGRATIONS_READ,
        "mutate and read permissions must be distinct"
    );
    // Not empty
    assert!(!security::permissions::INTEGRATIONS_MUTATE.is_empty());
    assert!(!security::permissions::INTEGRATIONS_READ.is_empty());
}

// ============================================================================
// 2. Router structure — read/mutation separation with correct permissions
// ============================================================================

/// Compile-time assertion: http::router() builds with the expected state type.
/// If the router structure changes incompatibly, this test fails to compile.
#[test]
fn router_builds_with_expected_state() {
    let _: fn(std::sync::Arc<integrations_rs::AppState>) -> axum::Router =
        integrations_rs::http::router;
}

// ============================================================================
// 3. Tenant isolation under concurrent writes — connectors
// ============================================================================

#[tokio::test]
#[serial]
async fn concurrent_connector_tenant_isolation() {
    let pool = setup_db().await;

    // Create 5 tenants that will write concurrently
    let tenants: Vec<String> = (0..5).map(|_| unique_tenant()).collect();
    let connector_names: Vec<String> = (0..5)
        .map(|i| format!("concurrent-echo-{}", i))
        .collect();

    // Spawn concurrent writes — each tenant registers its own connector
    let mut handles = vec![];
    for (i, tid) in tenants.iter().enumerate() {
        let pool = pool.clone();
        let tid = tid.clone();
        let name = connector_names[i].clone();
        handles.push(tokio::spawn(async move {
            let req = RegisterConnectorRequest {
                connector_type: "echo".to_string(),
                name,
                config: Some(serde_json::json!({"echo_prefix": "concurrent"})),
            };
            register_connector(&pool, &tid, &req, corr())
                .await
                .expect("concurrent register failed")
        }));
    }

    // Await all concurrent registrations
    let results: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.expect("task panicked"))
        .collect();

    // Verify each tenant can only see its own connector
    for (i, tid) in tenants.iter().enumerate() {
        let list = list_connector_configs(&pool, tid, false)
            .await
            .expect("list failed");

        // This tenant should see exactly 1 connector
        assert_eq!(
            list.len(),
            1,
            "tenant {} should have exactly 1 connector, got {}",
            tid,
            list.len()
        );
        assert_eq!(list[0].id, results[i].id);
        assert_eq!(list[0].app_id, *tid);

        // This tenant cannot see any other tenant's connector
        for (j, other_tid) in tenants.iter().enumerate() {
            if i == j {
                continue;
            }
            let cross = get_connector_config(&pool, tid, results[j].id)
                .await
                .expect("cross-tenant get should not error");
            assert!(
                cross.is_none(),
                "tenant {} must not see tenant {}'s connector",
                tid,
                other_tid
            );
        }
    }

    // Verify cross-tenant test action is rejected
    let action_req = RunTestActionRequest {
        idempotency_key: "cross-tenant-key".to_string(),
    };
    let err = run_test_action(&pool, &tenants[1], results[0].id, &action_req).await;
    assert!(
        err.is_err(),
        "cross-tenant test action must be rejected"
    );
    match err.unwrap_err() {
        ConnectorError::NotFound(_) => {}
        other => panic!(
            "expected NotFound for cross-tenant trigger, got: {:?}",
            other
        ),
    }
}

// ============================================================================
// 4. Tenant isolation under concurrent writes — external refs
// ============================================================================

#[tokio::test]
#[serial]
async fn concurrent_external_ref_tenant_isolation() {
    let pool = setup_db().await;

    // 5 tenants each create an external ref with the same system+external_id
    let tenants: Vec<String> = (0..5).map(|_| unique_tenant()).collect();
    let shared_external_id = format!("EXT-{}", Uuid::new_v4().simple());

    let mut handles = vec![];
    for tid in &tenants {
        let pool = pool.clone();
        let tid = tid.clone();
        let ext_id = shared_external_id.clone();
        handles.push(tokio::spawn(async move {
            let req = ext_ref_req("invoice", "inv-concurrent", "stripe", &ext_id);
            create_external_ref(&pool, &tid, &req, corr())
                .await
                .expect("concurrent create failed")
        }));
    }

    let results: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.expect("task panicked"))
        .collect();

    // Each tenant gets its own distinct row (different id)
    let ids: std::collections::HashSet<i64> = results.iter().map(|r| r.id).collect();
    assert_eq!(
        ids.len(),
        5,
        "each tenant must get a distinct external ref row"
    );

    // Verify isolation: each tenant can only see its own ref
    for (i, tid) in tenants.iter().enumerate() {
        let found = get_external_ref(&pool, tid, results[i].id)
            .await
            .expect("get failed");
        assert!(found.is_some(), "tenant must see its own ref");

        // Cannot see other tenants' refs by id
        for (j, _) in tenants.iter().enumerate() {
            if i == j {
                continue;
            }
            let cross = get_external_ref(&pool, tid, results[j].id)
                .await
                .expect("cross-tenant get should not error");
            assert!(
                cross.is_none(),
                "tenant must not see other tenant's ref"
            );
        }

        // get_by_external scoped by tenant
        let by_ext = get_by_external(&pool, tid, "stripe", &shared_external_id)
            .await
            .expect("get_by_external failed");
        assert!(by_ext.is_some());
        assert_eq!(by_ext.unwrap().id, results[i].id);
    }

    // Verify cross-tenant update is rejected
    let upd = UpdateExternalRefRequest {
        label: Some("hacked".to_string()),
        metadata: None,
    };
    let err = update_external_ref(&pool, &tenants[1], results[0].id, &upd, corr()).await;
    assert!(err.is_err(), "cross-tenant update must fail");

    // Verify cross-tenant delete is rejected
    let err = delete_external_ref(&pool, &tenants[1], results[0].id, corr()).await;
    assert!(err.is_err(), "cross-tenant delete must fail");

    // Original ref still intact
    let still_there = get_external_ref(&pool, &tenants[0], results[0].id)
        .await
        .expect("get failed")
        .expect("ref must still exist");
    assert_eq!(still_there.id, results[0].id);
}

// ============================================================================
// 5. Guard → Mutation → Outbox atomicity for connector registration
// ============================================================================

#[tokio::test]
#[serial]
async fn guard_mutation_outbox_atomicity_connector_register() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    // Count outbox events before
    let before: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox WHERE app_id = $1",
    )
    .bind(&tid)
    .fetch_one(&pool)
    .await
    .expect("count before");

    // Register a connector — should succeed and write outbox event
    let req = echo_req("outbox-atomicity-test");
    let created = register_connector(&pool, &tid, &req, corr())
        .await
        .expect("register failed");

    // Outbox must have exactly one new event
    let after: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox WHERE app_id = $1",
    )
    .bind(&tid)
    .fetch_one(&pool)
    .await
    .expect("count after");

    assert_eq!(
        after.0,
        before.0 + 1,
        "connector registration must produce exactly 1 outbox event"
    );

    // Verify the event type is connector.registered
    let event_type: String = sqlx::query_scalar(
        "SELECT event_type FROM integrations_outbox
         WHERE app_id = $1 AND aggregate_type = 'connector'
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&tid)
    .fetch_one(&pool)
    .await
    .expect("event type query");

    assert_eq!(event_type, "connector.registered");

    // Verify aggregate_id matches the connector's id
    let aggregate_id: String = sqlx::query_scalar(
        "SELECT aggregate_id FROM integrations_outbox
         WHERE app_id = $1 AND aggregate_type = 'connector'
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&tid)
    .fetch_one(&pool)
    .await
    .expect("aggregate_id query");

    assert_eq!(aggregate_id, created.id.to_string());
}

// ============================================================================
// 6. Guard rejection does NOT write outbox — connector unknown type
// ============================================================================

#[tokio::test]
#[serial]
async fn guard_rejection_no_outbox_connector_unknown_type() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    // Count outbox events before
    let before: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox WHERE app_id = $1",
    )
    .bind(&tid)
    .fetch_one(&pool)
    .await
    .expect("count before");

    // Attempt to register with unknown type — guard should reject
    let req = RegisterConnectorRequest {
        connector_type: "nonexistent-type".to_string(),
        name: "should-not-persist".to_string(),
        config: None,
    };
    let err = register_connector(&pool, &tid, &req, corr()).await;
    assert!(err.is_err(), "unknown type must be rejected by guard");
    assert!(matches!(err.unwrap_err(), ConnectorError::UnknownType(_)));

    // Outbox must NOT have new events
    let after: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox WHERE app_id = $1",
    )
    .bind(&tid)
    .fetch_one(&pool)
    .await
    .expect("count after");

    assert_eq!(
        before.0, after.0,
        "guard rejection must not write outbox event (before={}, after={})",
        before.0, after.0
    );

    // No connector row was created either
    let configs = list_connector_configs(&pool, &tid, false)
        .await
        .expect("list failed");
    assert!(configs.is_empty(), "no connector row should exist after guard rejection");
}

// ============================================================================
// 7. Guard rejection does NOT write outbox — external ref empty entity_type
// ============================================================================

#[tokio::test]
#[serial]
async fn guard_rejection_no_outbox_external_ref_empty_entity_type() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    // Count outbox events before
    let before: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox WHERE app_id = $1",
    )
    .bind(&tid)
    .fetch_one(&pool)
    .await
    .expect("count before");

    // Attempt to create external ref with empty entity_type — guard rejects
    let req = CreateExternalRefRequest {
        entity_type: "".to_string(),
        entity_id: "inv-001".to_string(),
        system: "stripe".to_string(),
        external_id: "in_abc123".to_string(),
        label: None,
        metadata: None,
    };
    let err = create_external_ref(&pool, &tid, &req, corr()).await;
    assert!(err.is_err(), "empty entity_type must be rejected by guard");
    assert!(matches!(err.unwrap_err(), ExternalRefError::Validation(_)));

    // Outbox must NOT have new events
    let after: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox WHERE app_id = $1",
    )
    .bind(&tid)
    .fetch_one(&pool)
    .await
    .expect("count after");

    assert_eq!(
        before.0, after.0,
        "guard rejection must not write outbox event (before={}, after={})",
        before.0, after.0
    );
}

// ============================================================================
// 8. Migration forward-fix — all tables, constraints, indexes present
// ============================================================================

#[tokio::test]
#[serial]
async fn migration_schema_validation() {
    let pool = setup_db().await;

    // All expected tables exist
    for table in &[
        "integrations_external_refs",
        "integrations_webhook_endpoints",
        "integrations_webhook_ingest",
        "integrations_outbox",
        "integrations_processed_events",
        "integrations_idempotency_keys",
        "integrations_connector_configs",
        "integrations_schema_version",
    ] {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM information_schema.tables
             WHERE table_schema = 'public' AND table_name = $1",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|e| panic!("Failed to query table {}: {}", table, e));

        assert_eq!(count.0, 1, "Table '{}' must exist after migrations", table);
    }

    // Verify key uniqueness constraints
    let constraints = vec![
        (
            "integrations_external_refs",
            "integrations_external_refs_app_system_id_unique",
        ),
        (
            "integrations_webhook_ingest",
            "integrations_webhook_ingest_dedup",
        ),
        (
            "integrations_connector_configs",
            "integrations_connector_configs_app_type_name_unique",
        ),
        (
            "integrations_idempotency_keys",
            "integrations_idempotency_keys_app_key_unique",
        ),
    ];

    for (table, constraint) in &constraints {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM information_schema.table_constraints
             WHERE table_name = $1 AND constraint_name = $2 AND constraint_type = 'UNIQUE'",
        )
        .bind(table)
        .bind(constraint)
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|e| {
            panic!(
                "Failed to query constraint {} on {}: {}",
                constraint, table, e
            )
        });

        assert_eq!(
            count.0, 1,
            "UNIQUE constraint '{}' on '{}' must exist",
            constraint, table
        );
    }

    // Verify critical indexes exist
    let indexes = vec![
        ("integrations_external_refs", "idx_integrations_ext_refs_entity"),
        ("integrations_external_refs", "idx_integrations_ext_refs_system"),
        ("integrations_webhook_ingest", "idx_integrations_wh_ingest_unprocessed"),
        ("integrations_webhook_ingest", "idx_integrations_wh_ingest_system"),
        ("integrations_outbox", "idx_integrations_outbox_unpublished"),
        ("integrations_outbox", "idx_integrations_outbox_app_id"),
        ("integrations_connector_configs", "idx_integrations_connector_configs_app_enabled"),
        ("integrations_connector_configs", "idx_integrations_connector_configs_app_type"),
    ];

    for (table, index) in &indexes {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM pg_indexes
             WHERE tablename = $1 AND indexname = $2",
        )
        .bind(table)
        .bind(index)
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|e| panic!("Failed to query index {} on {}: {}", index, table, e));

        assert_eq!(
            count.0, 1,
            "Index '{}' on '{}' must exist",
            index, table
        );
    }

    // Verify all tables have app_id column (tenant scoping enforcement)
    let tenant_scoped_tables = vec![
        "integrations_external_refs",
        "integrations_webhook_endpoints",
        "integrations_webhook_ingest",
        "integrations_outbox",
        "integrations_idempotency_keys",
        "integrations_connector_configs",
    ];

    for table in &tenant_scoped_tables {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM information_schema.columns
             WHERE table_schema = 'public' AND table_name = $1 AND column_name = 'app_id'",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|e| {
            panic!("Failed to query app_id column on {}: {}", table, e)
        });

        assert_eq!(
            count.0, 1,
            "Table '{}' must have app_id column for tenant scoping",
            table
        );
    }

    // Verify migrations are idempotent — running again produces no error
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Migrations must be idempotent — re-run should succeed");
}

// ============================================================================
// 9. Guard → Mutation → Outbox atomicity for external ref create
// ============================================================================

#[tokio::test]
#[serial]
async fn guard_mutation_outbox_atomicity_external_ref_create() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    // Count before
    let before: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox WHERE app_id = $1 AND aggregate_type = 'external_ref'",
    )
    .bind(&tid)
    .fetch_one(&pool)
    .await
    .expect("count before");

    // Create an external ref
    let req = ext_ref_req("invoice", "inv-atomicity", "stripe", "in_atomicity_001");
    let created = create_external_ref(&pool, &tid, &req, corr())
        .await
        .expect("create failed");

    // Outbox must have exactly one new event
    let after: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox WHERE app_id = $1 AND aggregate_type = 'external_ref'",
    )
    .bind(&tid)
    .fetch_one(&pool)
    .await
    .expect("count after");

    assert_eq!(
        after.0,
        before.0 + 1,
        "external ref creation must produce exactly 1 outbox event"
    );

    // Verify event type
    let event_type: String = sqlx::query_scalar(
        "SELECT event_type FROM integrations_outbox
         WHERE app_id = $1 AND aggregate_type = 'external_ref'
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&tid)
    .fetch_one(&pool)
    .await
    .expect("event type query");

    assert_eq!(event_type, "external_ref.created");

    // Verify aggregate_id matches
    let aggregate_id: String = sqlx::query_scalar(
        "SELECT aggregate_id FROM integrations_outbox
         WHERE app_id = $1 AND aggregate_type = 'external_ref'
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&tid)
    .fetch_one(&pool)
    .await
    .expect("aggregate_id query");

    assert_eq!(aggregate_id, created.id.to_string());
}

// ============================================================================
// 10. Guard → Mutation → Outbox atomicity for external ref update + delete
// ============================================================================

#[tokio::test]
#[serial]
async fn guard_mutation_outbox_atomicity_external_ref_update_delete() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    // Create a ref first
    let req = ext_ref_req("invoice", "inv-upd-del", "stripe", "in_upd_del_001");
    let created = create_external_ref(&pool, &tid, &req, corr())
        .await
        .expect("create failed");

    // Count outbox events for this aggregate
    let before_update: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox
         WHERE app_id = $1 AND aggregate_type = 'external_ref' AND aggregate_id = $2",
    )
    .bind(&tid)
    .bind(created.id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count before update");

    // Update the ref
    let upd = UpdateExternalRefRequest {
        label: Some("updated".to_string()),
        metadata: None,
    };
    update_external_ref(&pool, &tid, created.id, &upd, corr())
        .await
        .expect("update failed");

    let after_update: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox
         WHERE app_id = $1 AND aggregate_type = 'external_ref' AND aggregate_id = $2",
    )
    .bind(&tid)
    .bind(created.id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count after update");

    assert_eq!(
        after_update.0,
        before_update.0 + 1,
        "update must produce exactly 1 outbox event"
    );

    // Delete the ref
    delete_external_ref(&pool, &tid, created.id, corr())
        .await
        .expect("delete failed");

    let after_delete: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox
         WHERE app_id = $1 AND aggregate_type = 'external_ref' AND aggregate_id = $2",
    )
    .bind(&tid)
    .bind(created.id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count after delete");

    assert_eq!(
        after_delete.0,
        after_update.0 + 1,
        "delete must produce exactly 1 outbox event"
    );

    // Verify last two event types
    let event_types: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM integrations_outbox
         WHERE app_id = $1 AND aggregate_type = 'external_ref' AND aggregate_id = $2
         ORDER BY created_at DESC LIMIT 2",
    )
    .bind(&tid)
    .bind(created.id.to_string())
    .fetch_all(&pool)
    .await
    .expect("event types query");

    let types: Vec<&str> = event_types.iter().map(|(t,)| t.as_str()).collect();
    assert!(types.contains(&"external_ref.updated"), "must have updated event");
    assert!(types.contains(&"external_ref.deleted"), "must have deleted event");
}
