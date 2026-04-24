//! Integration tests: item entity_type accepted in resolve allowlists (bd-dz8rz).
//!
//! Verifies:
//!  1.  item_single_resolve_accepted — resolve_conflict_transactional accepts entity_type='item'.
//!  2.  item_bulk_resolve_accepted   — bulk_resolve_conflicts accepts entity_type='item'.
//!
//! Run:
//!   ./scripts/cargo-slot.sh test -p integrations-rs --test item_resolve_conflict_test

use integrations_rs::domain::sync::conflicts::{ConflictClass, CreateConflictRequest};
use integrations_rs::domain::sync::conflicts_repo::create_conflict;
use integrations_rs::domain::sync::resolve_service::{
    bulk_resolve_conflicts, resolve_conflict_transactional, BulkResolveItem, BulkResolveOutcome,
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use uuid::Uuid;

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
        .expect("Failed to run integrations migrations");
    pool
}

fn tenant() -> String {
    format!("item-resolve-test-{}", Uuid::new_v4().simple())
}

fn item_conflict(app_id: &str) -> CreateConflictRequest {
    CreateConflictRequest {
        app_id: app_id.to_string(),
        provider: "quickbooks".to_string(),
        entity_type: "item".to_string(),
        entity_id: format!("item-{}", Uuid::new_v4().simple()),
        conflict_class: ConflictClass::Edit,
        detected_by: "detector".to_string(),
        internal_value: Some(serde_json::json!({"name": "Widget A", "price": 9.99})),
        external_value: Some(serde_json::json!({"name": "Widget A", "price": 10.49})),
    }
}

// ── 1. single-item resolve accepts entity_type='item' ────────────────────────

#[tokio::test]
#[serial]
async fn item_single_resolve_accepted() {
    let pool = setup_db().await;
    let tid = tenant();
    let conflict = create_conflict(&pool, &item_conflict(&tid))
        .await
        .expect("create item conflict");

    let result = resolve_conflict_transactional(
        &pool,
        &tid,
        conflict.id,
        "test-user",
        "ITEM-123",
        Some("test"),
    )
    .await;

    assert!(
        result.is_ok(),
        "entity_type 'item' must be accepted by resolve_conflict_transactional; got: {:?}",
        result.err()
    );
    assert_eq!(result.unwrap().status, "resolved");
}

// ── 2. bulk resolve accepts entity_type='item' ────────────────────────────────

#[tokio::test]
#[serial]
async fn item_bulk_resolve_accepted() {
    let pool = setup_db().await;
    let tid = tenant();
    let conflict = create_conflict(&pool, &item_conflict(&tid))
        .await
        .expect("create item conflict");

    let authority_version: i64 = 1;
    let item = BulkResolveItem {
        conflict_id: conflict.id,
        action: "resolve".to_string(),
        authority_version,
        internal_id: Some("ITEM-456".to_string()),
        resolution_note: Some("bulk test".to_string()),
        caller_idempotency_key: None,
    };

    let outcomes = bulk_resolve_conflicts(&pool, &tid, "test-user", vec![item])
        .await
        .expect("bulk_resolve_conflicts must not return ExceedsCapacity");

    assert_eq!(outcomes.len(), 1);
    match &outcomes[0] {
        BulkResolveOutcome::Resolved { conflict_id, .. } => {
            assert_eq!(*conflict_id, conflict.id, "resolved conflict_id must match");
        }
        other => panic!(
            "entity_type 'item' must produce Resolved outcome; got: {:?}",
            other
        ),
    }
}
