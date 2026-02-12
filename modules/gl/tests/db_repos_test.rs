use chrono::Utc;
use gl_rs::db::init_pool;
use gl_rs::repos::{failed_repo, journal_repo, processed_repo};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

async fn setup_test_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5438/gl_test".to_string());

    init_pool(&database_url)
        .await
        .expect("Failed to create test pool")
}

#[tokio::test]
#[serial]
async fn test_processed_events_idempotency() {
    let pool = setup_test_pool().await;
    let event_id = Uuid::new_v4();

    // First check: event should not exist
    let exists = processed_repo::exists(&pool, event_id)
        .await
        .expect("Failed to check event existence");
    assert!(!exists, "Event should not exist initially");

    // Insert the event
    let mut tx = pool.begin().await.expect("Failed to begin transaction");
    processed_repo::insert(&mut tx, event_id, "test.event.type", "test-processor")
        .await
        .expect("Failed to insert processed event");
    tx.commit().await.expect("Failed to commit transaction");

    // Second check: event should now exist
    let exists = processed_repo::exists(&pool, event_id)
        .await
        .expect("Failed to check event existence");
    assert!(exists, "Event should exist after insertion");

    // Cleanup
    sqlx::query("DELETE FROM processed_events WHERE event_id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .expect("Failed to cleanup");
}

#[tokio::test]
#[serial]
async fn test_journal_entry_with_lines() {
    let pool = setup_test_pool().await;
    let entry_id = Uuid::new_v4();
    let source_event_id = Uuid::new_v4();

    // Begin transaction
    let mut tx = pool.begin().await.expect("Failed to begin transaction");

    // Insert journal entry
    let returned_id = journal_repo::insert_entry(
        &mut tx,
        entry_id,
        "tenant-123",
        "ar",
        source_event_id,
        "ar.invoice.created",
        Utc::now(),
        "USD",
        Some("Test invoice journal entry"),
        Some("invoice"),
        Some("INV-001"),
    )
    .await
    .expect("Failed to insert journal entry");

    assert_eq!(returned_id, entry_id, "Returned entry_id should match input");

    // Insert journal lines
    let lines = vec![
        journal_repo::JournalLineInsert {
            id: Uuid::new_v4(),
            line_no: 1,
            account_ref: "1200".to_string(), // AR account
            debit_minor: 10000,              // $100.00 debit
            credit_minor: 0,
            memo: Some("Accounts Receivable".to_string()),
        },
        journal_repo::JournalLineInsert {
            id: Uuid::new_v4(),
            line_no: 2,
            account_ref: "4000".to_string(), // Revenue account
            debit_minor: 0,
            credit_minor: 10000,             // $100.00 credit
            memo: Some("Revenue".to_string()),
        },
    ];

    journal_repo::bulk_insert_lines(&mut tx, entry_id, lines)
        .await
        .expect("Failed to insert journal lines");

    tx.commit().await.expect("Failed to commit transaction");

    // Verify the journal entry exists
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries WHERE id = $1")
        .bind(entry_id)
        .fetch_one(&pool)
        .await
        .expect("Failed to query journal entries");

    assert_eq!(count, 1, "Journal entry should exist");

    // Verify the journal lines exist
    let line_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM journal_lines WHERE journal_entry_id = $1")
        .bind(entry_id)
        .fetch_one(&pool)
        .await
        .expect("Failed to query journal lines");

    assert_eq!(line_count, 2, "Should have 2 journal lines");

    // Cleanup
    sqlx::query("DELETE FROM journal_lines WHERE journal_entry_id = $1")
        .bind(entry_id)
        .execute(&pool)
        .await
        .expect("Failed to cleanup lines");

    sqlx::query("DELETE FROM journal_entries WHERE id = $1")
        .bind(entry_id)
        .execute(&pool)
        .await
        .expect("Failed to cleanup entry");
}

#[tokio::test]
#[serial]
async fn test_failed_event_insertion() {
    let pool = setup_test_pool().await;
    let event_id = Uuid::new_v4();

    let mut tx = pool.begin().await.expect("Failed to begin transaction");

    // Insert a failed event
    let envelope_json = serde_json::json!({
        "event_id": event_id.to_string(),
        "occurred_at": "2024-01-01T00:00:00Z",
        "payload": {"test": "data"}
    });

    failed_repo::insert(
        &mut tx,
        event_id,
        "gl.events.posting.requested",
        "tenant-456",
        envelope_json,
        "Test error: validation failed",
        3,
    )
    .await
    .expect("Failed to insert failed event");

    tx.commit().await.expect("Failed to commit transaction");

    // Verify the failed event exists
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM failed_events WHERE event_id = $1")
        .bind(event_id)
        .fetch_one(&pool)
        .await
        .expect("Failed to query failed events");

    assert_eq!(count, 1, "Failed event should exist");

    // Cleanup
    sqlx::query("DELETE FROM failed_events WHERE event_id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .expect("Failed to cleanup");
}

#[tokio::test]
#[serial]
async fn test_transaction_rollback() {
    let pool = setup_test_pool().await;
    let entry_id = Uuid::new_v4();
    let source_event_id = Uuid::new_v4();

    // Begin transaction
    let mut tx = pool.begin().await.expect("Failed to begin transaction");

    // Insert journal entry
    journal_repo::insert_entry(
        &mut tx,
        entry_id,
        "tenant-789",
        "ar",
        source_event_id,
        "ar.invoice.created",
        Utc::now(),
        "USD",
        Some("Test rollback"),
        None,
        None,
    )
    .await
    .expect("Failed to insert journal entry");

    // Rollback instead of commit
    tx.rollback().await.expect("Failed to rollback transaction");

    // Verify the journal entry does NOT exist
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries WHERE id = $1")
        .bind(entry_id)
        .fetch_one(&pool)
        .await
        .expect("Failed to query journal entries");

    assert_eq!(count, 0, "Journal entry should not exist after rollback");
}
