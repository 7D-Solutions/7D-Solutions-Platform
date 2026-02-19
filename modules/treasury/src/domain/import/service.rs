//! Statement import service — hashes CSV, creates statement + transaction lines.
//!
//! Idempotency is two-layer:
//! 1. `statement_hash` (UUID v5 of raw CSV bytes) on the statement row — re-import
//!    of the same file short-circuits with the existing statement ID.
//! 2. `external_id` on each transaction line — `ON CONFLICT DO NOTHING` prevents
//!    duplicate rows even if the hash check is somehow bypassed.

use chrono::{NaiveDate, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::adapters::CsvFormat;
use super::{parser, ImportError, ImportResult, LineError};
use crate::domain::accounts::AccountStatus;
use crate::outbox::enqueue_event_tx;

/// UUID v5 namespace for statement content hashing.
const STATEMENT_HASH_NS: Uuid = Uuid::from_bytes([
    0x7d, 0x50, 0x1a, 0x71, 0xba, 0x4c, 0x4e, 0x2a, 0x8f, 0x01, 0xc3, 0xee, 0xd4, 0xa1, 0xb7,
    0x09,
]);

const EVT_STATEMENT_IMPORTED: &str = "bank_statement.imported";

// ============================================================================
// Public request type
// ============================================================================

pub struct ImportRequest {
    pub account_id: Uuid,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub opening_balance_minor: i64,
    pub closing_balance_minor: i64,
    pub csv_data: Vec<u8>,
    pub filename: Option<String>,
    /// Optional CSV format hint. When `None`, the parser auto-detects
    /// from the CSV headers, falling back to the generic bank format.
    pub format: Option<CsvFormat>,
}

// ============================================================================
// Import entry point
// ============================================================================

pub async fn import_statement(
    pool: &PgPool,
    app_id: &str,
    req: ImportRequest,
    correlation_id: String,
) -> Result<ImportResult, ImportError> {
    // 1. Compute content hash
    let statement_hash = Uuid::new_v5(&STATEMENT_HASH_NS, &req.csv_data);

    // 2. Verify account exists and is active
    verify_account(pool, app_id, req.account_id).await?;

    // 3. Check for duplicate import (same file re-uploaded)
    if let Some(existing_id) = find_by_hash(pool, req.account_id, statement_hash).await? {
        return Err(ImportError::DuplicateImport {
            statement_id: existing_id,
        });
    }

    // 4. Parse CSV (auto-detects format if not specified)
    let parsed = parser::parse_csv_with_format(&req.csv_data, req.format);
    if parsed.lines.is_empty() {
        if parsed.errors.is_empty() {
            return Err(ImportError::EmptyImport);
        }
        return Err(ImportError::AllLinesFailed(parsed.errors));
    }

    // 5. Validate period
    if req.period_start > req.period_end {
        return Err(ImportError::Validation(
            "period_start must be <= period_end".to_string(),
        ));
    }

    // 6. Transactional: create statement + insert lines + emit event
    let statement_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let currency = fetch_account_currency(pool, req.account_id).await?;

    let mut tx = pool.begin().await?;

    // Insert statement header
    sqlx::query(
        r#"
        INSERT INTO treasury_bank_statements
            (id, app_id, account_id, period_start, period_end,
             opening_balance_minor, closing_balance_minor, currency,
             status, imported_at, source_filename, statement_hash,
             created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8,
                'imported'::treasury_statement_status, $9, $10, $11, $9, $9)
        "#,
    )
    .bind(statement_id)
    .bind(app_id)
    .bind(req.account_id)
    .bind(req.period_start)
    .bind(req.period_end)
    .bind(req.opening_balance_minor)
    .bind(req.closing_balance_minor)
    .bind(&currency)
    .bind(now)
    .bind(req.filename.as_deref())
    .bind(statement_hash)
    .execute(&mut *tx)
    .await?;

    // Insert transaction lines
    let mut imported = 0usize;
    let mut skipped = 0usize;
    let line_errors: Vec<LineError> = parsed.errors;

    for (idx, line) in parsed.lines.iter().enumerate() {
        // Deterministic external_id: hash(statement_hash + line_index)
        let ext_id = format!("stmt:{}:line:{}", statement_hash, idx);

        let result = sqlx::query(
            r#"
            INSERT INTO treasury_bank_transactions
                (app_id, account_id, statement_id, transaction_date,
                 amount_minor, currency, description, reference, external_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (account_id, external_id) DO NOTHING
            "#,
        )
        .bind(app_id)
        .bind(req.account_id)
        .bind(statement_id)
        .bind(line.date)
        .bind(line.amount_minor)
        .bind(&currency)
        .bind(&line.description)
        .bind(line.reference.as_deref())
        .bind(&ext_id)
        .execute(&mut *tx)
        .await?;

        if result.rows_affected() > 0 {
            imported += 1;
        } else {
            skipped += 1;
        }
    }

    // Emit outbox event
    let payload = serde_json::json!({
        "statement_id": statement_id,
        "account_id": req.account_id,
        "app_id": app_id,
        "period_start": req.period_start.to_string(),
        "period_end": req.period_end.to_string(),
        "lines_imported": imported,
        "statement_hash": statement_hash.to_string(),
        "correlation_id": correlation_id,
        "imported_at": now,
    });

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVT_STATEMENT_IMPORTED,
        "bank_statement",
        &statement_id.to_string(),
        &payload,
    )
    .await?;

    tx.commit().await?;

    Ok(ImportResult {
        statement_id,
        lines_imported: imported,
        lines_skipped: skipped,
        errors: line_errors,
    })
}

// ============================================================================
// Helpers
// ============================================================================

async fn verify_account(
    pool: &PgPool,
    app_id: &str,
    account_id: Uuid,
) -> Result<(), ImportError> {
    let row: Option<(AccountStatus,)> = sqlx::query_as(
        "SELECT status FROM treasury_bank_accounts WHERE id = $1 AND app_id = $2",
    )
    .bind(account_id)
    .bind(app_id)
    .fetch_optional(pool)
    .await?;

    match row {
        None => Err(ImportError::AccountNotFound(account_id)),
        Some((status,)) if status != AccountStatus::Active => Err(ImportError::AccountNotActive),
        Some(_) => Ok(()),
    }
}

async fn find_by_hash(
    pool: &PgPool,
    account_id: Uuid,
    hash: Uuid,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT id FROM treasury_bank_statements WHERE account_id = $1 AND statement_hash = $2",
    )
    .bind(account_id)
    .bind(hash)
    .fetch_optional(pool)
    .await
}

async fn fetch_account_currency(pool: &PgPool, account_id: Uuid) -> Result<String, sqlx::Error> {
    let currency: String =
        sqlx::query_scalar("SELECT currency FROM treasury_bank_accounts WHERE id = $1")
            .bind(account_id)
            .fetch_one(pool)
            .await?;
    Ok(currency)
}

// ============================================================================
// Integrated tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::{service as acct_svc, CreateBankAccountRequest};
    use serial_test::serial;

    const TEST_APP: &str = "test-app-import";

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://treasury_user:treasury_pass@localhost:5444/treasury_db".to_string()
        })
    }

    async fn test_pool() -> PgPool {
        sqlx::PgPool::connect(&test_db_url())
            .await
            .expect("Failed to connect to treasury test database")
    }

    async fn cleanup(pool: &PgPool) {
        sqlx::query("DELETE FROM treasury_bank_transactions WHERE app_id = $1")
            .bind(TEST_APP)
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM treasury_bank_statements WHERE app_id = $1")
            .bind(TEST_APP)
            .execute(pool)
            .await
            .ok();
        sqlx::query(
            "DELETE FROM events_outbox WHERE aggregate_type = 'bank_statement' \
             OR aggregate_type = 'bank_account'",
        )
        .execute(pool)
        .await
        .ok();
        sqlx::query("DELETE FROM treasury_idempotency_keys WHERE app_id = $1")
            .bind(TEST_APP)
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM treasury_bank_accounts WHERE app_id = $1")
            .bind(TEST_APP)
            .execute(pool)
            .await
            .ok();
    }

    fn sample_csv() -> Vec<u8> {
        b"date,description,amount,reference\n\
          2024-01-15,Coffee Shop,-4.50,TXN001\n\
          2024-01-16,Salary,5000.00,SAL001\n\
          2024-01-17,Groceries,-82.30,TXN002\n"
            .to_vec()
    }

    async fn create_test_account(pool: &PgPool) -> Uuid {
        let req = CreateBankAccountRequest {
            account_name: "Import Test Account".to_string(),
            institution: Some("Test Bank".to_string()),
            account_number_last4: Some("9999".to_string()),
            routing_number: None,
            currency: "USD".to_string(),
            metadata: None,
        };
        let account = acct_svc::create_bank_account(pool, TEST_APP, &req, None, "test".to_string())
            .await
            .expect("create test account");
        account.id
    }

    #[tokio::test]
    #[serial]
    async fn test_import_creates_statement_and_lines() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let account_id = create_test_account(&pool).await;

        let req = ImportRequest {
            account_id,
            period_start: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            period_end: NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
            opening_balance_minor: 100000,
            closing_balance_minor: 591320,
            csv_data: sample_csv(),
            filename: Some("jan-2024.csv".to_string()),
            format: None,
        };

        let result = import_statement(&pool, TEST_APP, req, "c1".to_string())
            .await
            .expect("import failed");

        assert_eq!(result.lines_imported, 3);
        assert_eq!(result.lines_skipped, 0);
        assert!(result.errors.is_empty());

        // Verify statement exists
        let stmt_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM treasury_bank_statements WHERE id = $1",
        )
        .bind(result.statement_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(stmt_count, 1);

        // Verify transaction lines
        let txn_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM treasury_bank_transactions WHERE statement_id = $1",
        )
        .bind(result.statement_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(txn_count, 3);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_reimport_same_file_is_idempotent() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let account_id = create_test_account(&pool).await;

        let csv = sample_csv();
        let req1 = ImportRequest {
            account_id,
            period_start: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            period_end: NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
            opening_balance_minor: 100000,
            closing_balance_minor: 591320,
            csv_data: csv.clone(),
            filename: Some("jan-2024.csv".to_string()),
            format: None,
        };

        let first = import_statement(&pool, TEST_APP, req1, "c1".to_string())
            .await
            .expect("first import failed");

        let req2 = ImportRequest {
            account_id,
            period_start: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            period_end: NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
            opening_balance_minor: 100000,
            closing_balance_minor: 591320,
            csv_data: csv,
            filename: Some("jan-2024.csv".to_string()),
            format: None,
        };

        let second = import_statement(&pool, TEST_APP, req2, "c2".to_string()).await;
        assert!(
            matches!(second, Err(ImportError::DuplicateImport { statement_id }) if statement_id == first.statement_id),
            "expected DuplicateImport, got {:?}",
            second
        );

        // Verify no extra lines were created
        let txn_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM treasury_bank_transactions WHERE app_id = $1",
        )
        .bind(TEST_APP)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(txn_count, 3);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_import_invalid_rows_reported() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let account_id = create_test_account(&pool).await;

        let csv = b"date,description,amount\n\
                     2024-01-15,Valid,-10.00\n\
                     bad-date,Invalid Date,20.00\n\
                     2024-01-17,,30.00\n\
                     2024-01-18,Also Valid,-5.00\n";

        let req = ImportRequest {
            account_id,
            period_start: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            period_end: NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
            opening_balance_minor: 0,
            closing_balance_minor: 0,
            csv_data: csv.to_vec(),
            filename: None,
            format: None,
        };

        let result = import_statement(&pool, TEST_APP, req, "c1".to_string())
            .await
            .expect("import failed");

        assert_eq!(result.lines_imported, 2);
        assert_eq!(result.errors.len(), 2);
        assert_eq!(result.errors[0].line, 3); // bad date
        assert_eq!(result.errors[1].line, 4); // empty description

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_import_nonexistent_account() {
        let pool = test_pool().await;
        let fake_id = Uuid::new_v4();

        let req = ImportRequest {
            account_id: fake_id,
            period_start: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            period_end: NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
            opening_balance_minor: 0,
            closing_balance_minor: 0,
            csv_data: sample_csv(),
            filename: None,
            format: None,
        };

        let result = import_statement(&pool, TEST_APP, req, "c1".to_string()).await;
        assert!(matches!(result, Err(ImportError::AccountNotFound(_))));
    }

    // ================================================================
    // CC adapter integration tests
    // ================================================================

    async fn create_test_cc_account(pool: &PgPool) -> Uuid {
        use crate::domain::accounts::{
            service as acct_svc, CreateCreditCardAccountRequest,
        };
        let req = CreateCreditCardAccountRequest {
            account_name: "CC Import Test".to_string(),
            institution: Some("Test Issuer".to_string()),
            account_number_last4: Some("5555".to_string()),
            currency: "USD".to_string(),
            credit_limit_minor: Some(500_000),
            statement_closing_day: Some(15),
            cc_network: Some("Visa".to_string()),
            metadata: None,
        };
        acct_svc::create_credit_card_account(pool, TEST_APP, &req, None, "test".to_string())
            .await
            .expect("create CC test account")
            .id
    }

    #[tokio::test]
    #[serial]
    async fn test_import_chase_cc_statement() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let account_id = create_test_cc_account(&pool).await;

        let csv = b"Transaction Date,Post Date,Description,Category,Type,Amount,Memo\n\
                     01/15/2024,01/16/2024,STARBUCKS,Food & Drink,Sale,-4.50,\n\
                     01/18/2024,01/19/2024,AMAZON.COM,Shopping,Sale,-89.99,\n\
                     01/20/2024,01/20/2024,PAYMENT,,Payment,250.00,\n";

        let req = ImportRequest {
            account_id,
            period_start: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            period_end: NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
            opening_balance_minor: 0,
            closing_balance_minor: 0,
            csv_data: csv.to_vec(),
            filename: Some("chase-jan-2024.csv".to_string()),
            format: Some(CsvFormat::ChaseCredit),
        };

        let result = import_statement(&pool, TEST_APP, req, "c1".to_string())
            .await
            .expect("Chase CC import failed");

        assert_eq!(result.lines_imported, 3);
        assert!(result.errors.is_empty());

        // Verify amounts stored correctly
        let amounts: Vec<(i64,)> = sqlx::query_as(
            "SELECT amount_minor FROM treasury_bank_transactions \
             WHERE statement_id = $1 ORDER BY transaction_date",
        )
        .bind(result.statement_id)
        .fetch_all(&pool)
        .await
        .unwrap();

        assert_eq!(amounts[0].0, -450);   // charge
        assert_eq!(amounts[1].0, -8999);  // charge
        assert_eq!(amounts[2].0, 25000);  // payment

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_import_amex_cc_statement() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let account_id = create_test_cc_account(&pool).await;

        // Amex: charges positive, credits negative
        let csv = b"Date,Description,Amount\n\
                     01/15/2024,STARBUCKS,4.50\n\
                     01/18/2024,AMAZON.COM,89.99\n\
                     01/20/2024,PAYMENT RECEIVED,-250.00\n";

        let req = ImportRequest {
            account_id,
            period_start: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            period_end: NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
            opening_balance_minor: 0,
            closing_balance_minor: 0,
            csv_data: csv.to_vec(),
            filename: Some("amex-jan-2024.csv".to_string()),
            format: Some(CsvFormat::AmexCredit),
        };

        let result = import_statement(&pool, TEST_APP, req, "c1".to_string())
            .await
            .expect("Amex CC import failed");

        assert_eq!(result.lines_imported, 3);
        assert!(result.errors.is_empty());

        // Verify sign normalisation: Amex positive charges → negative stored
        let amounts: Vec<(i64,)> = sqlx::query_as(
            "SELECT amount_minor FROM treasury_bank_transactions \
             WHERE statement_id = $1 ORDER BY transaction_date",
        )
        .bind(result.statement_id)
        .fetch_all(&pool)
        .await
        .unwrap();

        assert_eq!(amounts[0].0, -450);   // charge (was +4.50)
        assert_eq!(amounts[1].0, -8999);  // charge (was +89.99)
        assert_eq!(amounts[2].0, 25000);  // payment (was -250.00)

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_cc_reimport_idempotent() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let account_id = create_test_cc_account(&pool).await;

        let csv = b"Transaction Date,Post Date,Description,Category,Type,Amount\n\
                     01/15/2024,01/16/2024,STARBUCKS,Food,Sale,-4.50\n";

        let req1 = ImportRequest {
            account_id,
            period_start: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            period_end: NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
            opening_balance_minor: 0,
            closing_balance_minor: 0,
            csv_data: csv.to_vec(),
            filename: None,
            format: Some(CsvFormat::ChaseCredit),
        };

        let first = import_statement(&pool, TEST_APP, req1, "c1".to_string())
            .await
            .expect("first CC import failed");

        let req2 = ImportRequest {
            account_id,
            period_start: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            period_end: NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
            opening_balance_minor: 0,
            closing_balance_minor: 0,
            csv_data: csv.to_vec(),
            filename: None,
            format: Some(CsvFormat::ChaseCredit),
        };

        let second = import_statement(&pool, TEST_APP, req2, "c2".to_string()).await;
        assert!(
            matches!(second, Err(ImportError::DuplicateImport { statement_id }) if statement_id == first.statement_id),
            "expected DuplicateImport, got {:?}",
            second
        );

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_auto_detect_chase_format() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let account_id = create_test_cc_account(&pool).await;

        // No format specified — should auto-detect Chase from headers
        let csv = b"Transaction Date,Post Date,Description,Category,Type,Amount\n\
                     01/15/2024,01/16/2024,STARBUCKS,Food,Sale,-4.50\n";

        let req = ImportRequest {
            account_id,
            period_start: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            period_end: NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
            opening_balance_minor: 0,
            closing_balance_minor: 0,
            csv_data: csv.to_vec(),
            filename: None,
            format: None, // auto-detect
        };

        let result = import_statement(&pool, TEST_APP, req, "c1".to_string())
            .await
            .expect("auto-detect Chase import failed");

        assert_eq!(result.lines_imported, 1);
        assert!(result.errors.is_empty());

        cleanup(&pool).await;
    }
}
