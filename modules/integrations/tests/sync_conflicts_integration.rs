//! Integration tests for the sync conflicts persistence layer (bd-bnvqs).
//!
//! Covers:
//!  1.  Migration idempotency — re-running migrations succeeds
//!  2.  Schema validation — table, constraints, indexes present
//!  3.  Create edit conflict — happy path
//!  4.  Create creation conflict — happy path
//!  5.  Create deletion conflict — no values required
//!  6.  Guard: edit/creation without values rejected
//!  7.  Guard: value blob exceeding 256 KB rejected
//!  8.  Resolve conflict — happy path, internal_id required
//!  9.  Guard: resolve without internal_id rejected at DB level
//! 10.  Ignore and unresolvable transitions
//! 11.  Guard: transition from non-pending blocked
//! 12.  list_pending — scoped to pending status only
//! 13.  list_conflicts — optional status filter
//! 14.  Tenant isolation — cross-tenant read returns nothing
//! 15.  Concurrent create — each conflict gets distinct id

use integrations_rs::domain::sync::conflicts::{
    ConflictClass, ConflictError, ConflictStatus, CreateConflictRequest, ResolveConflictRequest,
    MAX_VALUE_BYTES,
};
use integrations_rs::domain::sync::conflicts_repo::{
    close_conflict, create_conflict, get_conflict, list_conflicts, list_pending, resolve_conflict,
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ── Setup helpers ─────────────────────────────────────────────────────────────

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

fn tenant() -> String {
    format!("conflict-test-{}", Uuid::new_v4().simple())
}

fn edit_req(app_id: &str) -> CreateConflictRequest {
    CreateConflictRequest {
        app_id: app_id.to_string(),
        provider: "qbo".to_string(),
        entity_type: "invoice".to_string(),
        entity_id: format!("inv-{}", Uuid::new_v4().simple()),
        conflict_class: ConflictClass::Edit,
        detected_by: "detector".to_string(),
        internal_value: Some(serde_json::json!({"amount": 100})),
        external_value: Some(serde_json::json!({"amount": 200})),
    }
}

fn creation_req(app_id: &str) -> CreateConflictRequest {
    CreateConflictRequest {
        app_id: app_id.to_string(),
        provider: "qbo".to_string(),
        entity_type: "invoice".to_string(),
        entity_id: format!("inv-{}", Uuid::new_v4().simple()),
        conflict_class: ConflictClass::Creation,
        detected_by: "detector".to_string(),
        internal_value: Some(serde_json::json!({"draft": true})),
        external_value: Some(serde_json::json!({"amount": 500})),
    }
}

fn deletion_req(app_id: &str) -> CreateConflictRequest {
    CreateConflictRequest {
        app_id: app_id.to_string(),
        provider: "qbo".to_string(),
        entity_type: "invoice".to_string(),
        entity_id: format!("inv-{}", Uuid::new_v4().simple()),
        conflict_class: ConflictClass::Deletion,
        detected_by: "detector".to_string(),
        internal_value: None,
        external_value: None,
    }
}

// ── 1. Migration idempotency ──────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn migration_is_idempotent() {
    let pool = setup_db().await;
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Re-running migrations must succeed");
}

// ── 2. Schema validation ──────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn schema_conflicts_table_exists_with_correct_structure() {
    let pool = setup_db().await;

    // Table exists
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM information_schema.tables
         WHERE table_schema = 'public' AND table_name = 'integrations_sync_conflicts'",
    )
    .fetch_one(&pool)
    .await
    .expect("table query");
    assert_eq!(count.0, 1, "integrations_sync_conflicts table must exist");

    // Required columns
    for col in &[
        "id", "app_id", "provider", "entity_type", "entity_id",
        "conflict_class", "status", "detected_by", "detected_at",
        "internal_value", "external_value", "internal_id",
        "resolved_by", "resolved_at", "resolution_note",
        "created_at", "updated_at",
    ] {
        let c: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM information_schema.columns
             WHERE table_name = 'integrations_sync_conflicts' AND column_name = $1",
        )
        .bind(col)
        .fetch_one(&pool)
        .await
        .expect("column query");
        assert_eq!(c.0, 1, "column '{}' must exist", col);
    }

    // Indexes
    for idx in &[
        "integrations_sync_conflicts_pending_idx",
        "integrations_sync_conflicts_entity_idx",
        "integrations_sync_conflicts_app_created_idx",
    ] {
        let c: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM pg_indexes
             WHERE tablename = 'integrations_sync_conflicts' AND indexname = $1",
        )
        .bind(idx)
        .fetch_one(&pool)
        .await
        .expect("index query");
        assert_eq!(c.0, 1, "index '{}' must exist", idx);
    }
}

// ── 3. Create edit conflict — happy path ──────────────────────────────────────

#[tokio::test]
#[serial]
async fn create_edit_conflict_happy_path() {
    let pool = setup_db().await;
    let tid = tenant();
    let row = create_conflict(&pool, &edit_req(&tid))
        .await
        .expect("create edit conflict");
    assert_eq!(row.status, "pending");
    assert_eq!(row.conflict_class, "edit");
    assert!(row.internal_value.is_some());
    assert!(row.external_value.is_some());
    assert_eq!(row.app_id, tid);
}

// ── 4. Create creation conflict — happy path ──────────────────────────────────

#[tokio::test]
#[serial]
async fn create_creation_conflict_happy_path() {
    let pool = setup_db().await;
    let tid = tenant();
    let row = create_conflict(&pool, &creation_req(&tid))
        .await
        .expect("create creation conflict");
    assert_eq!(row.status, "pending");
    assert_eq!(row.conflict_class, "creation");
}

// ── 5. Create deletion conflict — no values needed ────────────────────────────

#[tokio::test]
#[serial]
async fn create_deletion_conflict_no_values() {
    let pool = setup_db().await;
    let tid = tenant();
    let row = create_conflict(&pool, &deletion_req(&tid))
        .await
        .expect("create deletion conflict");
    assert_eq!(row.conflict_class, "deletion");
    assert!(row.internal_value.is_none());
    assert!(row.external_value.is_none());
}

// ── 6. Guard: edit/creation without values rejected ───────────────────────────

#[tokio::test]
#[serial]
async fn guard_edit_without_values_rejected() {
    let pool = setup_db().await;
    let tid = tenant();
    let mut req = edit_req(&tid);
    req.internal_value = None;
    let err = create_conflict(&pool, &req).await;
    assert!(
        matches!(err, Err(ConflictError::MissingValues)),
        "must reject edit conflict without values, got: {:?}",
        err
    );
}

#[tokio::test]
#[serial]
async fn guard_creation_without_external_value_rejected() {
    let pool = setup_db().await;
    let tid = tenant();
    let mut req = creation_req(&tid);
    req.external_value = None;
    let err = create_conflict(&pool, &req).await;
    assert!(
        matches!(err, Err(ConflictError::MissingValues)),
        "must reject creation conflict without external_value"
    );
}

// ── 7. Guard: value blob > 256 KB rejected ────────────────────────────────────

#[tokio::test]
#[serial]
async fn guard_value_too_large_rejected() {
    let pool = setup_db().await;
    let tid = tenant();
    let big = "x".repeat(MAX_VALUE_BYTES + 1);
    let mut req = edit_req(&tid);
    req.internal_value = Some(serde_json::json!(big));
    let err = create_conflict(&pool, &req).await;
    assert!(
        matches!(err, Err(ConflictError::ValueTooLarge)),
        "must reject oversized value blob"
    );
}

// ── 8. Resolve conflict — happy path ─────────────────────────────────────────

#[tokio::test]
#[serial]
async fn resolve_conflict_happy_path() {
    let pool = setup_db().await;
    let tid = tenant();
    let conflict = create_conflict(&pool, &edit_req(&tid))
        .await
        .expect("create");

    let resolved = resolve_conflict(
        &pool,
        &tid,
        conflict.id,
        &ResolveConflictRequest {
            internal_id: "inv-internal-001".to_string(),
            resolved_by: "operator".to_string(),
            resolution_note: Some("merged".to_string()),
        },
    )
    .await
    .expect("resolve");

    assert_eq!(resolved.status, "resolved");
    assert_eq!(resolved.internal_id.as_deref(), Some("inv-internal-001"));
    assert!(resolved.resolved_at.is_some());
    assert_eq!(resolved.resolved_by.as_deref(), Some("operator"));
}

// ── 9. Guard: resolve without internal_id rejected at DB level ────────────────

#[tokio::test]
#[serial]
async fn guard_resolve_without_internal_id_rejected() {
    let pool = setup_db().await;
    let tid = tenant();
    let conflict = create_conflict(&pool, &edit_req(&tid))
        .await
        .expect("create");

    // Attempt to set status=resolved but internal_id=NULL — DB constraint fires
    let result = sqlx::query(
        "UPDATE integrations_sync_conflicts
         SET status = 'resolved', updated_at = NOW()
         WHERE id = $1",
    )
    .bind(conflict.id)
    .execute(&pool)
    .await;

    assert!(
        result.is_err(),
        "DB constraint must reject resolved row without internal_id"
    );
}

// ── 10. Ignore and unresolvable transitions ────────────────────────────────────

#[tokio::test]
#[serial]
async fn close_conflict_ignored() {
    let pool = setup_db().await;
    let tid = tenant();
    let conflict = create_conflict(&pool, &edit_req(&tid))
        .await
        .expect("create");

    let row = close_conflict(
        &pool,
        &tid,
        conflict.id,
        ConflictStatus::Ignored,
        "operator",
        Some("duplicate"),
    )
    .await
    .expect("ignore");

    assert_eq!(row.status, "ignored");
    assert!(row.resolved_at.is_some());
}

#[tokio::test]
#[serial]
async fn close_conflict_unresolvable() {
    let pool = setup_db().await;
    let tid = tenant();
    let conflict = create_conflict(&pool, &edit_req(&tid))
        .await
        .expect("create");

    let row = close_conflict(
        &pool,
        &tid,
        conflict.id,
        ConflictStatus::Unresolvable,
        "system",
        None,
    )
    .await
    .expect("unresolvable");

    assert_eq!(row.status, "unresolvable");
}

// ── 11. Guard: transition from non-pending blocked ────────────────────────────

#[tokio::test]
#[serial]
async fn guard_double_transition_blocked() {
    let pool = setup_db().await;
    let tid = tenant();
    let conflict = create_conflict(&pool, &edit_req(&tid))
        .await
        .expect("create");

    // First transition: ignore
    close_conflict(
        &pool,
        &tid,
        conflict.id,
        ConflictStatus::Ignored,
        "op",
        None,
    )
    .await
    .expect("first close");

    // Second transition: should fail
    let err = close_conflict(
        &pool,
        &tid,
        conflict.id,
        ConflictStatus::Unresolvable,
        "op",
        None,
    )
    .await;

    assert!(
        matches!(err, Err(ConflictError::InvalidTransition(_, _))),
        "must block transition from non-pending status, got: {:?}",
        err
    );
}

// ── 12. list_pending — only pending rows returned ─────────────────────────────

#[tokio::test]
#[serial]
async fn list_pending_returns_only_pending() {
    let pool = setup_db().await;
    let tid = tenant();

    // Create 3 pending
    for _ in 0..3 {
        create_conflict(&pool, &edit_req(&tid)).await.expect("create");
    }

    // Create 1 and resolve it
    let to_resolve = create_conflict(&pool, &edit_req(&tid))
        .await
        .expect("create resolvable");
    resolve_conflict(
        &pool,
        &tid,
        to_resolve.id,
        &ResolveConflictRequest {
            internal_id: "some-id".to_string(),
            resolved_by: "op".to_string(),
            resolution_note: None,
        },
    )
    .await
    .expect("resolve");

    let pending = list_pending(&pool, &tid, "qbo", "invoice", 100, 0)
        .await
        .expect("list pending");

    assert_eq!(
        pending.len(),
        3,
        "must return exactly 3 pending conflicts, got {}",
        pending.len()
    );
    for row in &pending {
        assert_eq!(row.status, "pending", "all rows must be pending");
    }
}

// ── 13. list_conflicts — optional status filter ───────────────────────────────

#[tokio::test]
#[serial]
async fn list_conflicts_status_filter() {
    let pool = setup_db().await;
    let tid = tenant();

    let c1 = create_conflict(&pool, &edit_req(&tid)).await.expect("c1");
    let _c2 = create_conflict(&pool, &edit_req(&tid)).await.expect("c2");

    // Resolve c1
    resolve_conflict(
        &pool,
        &tid,
        c1.id,
        &ResolveConflictRequest {
            internal_id: "res-id".to_string(),
            resolved_by: "op".to_string(),
            resolution_note: None,
        },
    )
    .await
    .expect("resolve");

    let all = list_conflicts(&pool, &tid, None, 100, 0)
        .await
        .expect("list all");
    assert!(all.len() >= 2, "must return at least 2 rows");

    let resolved_only = list_conflicts(&pool, &tid, Some("resolved"), 100, 0)
        .await
        .expect("list resolved");
    assert!(
        resolved_only.iter().all(|r| r.status == "resolved"),
        "filtered result must contain only resolved rows"
    );

    let pending_only = list_conflicts(&pool, &tid, Some("pending"), 100, 0)
        .await
        .expect("list pending");
    assert!(
        pending_only.iter().all(|r| r.status == "pending"),
        "filtered result must contain only pending rows"
    );
}

// ── 14. Tenant isolation ──────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn tenant_isolation_cross_tenant_read_returns_nothing() {
    let pool = setup_db().await;
    let tid_a = tenant();
    let tid_b = tenant();

    let row = create_conflict(&pool, &edit_req(&tid_a))
        .await
        .expect("create in tenant A");

    // Tenant B cannot read tenant A's conflict
    let found = get_conflict(&pool, &tid_b, row.id)
        .await
        .expect("get from tenant B");
    assert!(
        found.is_none(),
        "tenant B must not see tenant A's conflict"
    );

    let list = list_conflicts(&pool, &tid_b, None, 100, 0)
        .await
        .expect("list for tenant B");
    assert!(
        list.iter().all(|r| r.id != row.id),
        "tenant B's list must not contain tenant A's conflict"
    );
}

// ── 15. Concurrent create — distinct IDs ─────────────────────────────────────

#[tokio::test]
#[serial]
async fn concurrent_create_produces_distinct_ids() {
    let pool = setup_db().await;
    let tid = tenant();

    let mut handles = vec![];
    for _ in 0..10 {
        let pool = pool.clone();
        let req = CreateConflictRequest {
            app_id: tid.clone(),
            provider: "qbo".to_string(),
            entity_type: "invoice".to_string(),
            entity_id: format!("inv-{}", Uuid::new_v4().simple()),
            conflict_class: ConflictClass::Edit,
            detected_by: "detector".to_string(),
            internal_value: Some(serde_json::json!({"v": 1})),
            external_value: Some(serde_json::json!({"v": 2})),
        };
        handles.push(tokio::spawn(async move {
            create_conflict(&pool, &req).await.expect("concurrent create")
        }));
    }

    let rows: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.expect("task panicked"))
        .collect();

    let ids: std::collections::HashSet<_> = rows.iter().map(|r| r.id).collect();
    assert_eq!(ids.len(), 10, "all 10 concurrent creates must produce distinct UUIDs");
}
