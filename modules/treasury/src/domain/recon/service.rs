//! Reconciliation service — auto-match and manual-match with Guard→Mutation→Outbox.
//!
//! All matches are append-only. A rematch supersedes the prior active match
//! for that statement line (sets `superseded_by`) then inserts a new row.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::engine;
use super::models::*;
use super::ReconError;
use crate::outbox::enqueue_event_tx;

const EVT_RECON_AUTO_MATCHED: &str = "recon.auto_matched";
const EVT_RECON_MANUAL_MATCHED: &str = "recon.manual_matched";

// ============================================================================
// Auto-match
// ============================================================================

pub async fn run_auto_match(
    pool: &PgPool,
    app_id: &str,
    account_id: Uuid,
    correlation_id: &str,
) -> Result<AutoMatchResult, ReconError> {
    // Guard: fetch unmatched statement lines and payment txns for this account
    let stmt_lines = fetch_unmatched_statement_lines(pool, app_id, account_id).await?;
    let pay_txns = fetch_unmatched_payment_txns(pool, app_id, account_id).await?;

    // Engine: compute matches
    let candidates = engine::auto_match(&stmt_lines, &pay_txns);
    let matches_created = candidates.len();

    // Mutation + Outbox: insert each match within a transaction
    if !candidates.is_empty() {
        let mut tx = pool.begin().await?;

        for c in &candidates {
            insert_match_tx(
                &mut tx,
                app_id,
                c.statement_line.id,
                c.bank_transaction.id,
                ReconMatchType::Auto,
                Some(c.confidence),
                Some("auto-engine"),
            )
            .await?;

            // Mark both sides as matched
            mark_txn_matched(&mut tx, c.statement_line.id).await?;
            mark_txn_matched(&mut tx, c.bank_transaction.id).await?;
        }

        let event_id = Uuid::new_v4();
        let payload = serde_json::json!({
            "app_id": app_id,
            "account_id": account_id,
            "matches_created": matches_created,
            "correlation_id": correlation_id,
            "matched_at": Utc::now(),
        });
        enqueue_event_tx(
            &mut tx,
            event_id,
            EVT_RECON_AUTO_MATCHED,
            "recon",
            &account_id.to_string(),
            &payload,
        )
        .await?;

        tx.commit().await?;
    }

    let remaining_lines = stmt_lines.len() - matches_created;
    let remaining_txns = pay_txns.len() - matches_created;

    Ok(AutoMatchResult {
        matches_created,
        unmatched_statement_lines: remaining_lines,
        unmatched_transactions: remaining_txns,
    })
}

// ============================================================================
// Manual match
// ============================================================================

pub async fn create_manual_match(
    pool: &PgPool,
    app_id: &str,
    req: &ManualMatchRequest,
    actor: &str,
    correlation_id: &str,
) -> Result<ReconMatch, ReconError> {
    // Guard: verify both rows exist and belong to this app
    let sl = fetch_txn(pool, app_id, req.statement_line_id).await?;
    let bt = fetch_txn(pool, app_id, req.bank_transaction_id).await?;

    if sl.is_none() {
        return Err(ReconError::StatementLineNotFound(req.statement_line_id));
    }
    let sl = sl.unwrap();
    if bt.is_none() {
        return Err(ReconError::TransactionNotFound(req.bank_transaction_id));
    }
    let bt = bt.unwrap();

    // Guard: currency must match
    if sl.currency != bt.currency {
        return Err(ReconError::CurrencyMismatch {
            stmt_currency: sl.currency,
            txn_currency: bt.currency,
        });
    }

    // Mutation: supersede any existing active match for this statement line, insert new
    let mut tx = pool.begin().await?;

    supersede_active_match(&mut tx, req.statement_line_id).await?;

    let match_id = insert_match_tx(
        &mut tx,
        app_id,
        req.statement_line_id,
        req.bank_transaction_id,
        ReconMatchType::Manual,
        None,
        Some(actor),
    )
    .await?;

    mark_txn_matched(&mut tx, req.statement_line_id).await?;
    mark_txn_matched(&mut tx, req.bank_transaction_id).await?;

    // Outbox
    let event_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "app_id": app_id,
        "match_id": match_id,
        "statement_line_id": req.statement_line_id,
        "bank_transaction_id": req.bank_transaction_id,
        "actor": actor,
        "correlation_id": correlation_id,
        "matched_at": Utc::now(),
    });
    enqueue_event_tx(
        &mut tx,
        event_id,
        EVT_RECON_MANUAL_MATCHED,
        "recon",
        &match_id.to_string(),
        &payload,
    )
    .await?;

    tx.commit().await?;

    // Fetch the newly created match
    let m = fetch_match(pool, match_id)
        .await?
        .ok_or(ReconError::MatchNotFound(match_id))?;
    Ok(m)
}

// ============================================================================
// Queries
// ============================================================================

pub async fn list_matches(
    pool: &PgPool,
    app_id: &str,
    account_id: Uuid,
    include_superseded: bool,
) -> Result<Vec<ReconMatch>, ReconError> {
    let matches = if include_superseded {
        sqlx::query_as::<_, ReconMatch>(
            r#"
            SELECT m.* FROM treasury_recon_matches m
            JOIN treasury_bank_transactions t ON m.bank_transaction_id = t.id
            WHERE m.app_id = $1 AND t.account_id = $2
            ORDER BY m.created_at DESC
            "#,
        )
        .bind(app_id)
        .bind(account_id)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, ReconMatch>(
            r#"
            SELECT m.* FROM treasury_recon_matches m
            JOIN treasury_bank_transactions t ON m.bank_transaction_id = t.id
            WHERE m.app_id = $1 AND t.account_id = $2
              AND m.superseded_by IS NULL
            ORDER BY m.created_at DESC
            "#,
        )
        .bind(app_id)
        .bind(account_id)
        .fetch_all(pool)
        .await?
    };
    Ok(matches)
}

pub async fn list_unmatched(
    pool: &PgPool,
    app_id: &str,
    account_id: Uuid,
) -> Result<Vec<UnmatchedTxn>, ReconError> {
    let rows = sqlx::query_as::<_, UnmatchedTxn>(
        r#"
        SELECT id, account_id, transaction_date, amount_minor, currency,
               description, reference, statement_id
        FROM treasury_bank_transactions
        WHERE app_id = $1 AND account_id = $2 AND status = 'unmatched'
        ORDER BY transaction_date, id
        "#,
    )
    .bind(app_id)
    .bind(account_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

// ============================================================================
// Internal helpers
// ============================================================================

async fn fetch_unmatched_statement_lines(
    pool: &PgPool,
    app_id: &str,
    account_id: Uuid,
) -> Result<Vec<UnmatchedTxn>, sqlx::Error> {
    sqlx::query_as::<_, UnmatchedTxn>(
        r#"
        SELECT id, account_id, transaction_date, amount_minor, currency,
               description, reference, statement_id
        FROM treasury_bank_transactions
        WHERE app_id = $1 AND account_id = $2
          AND status = 'unmatched'
          AND statement_id IS NOT NULL
        ORDER BY transaction_date, id
        "#,
    )
    .bind(app_id)
    .bind(account_id)
    .fetch_all(pool)
    .await
}

async fn fetch_unmatched_payment_txns(
    pool: &PgPool,
    app_id: &str,
    account_id: Uuid,
) -> Result<Vec<UnmatchedTxn>, sqlx::Error> {
    sqlx::query_as::<_, UnmatchedTxn>(
        r#"
        SELECT id, account_id, transaction_date, amount_minor, currency,
               description, reference, statement_id
        FROM treasury_bank_transactions
        WHERE app_id = $1 AND account_id = $2
          AND status = 'unmatched'
          AND statement_id IS NULL
        ORDER BY transaction_date, id
        "#,
    )
    .bind(app_id)
    .bind(account_id)
    .fetch_all(pool)
    .await
}

async fn fetch_txn(
    pool: &PgPool,
    app_id: &str,
    txn_id: Uuid,
) -> Result<Option<UnmatchedTxn>, sqlx::Error> {
    sqlx::query_as::<_, UnmatchedTxn>(
        r#"
        SELECT id, account_id, transaction_date, amount_minor, currency,
               description, reference, statement_id
        FROM treasury_bank_transactions
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(txn_id)
    .bind(app_id)
    .fetch_optional(pool)
    .await
}

async fn fetch_match(pool: &PgPool, match_id: Uuid) -> Result<Option<ReconMatch>, sqlx::Error> {
    sqlx::query_as::<_, ReconMatch>(
        "SELECT * FROM treasury_recon_matches WHERE id = $1",
    )
    .bind(match_id)
    .fetch_optional(pool)
    .await
}

async fn insert_match_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    statement_line_id: Uuid,
    bank_transaction_id: Uuid,
    match_type: ReconMatchType,
    confidence: Option<rust_decimal::Decimal>,
    matched_by: Option<&str>,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    let now = Utc::now();

    sqlx::query(
        r#"
        INSERT INTO treasury_recon_matches
            (id, app_id, statement_line_id, bank_transaction_id, match_type,
             confidence_score, matched_by, status, matched_at, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, 'confirmed', $8, $8, $8)
        "#,
    )
    .bind(id)
    .bind(app_id)
    .bind(statement_line_id)
    .bind(bank_transaction_id)
    .bind(match_type)
    .bind(confidence)
    .bind(matched_by)
    .bind(now)
    .execute(&mut **tx)
    .await?;

    Ok(id)
}

async fn supersede_active_match(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    statement_line_id: Uuid,
) -> Result<(), sqlx::Error> {
    // Find current active match for this statement line (if any)
    let existing: Option<Uuid> = sqlx::query_scalar(
        r#"
        SELECT id FROM treasury_recon_matches
        WHERE statement_line_id = $1 AND superseded_by IS NULL
        "#,
    )
    .bind(statement_line_id)
    .fetch_optional(&mut **tx)
    .await?;

    if let Some(old_id) = existing {
        // We need a placeholder — insert the new match first, then update.
        // Instead, mark the old match with a sentinel and caller updates after insert.
        // Simpler: just mark old match status as rejected and set superseded_by to
        // a well-known sentinel; the caller's insert_match_tx happens right after.
        // Actually, the unique index is on (statement_line_id) WHERE superseded_by IS NULL.
        // We need to clear the old row out of the index BEFORE inserting the new one.
        // Set superseded_by to the old match's own ID as a temporary marker.
        sqlx::query(
            r#"
            UPDATE treasury_recon_matches
            SET superseded_by = id, status = 'rejected', updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(old_id)
        .execute(&mut **tx)
        .await?;

        // Also revert the old bank_transaction_id to unmatched
        let old_txn_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT bank_transaction_id FROM treasury_recon_matches WHERE id = $1",
        )
        .bind(old_id)
        .fetch_optional(&mut **tx)
        .await?;

        if let Some(txn_id) = old_txn_id {
            sqlx::query(
                "UPDATE treasury_bank_transactions SET status = 'unmatched', updated_at = NOW() WHERE id = $1",
            )
            .bind(txn_id)
            .execute(&mut **tx)
            .await?;
        }
    }

    Ok(())
}

async fn mark_txn_matched(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    txn_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE treasury_bank_transactions SET status = 'matched', updated_at = NOW() WHERE id = $1",
    )
    .bind(txn_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ============================================================================
// Integrated tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::{service as acct_svc, CreateBankAccountRequest};
    use crate::domain::txns::models::InsertBankTxnRequest;
    use crate::domain::txns::service as txn_svc;
    use chrono::NaiveDate;
    use serial_test::serial;

    const TEST_APP: &str = "test-app-recon";

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
        sqlx::query("DELETE FROM treasury_recon_matches WHERE app_id = $1")
            .bind(TEST_APP)
            .execute(pool)
            .await
            .ok();
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
            "DELETE FROM events_outbox WHERE aggregate_type IN ('recon', 'bank_account', 'bank_statement')",
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

    async fn create_test_account(pool: &PgPool) -> Uuid {
        let req = CreateBankAccountRequest {
            account_name: "Recon Test Account".to_string(),
            institution: Some("Test Bank".to_string()),
            account_number_last4: Some("8888".to_string()),
            routing_number: None,
            currency: "USD".to_string(),
            metadata: None,
        };
        acct_svc::create_bank_account(pool, TEST_APP, &req, None, "test".to_string())
            .await
            .expect("create test account")
            .id
    }

    /// Insert a statement line (has statement_id).
    async fn insert_statement_line(
        pool: &PgPool,
        account_id: Uuid,
        amount: i64,
        date: NaiveDate,
        reference: Option<&str>,
    ) -> Uuid {
        let stmt_id = Uuid::new_v4();
        // Create a statement row first
        sqlx::query(
            r#"INSERT INTO treasury_bank_statements
               (id, app_id, account_id, period_start, period_end,
                opening_balance_minor, closing_balance_minor, currency,
                status, imported_at, statement_hash, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $4, 0, 0, 'USD',
                       'imported'::treasury_statement_status, NOW(),
                       gen_random_uuid(), NOW(), NOW())
               ON CONFLICT DO NOTHING"#,
        )
        .bind(stmt_id)
        .bind(TEST_APP)
        .bind(account_id)
        .bind(date)
        .execute(pool)
        .await
        .unwrap();

        let txn_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO treasury_bank_transactions
               (id, app_id, account_id, statement_id, transaction_date,
                amount_minor, currency, description, reference, external_id)
               VALUES ($1, $2, $3, $4, $5, $6, 'USD', 'stmt line', $7, $8)"#,
        )
        .bind(txn_id)
        .bind(TEST_APP)
        .bind(account_id)
        .bind(stmt_id)
        .bind(date)
        .bind(amount)
        .bind(reference)
        .bind(format!("stmt:{}:line:0", stmt_id))
        .execute(pool)
        .await
        .unwrap();

        txn_id
    }

    /// Insert a payment-event transaction (no statement_id).
    async fn insert_payment_txn(
        pool: &PgPool,
        account_id: Uuid,
        amount: i64,
        date: NaiveDate,
        reference: Option<&str>,
    ) -> Uuid {
        let req = InsertBankTxnRequest {
            app_id: TEST_APP.to_string(),
            account_id,
            amount_minor: amount,
            currency: "USD".to_string(),
            transaction_date: date,
            description: Some("payment event".to_string()),
            reference: reference.map(String::from),
            external_id: format!("pay:{}", Uuid::new_v4()),
            auth_date: None,
            settle_date: None,
            merchant_name: None,
            merchant_category_code: None,
        };
        let mut tx = pool.begin().await.unwrap();
        txn_svc::insert_bank_txn_tx(&mut tx, &req).await.unwrap();
        tx.commit().await.unwrap();

        // Fetch the ID back
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM treasury_bank_transactions WHERE external_id = $1",
        )
        .bind(&req.external_id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    #[serial]
    async fn test_auto_match_exact() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let acct = create_test_account(&pool).await;
        let d = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();

        insert_statement_line(&pool, acct, -450, d, Some("TXN001")).await;
        insert_payment_txn(&pool, acct, -450, d, Some("TXN001")).await;

        let result = run_auto_match(&pool, TEST_APP, acct, "test-corr").await.unwrap();
        assert_eq!(result.matches_created, 1);
        assert_eq!(result.unmatched_statement_lines, 0);
        assert_eq!(result.unmatched_transactions, 0);

        // Running again should find nothing new (idempotent)
        let result2 = run_auto_match(&pool, TEST_APP, acct, "test-corr2").await.unwrap();
        assert_eq!(result2.matches_created, 0);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_manual_match_creates_record() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let acct = create_test_account(&pool).await;
        let d = NaiveDate::from_ymd_opt(2024, 2, 10).unwrap();

        let sl_id = insert_statement_line(&pool, acct, -1000, d, None).await;
        let pt_id = insert_payment_txn(&pool, acct, -999, d, None).await;

        let req = ManualMatchRequest {
            statement_line_id: sl_id,
            bank_transaction_id: pt_id,
        };
        let m = create_manual_match(&pool, TEST_APP, &req, "tester", "corr1")
            .await
            .unwrap();

        assert_eq!(m.statement_line_id, Some(sl_id));
        assert_eq!(m.bank_transaction_id, pt_id);
        assert_eq!(m.match_type, ReconMatchType::Manual);
        assert!(m.superseded_by.is_none());

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_manual_rematch_supersedes_old() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let acct = create_test_account(&pool).await;
        let d = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();

        let sl_id = insert_statement_line(&pool, acct, -500, d, None).await;
        let pt1_id = insert_payment_txn(&pool, acct, -500, d, Some("WRONG")).await;
        let pt2_id = insert_payment_txn(&pool, acct, -500, d, Some("RIGHT")).await;

        // First match
        let req1 = ManualMatchRequest {
            statement_line_id: sl_id,
            bank_transaction_id: pt1_id,
        };
        let m1 = create_manual_match(&pool, TEST_APP, &req1, "tester", "corr1")
            .await
            .unwrap();

        // Rematch to different transaction
        let req2 = ManualMatchRequest {
            statement_line_id: sl_id,
            bank_transaction_id: pt2_id,
        };
        let m2 = create_manual_match(&pool, TEST_APP, &req2, "tester", "corr2")
            .await
            .unwrap();

        assert_ne!(m1.id, m2.id);
        assert!(m2.superseded_by.is_none(), "new match is active");

        // Old match should be superseded
        let old = fetch_match(&pool, m1.id).await.unwrap().unwrap();
        assert!(old.superseded_by.is_some(), "old match must be superseded");
        assert_eq!(old.status, ReconMatchStatus::Rejected);

        // Old transaction should be unmatched again
        let old_txn_status: String = sqlx::query_scalar(
            "SELECT status::text FROM treasury_bank_transactions WHERE id = $1",
        )
        .bind(pt1_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(old_txn_status, "unmatched");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_list_matches_excludes_superseded() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let acct = create_test_account(&pool).await;
        let d = NaiveDate::from_ymd_opt(2024, 4, 1).unwrap();

        let sl_id = insert_statement_line(&pool, acct, -200, d, None).await;
        let pt1_id = insert_payment_txn(&pool, acct, -200, d, None).await;
        let pt2_id = insert_payment_txn(&pool, acct, -200, d, None).await;

        let req1 = ManualMatchRequest {
            statement_line_id: sl_id,
            bank_transaction_id: pt1_id,
        };
        create_manual_match(&pool, TEST_APP, &req1, "t", "c1").await.unwrap();

        let req2 = ManualMatchRequest {
            statement_line_id: sl_id,
            bank_transaction_id: pt2_id,
        };
        create_manual_match(&pool, TEST_APP, &req2, "t", "c2").await.unwrap();

        // Default: exclude superseded
        let active = list_matches(&pool, TEST_APP, acct, false).await.unwrap();
        assert_eq!(active.len(), 1);

        // Include superseded
        let all = list_matches(&pool, TEST_APP, acct, true).await.unwrap();
        assert_eq!(all.len(), 2);

        cleanup(&pool).await;
    }
}
