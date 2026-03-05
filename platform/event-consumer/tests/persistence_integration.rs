//! Integration tests for event-consumer persistence layer (dedupe + DLQ).
//! Requires a real Postgres instance (default: audit_db on port 5440).

mod helpers;

use event_consumer::{
    DedupeError, DedupeOutcome, FailureKind, HandlerError,
    classify_handler_error, with_dedupe, write_dlq_entry,
};
use uuid::Uuid;

#[tokio::test]
async fn dedupe_executes_handler_once() {
    let pool = helpers::get_pool().await;
    helpers::run_migrations(&pool).await;

    let event_id = Uuid::new_v4();
    let subject = "test.dedupe.once";

    // First call — handler should execute.
    let outcome = with_dedupe(&pool, event_id, subject, || async { Ok(()) })
        .await
        .expect("with_dedupe failed");
    assert_eq!(outcome, DedupeOutcome::Executed);

    // Second call with same event_id — handler should be skipped.
    let outcome = with_dedupe(&pool, event_id, subject, || async {
        panic!("Handler should not be called for duplicate")
    })
    .await
    .expect("with_dedupe failed on duplicate");
    assert_eq!(outcome, DedupeOutcome::Duplicate);
}

#[tokio::test]
async fn dedupe_allows_retry_after_handler_failure() {
    let pool = helpers::get_pool().await;
    helpers::run_migrations(&pool).await;

    let event_id = Uuid::new_v4();
    let subject = "test.dedupe.retry";

    // First call — handler fails.
    let result = with_dedupe(&pool, event_id, subject, || async {
        Err(DedupeError::Handler("simulated failure".to_string()))
    })
    .await;
    assert!(result.is_err());

    // Retry — handler should execute again (dedupe row was removed).
    let outcome = with_dedupe(&pool, event_id, subject, || async { Ok(()) })
        .await
        .expect("retry should succeed after failure");
    assert_eq!(outcome, DedupeOutcome::Executed);
}

#[tokio::test]
async fn dedupe_different_event_ids_both_execute() {
    let pool = helpers::get_pool().await;
    helpers::run_migrations(&pool).await;

    let id_a = Uuid::new_v4();
    let id_b = Uuid::new_v4();

    let a = with_dedupe(&pool, id_a, "test.a", || async { Ok(()) })
        .await
        .unwrap();
    let b = with_dedupe(&pool, id_b, "test.b", || async { Ok(()) })
        .await
        .unwrap();

    assert_eq!(a, DedupeOutcome::Executed);
    assert_eq!(b, DedupeOutcome::Executed);
}

#[tokio::test]
async fn dlq_write_and_read_fatal() {
    let pool = helpers::get_pool().await;
    helpers::run_migrations(&pool).await;

    let event_id = Uuid::new_v4();
    let payload = serde_json::json!({"order_id": 42, "reason": "bad data"});

    write_dlq_entry(
        &pool,
        event_id,
        "orders.created",
        FailureKind::Fatal,
        "schema validation failed",
        &payload,
    )
    .await
    .expect("DLQ write failed");

    // Read it back.
    let entries = event_consumer::dlq::list_dlq_entries(&pool, Some(FailureKind::Fatal), 10)
        .await
        .expect("DLQ list failed");

    let entry = entries
        .iter()
        .find(|e| e.event_id == event_id)
        .expect("DLQ entry not found");

    assert_eq!(entry.failure_kind, FailureKind::Fatal);
    assert_eq!(entry.error_message, "schema validation failed");
    assert_eq!(entry.subject, "orders.created");
    assert_eq!(entry.payload["order_id"], 42);
}

#[tokio::test]
async fn dlq_write_poison() {
    let pool = helpers::get_pool().await;
    helpers::run_migrations(&pool).await;

    let event_id = Uuid::new_v4();

    write_dlq_entry(
        &pool,
        event_id,
        "unparseable.subject",
        FailureKind::Poison,
        "failed to deserialize envelope",
        &serde_json::json!(null),
    )
    .await
    .expect("DLQ write failed");

    let entries = event_consumer::dlq::list_dlq_entries(&pool, Some(FailureKind::Poison), 10)
        .await
        .unwrap();

    assert!(entries.iter().any(|e| e.event_id == event_id));
}

#[tokio::test]
async fn dlq_upsert_overwrites_on_conflict() {
    let pool = helpers::get_pool().await;
    helpers::run_migrations(&pool).await;

    let event_id = Uuid::new_v4();

    // First write as retryable.
    write_dlq_entry(
        &pool,
        event_id,
        "test.upsert",
        FailureKind::Retryable,
        "timeout",
        &serde_json::json!({}),
    )
    .await
    .unwrap();

    // Second write upgrades to fatal.
    write_dlq_entry(
        &pool,
        event_id,
        "test.upsert",
        FailureKind::Fatal,
        "permanently bad",
        &serde_json::json!({"upgraded": true}),
    )
    .await
    .unwrap();

    let entries = event_consumer::dlq::list_dlq_entries(&pool, None, 100)
        .await
        .unwrap();
    let entry = entries
        .iter()
        .find(|e| e.event_id == event_id)
        .unwrap();

    assert_eq!(entry.failure_kind, FailureKind::Fatal);
    assert_eq!(entry.error_message, "permanently bad");
}

#[tokio::test]
async fn classify_handler_error_maps_correctly() {
    assert_eq!(
        classify_handler_error(&HandlerError::Transient("timeout".into())),
        FailureKind::Retryable,
    );
    assert_eq!(
        classify_handler_error(&HandlerError::Permanent("bad schema".into())),
        FailureKind::Fatal,
    );
}
