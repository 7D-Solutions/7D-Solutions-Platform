//! Integration tests for bulk_resolve_conflicts (bd-tzizs).
//!
//! Verifies:
//!  1.  Happy path — resolve / ignore / unresolvable each produce the correct outcome.
//!  2.  Mixed-success batch — some items succeed, others fail per-item (error isolation).
//!  3.  Deterministic-key dedupe — retrying with the same items returns AlreadyResolved /
//!      AlreadyIgnored / AlreadyUnresolvable, not re-applying the transition.
//!  4.  Caller-key aliasing — server det-key controls dedupe regardless of caller key.
//!  5.  TerminalByOther — conflict already terminal under a different det-key → correct outcome.
//!  6.  Capacity guard — submitting > 100 items returns ExceedsCapacity.
//!  7.  Per-item guards — invalid_action, missing_internal_id, not_found, unsupported_entity.
//!  8.  Tenant isolation — cannot bulk-resolve another tenant's conflicts.
//!  9.  Outbox event — resolve action emits integrations.sync.conflict.resolved in same tx.
//! 10.  Explicit entity-type + conflict-class dispatch — all supported (entity, class) pairs accepted.
//!
//! Run:
//!   ./scripts/cargo-slot.sh test -p integrations-rs --test sync_bulk_resolve_test

use integrations_rs::domain::sync::conflicts::{ConflictClass, CreateConflictRequest};
use integrations_rs::domain::sync::conflicts_repo::{create_conflict, get_conflict};
use integrations_rs::domain::sync::dedupe::compute_resolve_det_key;
use integrations_rs::domain::sync::resolve_service::{
    bulk_resolve_conflicts, BulkResolveError, BulkResolveItem, BulkResolveOutcome, BULK_RESOLVE_CAP,
};
use integrations_rs::events::EVENT_TYPE_SYNC_CONFLICT_RESOLVED;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
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
        .acquire_timeout(Duration::from_secs(90))
        .connect(&url)
        .await
        .expect("Failed to connect to integrations test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    pool
}

fn tenant() -> String {
    format!("bulk-resolve-test-{}", Uuid::new_v4().simple())
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

fn resolve_item(conflict_id: Uuid, authority_version: i64) -> BulkResolveItem {
    BulkResolveItem {
        conflict_id,
        action: "resolve".to_string(),
        authority_version,
        internal_id: Some("int-001".to_string()),
        resolution_note: Some("bulk test".to_string()),
        caller_idempotency_key: None,
    }
}

fn ignore_item(conflict_id: Uuid, authority_version: i64) -> BulkResolveItem {
    BulkResolveItem {
        conflict_id,
        action: "ignore".to_string(),
        authority_version,
        internal_id: None,
        resolution_note: None,
        caller_idempotency_key: None,
    }
}

fn unresolvable_item(conflict_id: Uuid, authority_version: i64) -> BulkResolveItem {
    BulkResolveItem {
        conflict_id,
        action: "unresolvable".to_string(),
        authority_version,
        internal_id: None,
        resolution_note: None,
        caller_idempotency_key: None,
    }
}

async fn cleanup(pool: &sqlx::PgPool, app_id: &str) {
    let _ = sqlx::query("DELETE FROM integrations_outbox WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM integrations_sync_conflicts WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await;
}

// ── 1. Happy path: resolve / ignore / unresolvable ────────────────────────────

#[tokio::test]
#[serial]
async fn bulk_resolve_resolve_action_transitions_to_resolved() {
    let pool = setup_db().await;
    let tid = tenant();
    let conflict = create_conflict(&pool, &edit_conflict(&tid, "invoice"))
        .await
        .expect("create conflict");

    let outcomes =
        bulk_resolve_conflicts(&pool, &tid, "operator", vec![resolve_item(conflict.id, 1)])
            .await
            .expect("bulk resolve");

    assert_eq!(outcomes.len(), 1);
    let det_key = compute_resolve_det_key(conflict.id, "resolve", 1);
    assert!(
        matches!(&outcomes[0], BulkResolveOutcome::Resolved { conflict_id, deterministic_key, .. }
            if *conflict_id == conflict.id && *deterministic_key == det_key),
        "unexpected outcome: {:?}",
        outcomes[0]
    );

    let row = get_conflict(&pool, &tid, conflict.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, "resolved");
    assert_eq!(
        row.resolution_idempotency_key.as_deref(),
        Some(det_key.as_str())
    );

    cleanup(&pool, &tid).await;
}

#[tokio::test]
#[serial]
async fn bulk_resolve_ignore_action_transitions_to_ignored() {
    let pool = setup_db().await;
    let tid = tenant();
    let conflict = create_conflict(&pool, &edit_conflict(&tid, "customer"))
        .await
        .expect("create");

    let outcomes =
        bulk_resolve_conflicts(&pool, &tid, "operator", vec![ignore_item(conflict.id, 2)])
            .await
            .expect("bulk resolve");

    assert!(
        matches!(&outcomes[0], BulkResolveOutcome::Ignored { conflict_id, .. } if *conflict_id == conflict.id),
        "{:?}",
        outcomes[0]
    );
    let row = get_conflict(&pool, &tid, conflict.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, "ignored");

    cleanup(&pool, &tid).await;
}

#[tokio::test]
#[serial]
async fn bulk_resolve_unresolvable_action_marks_unresolvable() {
    let pool = setup_db().await;
    let tid = tenant();
    let conflict = create_conflict(&pool, &edit_conflict(&tid, "payment"))
        .await
        .expect("create");

    let outcomes = bulk_resolve_conflicts(
        &pool,
        &tid,
        "operator",
        vec![unresolvable_item(conflict.id, 3)],
    )
    .await
    .expect("bulk resolve");

    assert!(
        matches!(&outcomes[0], BulkResolveOutcome::MarkedUnresolvable { conflict_id, .. } if *conflict_id == conflict.id),
        "{:?}",
        outcomes[0]
    );
    let row = get_conflict(&pool, &tid, conflict.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, "unresolvable");

    cleanup(&pool, &tid).await;
}

// ── 2. Mixed-success batch ────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn bulk_resolve_mixed_success_batch_processes_all_items() {
    let pool = setup_db().await;
    let tid = tenant();

    let c1 = create_conflict(&pool, &edit_conflict(&tid, "invoice"))
        .await
        .unwrap();
    let c2 = create_conflict(&pool, &edit_conflict(&tid, "customer"))
        .await
        .unwrap();
    let phantom_id = Uuid::new_v4(); // doesn't exist

    let outcomes = bulk_resolve_conflicts(
        &pool,
        &tid,
        "operator",
        vec![
            resolve_item(c1.id, 1),
            BulkResolveItem {
                conflict_id: phantom_id,
                action: "resolve".to_string(),
                authority_version: 1,
                internal_id: Some("x".to_string()),
                resolution_note: None,
                caller_idempotency_key: None,
            },
            ignore_item(c2.id, 1),
        ],
    )
    .await
    .expect("bulk resolve");

    assert_eq!(outcomes.len(), 3);
    assert!(
        matches!(&outcomes[0], BulkResolveOutcome::Resolved { .. }),
        "{:?}",
        outcomes[0]
    );
    assert!(
        matches!(&outcomes[1], BulkResolveOutcome::NotFound { .. }),
        "{:?}",
        outcomes[1]
    );
    assert!(
        matches!(&outcomes[2], BulkResolveOutcome::Ignored { .. }),
        "{:?}",
        outcomes[2]
    );

    cleanup(&pool, &tid).await;
}

// ── 3. Deterministic-key dedupe ───────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn bulk_resolve_idempotent_retry_returns_already_resolved() {
    let pool = setup_db().await;
    let tid = tenant();
    let conflict = create_conflict(&pool, &edit_conflict(&tid, "invoice"))
        .await
        .unwrap();

    // First call
    bulk_resolve_conflicts(&pool, &tid, "op", vec![resolve_item(conflict.id, 5)])
        .await
        .unwrap();

    // Same item submitted again — same (conflict_id, action, authority_version) → same det_key
    let outcomes = bulk_resolve_conflicts(&pool, &tid, "op", vec![resolve_item(conflict.id, 5)])
        .await
        .unwrap();

    assert!(
        matches!(&outcomes[0], BulkResolveOutcome::AlreadyResolved { .. }),
        "expected AlreadyResolved on retry, got {:?}",
        outcomes[0]
    );

    cleanup(&pool, &tid).await;
}

#[tokio::test]
#[serial]
async fn bulk_resolve_idempotent_retry_returns_already_ignored() {
    let pool = setup_db().await;
    let tid = tenant();
    let conflict = create_conflict(&pool, &edit_conflict(&tid, "customer"))
        .await
        .unwrap();

    bulk_resolve_conflicts(&pool, &tid, "op", vec![ignore_item(conflict.id, 2)])
        .await
        .unwrap();

    let outcomes = bulk_resolve_conflicts(&pool, &tid, "op", vec![ignore_item(conflict.id, 2)])
        .await
        .unwrap();

    assert!(
        matches!(&outcomes[0], BulkResolveOutcome::AlreadyIgnored { .. }),
        "{:?}",
        outcomes[0]
    );

    cleanup(&pool, &tid).await;
}

#[tokio::test]
#[serial]
async fn bulk_resolve_idempotent_retry_preserves_det_key() {
    let pool = setup_db().await;
    let tid = tenant();
    let conflict = create_conflict(&pool, &edit_conflict(&tid, "payment"))
        .await
        .unwrap();

    bulk_resolve_conflicts(&pool, &tid, "op", vec![resolve_item(conflict.id, 7)])
        .await
        .unwrap();

    let outcomes = bulk_resolve_conflicts(&pool, &tid, "op", vec![resolve_item(conflict.id, 7)])
        .await
        .unwrap();

    let expected_key = compute_resolve_det_key(conflict.id, "resolve", 7);
    assert!(
        matches!(&outcomes[0], BulkResolveOutcome::AlreadyResolved { deterministic_key, .. }
            if *deterministic_key == expected_key),
        "{:?}",
        outcomes[0]
    );

    cleanup(&pool, &tid).await;
}

// ── 4. Caller-key aliasing ────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn bulk_resolve_caller_key_is_alias_only() {
    let pool = setup_db().await;
    let tid = tenant();
    let c = create_conflict(&pool, &edit_conflict(&tid, "invoice"))
        .await
        .unwrap();

    let mut item = resolve_item(c.id, 1);
    item.caller_idempotency_key = Some("caller-provided-key-abc".to_string());

    let outcomes = bulk_resolve_conflicts(&pool, &tid, "op", vec![item])
        .await
        .unwrap();

    // Outcome must include the server deterministic key, not the caller key
    let expected_det = compute_resolve_det_key(c.id, "resolve", 1);
    match &outcomes[0] {
        BulkResolveOutcome::Resolved {
            deterministic_key,
            caller_idempotency_key,
            ..
        } => {
            assert_eq!(
                deterministic_key, &expected_det,
                "server det key must be sha256-based"
            );
            assert_eq!(
                caller_idempotency_key.as_deref(),
                Some("caller-provided-key-abc"),
                "caller key must be echoed back"
            );
        }
        other => panic!("expected Resolved, got {:?}", other),
    }

    // Retry with a different caller key but SAME (conflict_id, action, authority_version)
    // → same det_key → AlreadyResolved (server key drives dedupe, not caller key)
    let mut retry = resolve_item(c.id, 1);
    retry.caller_idempotency_key = Some("completely-different-caller-key".to_string());

    let retry_outcomes = bulk_resolve_conflicts(&pool, &tid, "op", vec![retry])
        .await
        .unwrap();

    assert!(
        matches!(
            &retry_outcomes[0],
            BulkResolveOutcome::AlreadyResolved { .. }
        ),
        "different caller key, same server key → must be idempotent: {:?}",
        retry_outcomes[0]
    );

    cleanup(&pool, &tid).await;
}

// ── 5. TerminalByOther ────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn bulk_resolve_terminal_by_other_when_authority_version_differs() {
    let pool = setup_db().await;
    let tid = tenant();
    let c = create_conflict(&pool, &edit_conflict(&tid, "customer"))
        .await
        .unwrap();

    // Resolve under authority_version=1 → stored det_key = sha256(id:resolve:1)
    bulk_resolve_conflicts(&pool, &tid, "op", vec![resolve_item(c.id, 1)])
        .await
        .unwrap();

    // Now retry with authority_version=2 → different det_key → TerminalByOther
    let outcomes = bulk_resolve_conflicts(&pool, &tid, "op", vec![resolve_item(c.id, 2)])
        .await
        .unwrap();

    assert!(
        matches!(&outcomes[0], BulkResolveOutcome::TerminalByOther { current_status, .. }
            if current_status == "resolved"),
        "{:?}",
        outcomes[0]
    );

    cleanup(&pool, &tid).await;
}

// ── 6. Capacity guard ─────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn bulk_resolve_exceeds_capacity_returns_error() {
    let pool = setup_db().await;
    let tid = tenant();

    let items: Vec<BulkResolveItem> = (0..=BULK_RESOLVE_CAP)
        .map(|i| BulkResolveItem {
            conflict_id: Uuid::new_v4(),
            action: "resolve".to_string(),
            authority_version: i as i64,
            internal_id: Some("x".to_string()),
            resolution_note: None,
            caller_idempotency_key: None,
        })
        .collect();

    let result = bulk_resolve_conflicts(&pool, &tid, "op", items).await;
    assert!(
        matches!(result, Err(BulkResolveError::ExceedsCapacity(_))),
        "expected ExceedsCapacity error"
    );
}

// ── 7. Per-item guards ────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn bulk_resolve_invalid_action_returns_per_item_error() {
    let pool = setup_db().await;
    let tid = tenant();
    let c = create_conflict(&pool, &edit_conflict(&tid, "invoice"))
        .await
        .unwrap();

    let outcomes = bulk_resolve_conflicts(
        &pool,
        &tid,
        "op",
        vec![BulkResolveItem {
            conflict_id: c.id,
            action: "delete".to_string(),
            authority_version: 1,
            internal_id: None,
            resolution_note: None,
            caller_idempotency_key: None,
        }],
    )
    .await
    .unwrap();

    assert!(
        matches!(&outcomes[0], BulkResolveOutcome::InvalidAction { action, .. } if action == "delete"),
        "{:?}",
        outcomes[0]
    );

    cleanup(&pool, &tid).await;
}

#[tokio::test]
#[serial]
async fn bulk_resolve_missing_internal_id_returns_per_item_error() {
    let pool = setup_db().await;
    let tid = tenant();
    let c = create_conflict(&pool, &edit_conflict(&tid, "invoice"))
        .await
        .unwrap();

    let outcomes = bulk_resolve_conflicts(
        &pool,
        &tid,
        "op",
        vec![BulkResolveItem {
            conflict_id: c.id,
            action: "resolve".to_string(),
            authority_version: 1,
            internal_id: None,
            resolution_note: None,
            caller_idempotency_key: None,
        }],
    )
    .await
    .unwrap();

    assert!(
        matches!(&outcomes[0], BulkResolveOutcome::MissingInternalId { conflict_id }
            if *conflict_id == c.id),
        "{:?}",
        outcomes[0]
    );

    cleanup(&pool, &tid).await;
}

#[tokio::test]
#[serial]
async fn bulk_resolve_not_found_returns_per_item_not_found() {
    let pool = setup_db().await;
    let tid = tenant();
    let phantom = Uuid::new_v4();

    let outcomes = bulk_resolve_conflicts(&pool, &tid, "op", vec![resolve_item(phantom, 1)])
        .await
        .unwrap();

    assert!(
        matches!(&outcomes[0], BulkResolveOutcome::NotFound { conflict_id } if *conflict_id == phantom),
        "{:?}",
        outcomes[0]
    );
}

// ── 8. Tenant isolation ───────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn bulk_resolve_cannot_resolve_other_tenants_conflict() {
    let pool = setup_db().await;
    let owner = tenant();
    let attacker = tenant();

    let conflict = create_conflict(&pool, &edit_conflict(&owner, "invoice"))
        .await
        .unwrap();

    // Attacker submits owner's conflict_id
    let outcomes = bulk_resolve_conflicts(
        &pool,
        &attacker,
        "attacker",
        vec![resolve_item(conflict.id, 1)],
    )
    .await
    .unwrap();

    assert!(
        matches!(&outcomes[0], BulkResolveOutcome::NotFound { .. }),
        "cross-tenant access must return NotFound: {:?}",
        outcomes[0]
    );

    // Verify owner's conflict is still pending
    let row = get_conflict(&pool, &owner, conflict.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.status, "pending",
        "owner's conflict must remain pending"
    );

    cleanup(&pool, &owner).await;
}

// ── 9. Outbox event ───────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn bulk_resolve_resolve_action_writes_outbox_event() {
    let pool = setup_db().await;
    let tid = tenant();
    let conflict = create_conflict(&pool, &edit_conflict(&tid, "invoice"))
        .await
        .unwrap();

    bulk_resolve_conflicts(&pool, &tid, "op", vec![resolve_item(conflict.id, 1)])
        .await
        .unwrap();

    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT event_type, aggregate_id
         FROM integrations_outbox
         WHERE aggregate_type = 'sync_conflict' AND aggregate_id = $1
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(conflict.id.to_string())
    .fetch_optional(&pool)
    .await
    .expect("outbox query");

    let (event_type, agg_id) = row.expect("outbox row must exist");
    assert_eq!(event_type, EVENT_TYPE_SYNC_CONFLICT_RESOLVED);
    assert_eq!(agg_id, conflict.id.to_string());

    cleanup(&pool, &tid).await;
}

#[tokio::test]
#[serial]
async fn bulk_resolve_ignore_action_does_not_write_outbox_event() {
    let pool = setup_db().await;
    let tid = tenant();
    let conflict = create_conflict(&pool, &edit_conflict(&tid, "invoice"))
        .await
        .unwrap();

    bulk_resolve_conflicts(&pool, &tid, "op", vec![ignore_item(conflict.id, 1)])
        .await
        .unwrap();

    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox
         WHERE aggregate_type = 'sync_conflict' AND aggregate_id = $1",
    )
    .bind(conflict.id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count query");

    assert_eq!(count.0, 0, "ignore action must not write outbox event");

    cleanup(&pool, &tid).await;
}

// ── 10. Explicit entity-type + conflict-class dispatch ────────────────────────

#[tokio::test]
#[serial]
async fn bulk_resolve_supports_all_entity_and_class_combinations() {
    let pool = setup_db().await;
    let tid = tenant();

    let pairs = [
        ("customer", ConflictClass::Edit),
        ("customer", ConflictClass::Creation),
        ("customer", ConflictClass::Deletion),
        ("invoice", ConflictClass::Edit),
        ("invoice", ConflictClass::Creation),
        ("invoice", ConflictClass::Deletion),
        ("payment", ConflictClass::Edit),
        ("payment", ConflictClass::Creation),
        ("payment", ConflictClass::Deletion),
    ];

    for (entity_type, conflict_class) in pairs {
        let req = CreateConflictRequest {
            app_id: tid.clone(),
            provider: "quickbooks".to_string(),
            entity_type: entity_type.to_string(),
            entity_id: format!("ent-{}", Uuid::new_v4().simple()),
            conflict_class: conflict_class.clone(),
            detected_by: "test".to_string(),
            internal_value: if matches!(conflict_class, ConflictClass::Deletion) {
                None
            } else {
                Some(serde_json::json!({"v": 1}))
            },
            external_value: if matches!(conflict_class, ConflictClass::Deletion) {
                None
            } else {
                Some(serde_json::json!({"v": 2}))
            },
        };
        let conflict = create_conflict(&pool, &req).await.unwrap();
        let outcomes =
            bulk_resolve_conflicts(&pool, &tid, "op", vec![resolve_item(conflict.id, 1)])
                .await
                .unwrap();

        assert!(
            matches!(&outcomes[0], BulkResolveOutcome::Resolved { .. }),
            "({}, {:?}) should be accepted: {:?}",
            entity_type,
            conflict_class,
            outcomes[0]
        );
    }

    cleanup(&pool, &tid).await;
}
