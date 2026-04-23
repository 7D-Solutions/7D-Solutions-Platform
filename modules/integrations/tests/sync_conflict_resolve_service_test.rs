//! Integration tests for resolve_conflict_transactional (bd-meaqw).
//!
//! Verifies:
//!  1.  Happy path — conflict transitions to `resolved`, outbox row written in same tx.
//!  2.  Explicit entity-type dispatch — customer, invoice, payment all accepted.
//!  3.  Guard: empty internal_id rejected before any DB call.
//!  4.  Guard: non-pending conflict → InvalidTransition.
//!  5.  Guard: not-found conflict → NotFound.
//!  6.  Guard: unsupported entity_type → UnsupportedEntityType.
//!  7.  Event payload contract — fields in outbox row match the resolved conflict row.
//!  8.  Tenant isolation — cannot resolve another tenant's conflict.

use integrations_rs::domain::sync::conflicts::{
    ConflictClass, ConflictStatus, CreateConflictRequest,
};
use integrations_rs::domain::sync::conflicts_repo::{close_conflict, create_conflict};
use integrations_rs::domain::sync::resolve_service::{
    resolve_conflict_transactional, ResolveConflictError,
};
use integrations_rs::events::EVENT_TYPE_SYNC_CONFLICT_RESOLVED;
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
    format!("resolve-svc-test-{}", Uuid::new_v4().simple())
}

fn edit_conflict(app_id: &str, entity_type: &str) -> CreateConflictRequest {
    CreateConflictRequest {
        app_id: app_id.to_string(),
        provider: "quickbooks".to_string(),
        entity_type: entity_type.to_string(),
        entity_id: format!("ent-{}", Uuid::new_v4().simple()),
        conflict_class: ConflictClass::Edit,
        detected_by: "detector".to_string(),
        internal_value: Some(serde_json::json!({"amount": 100})),
        external_value: Some(serde_json::json!({"amount": 200})),
    }
}

// ── 1. Happy path ─────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn resolve_conflict_happy_path_transitions_status_and_writes_outbox() {
    let pool = setup_db().await;
    let tid = tenant();
    let conflict = create_conflict(&pool, &edit_conflict(&tid, "invoice"))
        .await
        .expect("create conflict");

    let resolved = resolve_conflict_transactional(
        &pool,
        &tid,
        conflict.id,
        "operator",
        "inv-internal-001",
        Some("merged platform value"),
    )
    .await
    .expect("resolve conflict");

    assert_eq!(resolved.status, "resolved");
    assert_eq!(resolved.internal_id.as_deref(), Some("inv-internal-001"));
    assert!(resolved.resolved_at.is_some());
    assert_eq!(resolved.resolved_by.as_deref(), Some("operator"));
    assert_eq!(
        resolved.resolution_note.as_deref(),
        Some("merged platform value")
    );

    // Outbox row must exist for the resolved event.
    let outbox_row: Option<(String, String, serde_json::Value)> = sqlx::query_as(
        "SELECT event_type, aggregate_id, payload
         FROM integrations_outbox
         WHERE aggregate_type = 'sync_conflict' AND aggregate_id = $1
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(conflict.id.to_string())
    .fetch_optional(&pool)
    .await
    .expect("outbox query");

    let (event_type, agg_id, payload) = outbox_row.expect("outbox row must exist");
    assert_eq!(event_type, EVENT_TYPE_SYNC_CONFLICT_RESOLVED);
    assert_eq!(agg_id, conflict.id.to_string());
    assert_eq!(payload["payload"]["conflict_id"], conflict.id.to_string());
    assert_eq!(payload["payload"]["entity_type"], "invoice");
    assert_eq!(payload["payload"]["internal_id"], "inv-internal-001");
    assert_eq!(payload["payload"]["resolved_by"], "operator");
}

// ── 2. All supported entity types dispatch successfully ───────────────────────

#[tokio::test]
#[serial]
async fn explicit_dispatch_accepts_all_supported_entity_types() {
    let pool = setup_db().await;

    for entity_type in &["customer", "invoice", "payment"] {
        let tid = tenant();
        let conflict = create_conflict(&pool, &edit_conflict(&tid, entity_type))
            .await
            .expect("create conflict");

        let result =
            resolve_conflict_transactional(&pool, &tid, conflict.id, "op", "internal-001", None)
                .await;

        assert!(
            result.is_ok(),
            "entity_type '{}' must be accepted; got: {:?}",
            entity_type,
            result.err()
        );
        assert_eq!(result.unwrap().status, "resolved");
    }
}

// ── 3. Guard: empty internal_id rejected ─────────────────────────────────────

#[tokio::test]
#[serial]
async fn guard_empty_internal_id_rejected_before_db() {
    let pool = setup_db().await;
    let tid = tenant();
    let conflict = create_conflict(&pool, &edit_conflict(&tid, "invoice"))
        .await
        .expect("create conflict");

    let err = resolve_conflict_transactional(&pool, &tid, conflict.id, "op", "", None)
        .await
        .expect_err("must reject empty internal_id");

    assert!(
        matches!(err, ResolveConflictError::MissingInternalId),
        "expected MissingInternalId, got: {:?}",
        err
    );
}

// ── 4. Guard: non-pending conflict → InvalidTransition ───────────────────────

#[tokio::test]
#[serial]
async fn guard_non_pending_conflict_returns_invalid_transition() {
    let pool = setup_db().await;
    let tid = tenant();
    let conflict = create_conflict(&pool, &edit_conflict(&tid, "invoice"))
        .await
        .expect("create conflict");

    // Close it first.
    close_conflict(
        &pool,
        &tid,
        conflict.id,
        ConflictStatus::Ignored,
        "op",
        None,
    )
    .await
    .expect("close conflict");

    let err = resolve_conflict_transactional(&pool, &tid, conflict.id, "op", "internal-001", None)
        .await
        .expect_err("must reject transition from non-pending");

    assert!(
        matches!(err, ResolveConflictError::InvalidTransition(_, _)),
        "expected InvalidTransition, got: {:?}",
        err
    );
}

// ── 5. Guard: not-found conflict → NotFound ───────────────────────────────────

#[tokio::test]
#[serial]
async fn guard_not_found_conflict_returns_not_found() {
    let pool = setup_db().await;
    let tid = tenant();

    let err =
        resolve_conflict_transactional(&pool, &tid, Uuid::new_v4(), "op", "internal-001", None)
            .await
            .expect_err("must return NotFound for unknown UUID");

    assert!(
        matches!(err, ResolveConflictError::NotFound(_)),
        "expected NotFound, got: {:?}",
        err
    );
}

// ── 6. Guard: unsupported entity_type → UnsupportedEntityType ────────────────

#[tokio::test]
#[serial]
async fn guard_unsupported_entity_type_rejected() {
    let pool = setup_db().await;
    let tid = tenant();

    // Insert a conflict directly with an unsupported entity_type.
    let conflict = sqlx::query_as::<_, integrations_rs::domain::sync::conflicts::ConflictRow>(
        r#"
        INSERT INTO integrations_sync_conflicts (
            app_id, provider, entity_type, entity_id,
            conflict_class, detected_by,
            internal_value, external_value
        )
        VALUES ($1, 'quickbooks', 'vendor', $2, 'edit', 'detector', $3, $4)
        RETURNING
            id, app_id, provider, entity_type, entity_id,
            conflict_class, status, detected_by, detected_at,
            internal_value, external_value, internal_id,
            resolved_by, resolved_at, resolution_note,
            created_at, updated_at
        "#,
    )
    .bind(&tid)
    .bind(format!("vendor-{}", Uuid::new_v4().simple()))
    .bind(serde_json::json!({"name": "Vendor A"}))
    .bind(serde_json::json!({"name": "Vendor B"}))
    .fetch_one(&pool)
    .await
    .expect("insert vendor conflict");

    let err = resolve_conflict_transactional(&pool, &tid, conflict.id, "op", "internal-001", None)
        .await
        .expect_err("must reject unsupported entity_type");

    assert!(
        matches!(err, ResolveConflictError::UnsupportedEntityType(_)),
        "expected UnsupportedEntityType, got: {:?}",
        err
    );
}

// ── 7. Event payload contract ─────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn event_payload_matches_conflict_row_fields() {
    let pool = setup_db().await;
    let tid = tenant();
    let conflict = create_conflict(&pool, &edit_conflict(&tid, "customer"))
        .await
        .expect("create conflict");

    resolve_conflict_transactional(
        &pool,
        &tid,
        conflict.id,
        "alice",
        "cust-internal-999",
        Some("resolved via manual review"),
    )
    .await
    .expect("resolve");

    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM integrations_outbox
         WHERE aggregate_type = 'sync_conflict' AND aggregate_id = $1
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(conflict.id.to_string())
    .fetch_one(&pool)
    .await
    .expect("outbox row");

    assert_eq!(payload["event_type"], EVENT_TYPE_SYNC_CONFLICT_RESOLVED);
    assert_eq!(payload["payload"]["app_id"], tid.as_str());
    assert_eq!(payload["payload"]["conflict_id"], conflict.id.to_string());
    assert_eq!(payload["payload"]["provider"], "quickbooks");
    assert_eq!(payload["payload"]["entity_type"], "customer");
    assert_eq!(payload["payload"]["conflict_class"], "edit");
    assert_eq!(payload["payload"]["resolved_by"], "alice");
    assert_eq!(payload["payload"]["internal_id"], "cust-internal-999");
    assert_eq!(
        payload["payload"]["resolution_note"],
        "resolved via manual review"
    );
}

// ── 8. Tenant isolation ───────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn tenant_isolation_cannot_resolve_other_tenants_conflict() {
    let pool = setup_db().await;
    let tid_a = tenant();
    let tid_b = tenant();

    let conflict = create_conflict(&pool, &edit_conflict(&tid_a, "invoice"))
        .await
        .expect("create conflict for tenant A");

    let err =
        resolve_conflict_transactional(&pool, &tid_b, conflict.id, "op", "internal-001", None)
            .await
            .expect_err("tenant B must not resolve tenant A's conflict");

    assert!(
        matches!(err, ResolveConflictError::NotFound(_)),
        "expected NotFound when wrong tenant resolves, got: {:?}",
        err
    );
}
