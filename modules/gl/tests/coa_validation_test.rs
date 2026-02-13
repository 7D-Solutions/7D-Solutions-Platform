use chrono::Utc;
use gl_rs::contracts::gl_posting_request_v1::{GlPostingRequestV1, JournalLine, SourceDocType};
use gl_rs::db::init_pool;
use gl_rs::repos::account_repo::{AccountType, NormalBalance};
use gl_rs::services::journal_service::process_gl_posting_request;
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

/// Helper to insert a test account
async fn insert_test_account(
    pool: &PgPool,
    tenant_id: &str,
    code: &str,
    name: &str,
    account_type: AccountType,
    normal_balance: NormalBalance,
    is_active: bool,
) -> Uuid {
    let id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(code)
    .bind(name)
    .bind(account_type)
    .bind(normal_balance)
    .bind(is_active)
    .bind(Utc::now())
    .execute(pool)
    .await
    .expect("Failed to insert test account");

    id
}

/// Helper to cleanup test data
async fn cleanup_test_data(pool: &PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM journal_lines WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup journal lines");

    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup journal entries");

    sqlx::query("DELETE FROM processed_events WHERE processor = 'test-processor'")
        .execute(pool)
        .await
        .expect("Failed to cleanup processed events");

    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("Failed to cleanup accounts");
}

#[tokio::test]
#[serial]
async fn test_posting_succeeds_with_valid_accounts() {
    let pool = setup_test_pool().await;
    let tenant_id = "tenant-coa-001";

    // Setup: Create active accounts
    insert_test_account(
        &pool,
        tenant_id,
        "1200",
        "Accounts Receivable",
        AccountType::Asset,
        NormalBalance::Debit,
        true,
    )
    .await;

    insert_test_account(
        &pool,
        tenant_id,
        "4000",
        "Revenue",
        AccountType::Revenue,
        NormalBalance::Credit,
        true,
    )
    .await;

    // Create posting request
    let payload = GlPostingRequestV1 {
        posting_date: "2024-02-11".to_string(),
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: "inv_coa_001".to_string(),
        description: "Test invoice with valid accounts".to_string(),
        lines: vec![
            JournalLine {
                account_ref: "1200".to_string(),
                debit: 100.0,
                credit: 0.0,
                memo: Some("AR".to_string()),
                dimensions: None,
            },
            JournalLine {
                account_ref: "4000".to_string(),
                debit: 0.0,
                credit: 100.0,
                memo: Some("Revenue".to_string()),
                dimensions: None,
            },
        ],
    };

    let event_id = Uuid::new_v4();

    // Process the posting request
    let result = process_gl_posting_request(
        &pool,
        event_id,
        tenant_id,
        "ar",
        "ar.invoice.created",
        &payload,
    )
    .await;

    // Should succeed
    assert!(result.is_ok(), "Posting should succeed with valid accounts");

    let entry_id = result.unwrap();

    // Verify journal entry was created
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries WHERE id = $1")
        .bind(entry_id)
        .fetch_one(&pool)
        .await
        .expect("Failed to query journal entries");

    assert_eq!(count, 1, "Journal entry should be created");

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_posting_fails_account_not_found() {
    let pool = setup_test_pool().await;
    let tenant_id = "tenant-coa-002";

    // Setup: Create only one account (missing the second one)
    insert_test_account(
        &pool,
        tenant_id,
        "1200",
        "Accounts Receivable",
        AccountType::Asset,
        NormalBalance::Debit,
        true,
    )
    .await;

    // Create posting request with a non-existent account
    let payload = GlPostingRequestV1 {
        posting_date: "2024-02-11".to_string(),
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: "inv_coa_002".to_string(),
        description: "Test invoice with missing account".to_string(),
        lines: vec![
            JournalLine {
                account_ref: "1200".to_string(),
                debit: 100.0,
                credit: 0.0,
                memo: Some("AR".to_string()),
                dimensions: None,
            },
            JournalLine {
                account_ref: "9999".to_string(), // Non-existent account
                debit: 0.0,
                credit: 100.0,
                memo: Some("Missing Account".to_string()),
                dimensions: None,
            },
        ],
    };

    let event_id = Uuid::new_v4();

    // Process the posting request
    let result = process_gl_posting_request(
        &pool,
        event_id,
        tenant_id,
        "ar",
        "ar.invoice.created",
        &payload,
    )
    .await;

    // Should fail with validation error
    assert!(result.is_err(), "Posting should fail with non-existent account");

    let error = result.unwrap_err();
    let error_msg = error.to_string();

    // Verify error message mentions the account
    assert!(
        error_msg.contains("9999") || error_msg.contains("not found"),
        "Error should mention the missing account: {}",
        error_msg
    );

    // Verify no journal entry was created (transaction rolled back)
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND reference_id = $2",
    )
    .bind(tenant_id)
    .bind("inv_coa_002")
    .fetch_one(&pool)
    .await
    .expect("Failed to query journal entries");

    assert_eq!(count, 0, "No journal entry should be created for failed posting");

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_posting_fails_account_inactive() {
    let pool = setup_test_pool().await;
    let tenant_id = "tenant-coa-003";

    // Setup: Create accounts, one inactive
    insert_test_account(
        &pool,
        tenant_id,
        "1200",
        "Accounts Receivable",
        AccountType::Asset,
        NormalBalance::Debit,
        true, // Active
    )
    .await;

    insert_test_account(
        &pool,
        tenant_id,
        "4000",
        "Inactive Revenue",
        AccountType::Revenue,
        NormalBalance::Credit,
        false, // Inactive
    )
    .await;

    // Create posting request with an inactive account
    let payload = GlPostingRequestV1 {
        posting_date: "2024-02-11".to_string(),
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: "inv_coa_003".to_string(),
        description: "Test invoice with inactive account".to_string(),
        lines: vec![
            JournalLine {
                account_ref: "1200".to_string(),
                debit: 100.0,
                credit: 0.0,
                memo: Some("AR".to_string()),
                dimensions: None,
            },
            JournalLine {
                account_ref: "4000".to_string(), // Inactive account
                debit: 0.0,
                credit: 100.0,
                memo: Some("Inactive Revenue".to_string()),
                dimensions: None,
            },
        ],
    };

    let event_id = Uuid::new_v4();

    // Process the posting request
    let result = process_gl_posting_request(
        &pool,
        event_id,
        tenant_id,
        "ar",
        "ar.invoice.created",
        &payload,
    )
    .await;

    // Should fail with validation error
    assert!(result.is_err(), "Posting should fail with inactive account");

    let error = result.unwrap_err();
    let error_msg = error.to_string();

    // Verify error message mentions inactive or the account
    assert!(
        error_msg.contains("4000") || error_msg.contains("inactive"),
        "Error should mention the inactive account: {}",
        error_msg
    );

    // Verify no journal entry was created (transaction rolled back)
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND reference_id = $2",
    )
    .bind(tenant_id)
    .bind("inv_coa_003")
    .fetch_one(&pool)
    .await
    .expect("Failed to query journal entries");

    assert_eq!(count, 0, "No journal entry should be created for failed posting");

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_posting_validates_all_lines() {
    let pool = setup_test_pool().await;
    let tenant_id = "tenant-coa-004";

    // Setup: Create only one account
    insert_test_account(
        &pool,
        tenant_id,
        "1200",
        "Accounts Receivable",
        AccountType::Asset,
        NormalBalance::Debit,
        true,
    )
    .await;

    // Create posting request with multiple missing accounts
    let payload = GlPostingRequestV1 {
        posting_date: "2024-02-11".to_string(),
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: "inv_coa_004".to_string(),
        description: "Test invoice with multiple lines".to_string(),
        lines: vec![
            JournalLine {
                account_ref: "1200".to_string(), // Valid
                debit: 150.0,
                credit: 0.0,
                memo: Some("AR".to_string()),
                dimensions: None,
            },
            JournalLine {
                account_ref: "8888".to_string(), // Missing - should fail on this line
                debit: 0.0,
                credit: 100.0,
                memo: Some("Missing 1".to_string()),
                dimensions: None,
            },
            JournalLine {
                account_ref: "9999".to_string(), // Missing - won't reach here
                debit: 0.0,
                credit: 50.0,
                memo: Some("Missing 2".to_string()),
                dimensions: None,
            },
        ],
    };

    let event_id = Uuid::new_v4();

    // Process the posting request
    let result = process_gl_posting_request(
        &pool,
        event_id,
        tenant_id,
        "ar",
        "ar.invoice.created",
        &payload,
    )
    .await;

    // Should fail on the first invalid account (line 1, which is index 1)
    assert!(result.is_err(), "Posting should fail validation");

    let error = result.unwrap_err();
    let error_msg = error.to_string();

    // Should fail on account 8888 (line index 1)
    assert!(
        error_msg.contains("8888") || error_msg.contains("Line 1"),
        "Error should reference the first invalid account: {}",
        error_msg
    );

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}

#[tokio::test]
#[serial]
async fn test_posting_preserves_idempotency_with_coa_validation() {
    let pool = setup_test_pool().await;
    let tenant_id = "tenant-coa-005";

    // Setup: Create active accounts
    insert_test_account(
        &pool,
        tenant_id,
        "1200",
        "Accounts Receivable",
        AccountType::Asset,
        NormalBalance::Debit,
        true,
    )
    .await;

    insert_test_account(
        &pool,
        tenant_id,
        "4000",
        "Revenue",
        AccountType::Revenue,
        NormalBalance::Credit,
        true,
    )
    .await;

    // Create posting request
    let payload = GlPostingRequestV1 {
        posting_date: "2024-02-11".to_string(),
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: "inv_coa_005".to_string(),
        description: "Test idempotency".to_string(),
        lines: vec![
            JournalLine {
                account_ref: "1200".to_string(),
                debit: 100.0,
                credit: 0.0,
                memo: Some("AR".to_string()),
                dimensions: None,
            },
            JournalLine {
                account_ref: "4000".to_string(),
                debit: 0.0,
                credit: 100.0,
                memo: Some("Revenue".to_string()),
                dimensions: None,
            },
        ],
    };

    let event_id = Uuid::new_v4();

    // First posting should succeed
    let result1 = process_gl_posting_request(
        &pool,
        event_id,
        tenant_id,
        "ar",
        "ar.invoice.created",
        &payload,
    )
    .await;

    assert!(result1.is_ok(), "First posting should succeed");

    // Second posting with same event_id should be idempotent (duplicate event)
    let result2 = process_gl_posting_request(
        &pool,
        event_id,
        tenant_id,
        "ar",
        "ar.invoice.created",
        &payload,
    )
    .await;

    assert!(result2.is_err(), "Second posting should be rejected as duplicate");

    let error = result2.unwrap_err();
    assert!(
        error.to_string().contains("duplicate") || error.to_string().contains("already processed"),
        "Error should indicate duplicate event: {}",
        error
    );

    // Verify only one journal entry was created
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND reference_id = $2",
    )
    .bind(tenant_id)
    .bind("inv_coa_005")
    .fetch_one(&pool)
    .await
    .expect("Failed to query journal entries");

    assert_eq!(count, 1, "Only one journal entry should exist");

    // Cleanup
    cleanup_test_data(&pool, tenant_id).await;
}
