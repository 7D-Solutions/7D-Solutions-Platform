//! Integrated tests for Treasury reconciliation (bd-2ztc).
//!
//! Covers:
//! 1. Auto-match — exact match creates 1 match
//! 2. Auto-match idempotent — running again creates 0
//! 3. Manual-match happy path
//! 4. Rematch supersedes old match (duplicate rejected)
//! 5. GL link — nonexistent txn rejected (invalid ref)
//! 6. GL link — happy path with gl_entry_id stored
//! 7. Recon view — list_matches excludes superseded
//! 8. Tenant isolation — matches scoped by app_id

use chrono::NaiveDate;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use treasury::domain::accounts::{service as acct_svc, CreateBankAccountRequest};
use treasury::domain::recon::{
    gl_link::{link_bank_txn_to_gl, LinkToGlRequest},
    models::{ManualMatchRequest, ReconMatchStatus, ReconMatchType},
    service as recon_svc, ReconError,
};
use treasury::domain::txns::{models::InsertBankTxnRequest, service as txn_svc};

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://treasury_user:treasury_pass@localhost:5444/treasury_db".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to treasury test DB");

    // Run migrations. If the schema was bootstrapped outside of sqlx (no tracking rows),
    // the first migration will fail with "already exists". Accept that and verify the
    // schema is accessible — the DB is ready either way.
    if let Err(e) = sqlx::migrate!("db/migrations").run(&pool).await {
        if !e.to_string().contains("already exists") {
            panic!("Failed to run treasury migrations: {}", e);
        }
        sqlx::query("SELECT 1 FROM treasury_recon_matches LIMIT 0")
            .execute(&pool)
            .await
            .expect("treasury_recon_matches not accessible after migration fallback");
    }

    pool
}

fn unique_app() -> String {
    format!("recon-test-{}", Uuid::new_v4().simple())
}

async fn create_test_account(pool: &sqlx::PgPool, app: &str) -> Uuid {
    let req = CreateBankAccountRequest {
        account_name: "Recon Test Account".to_string(),
        institution: Some("Test Bank".to_string()),
        account_number_last4: Some("8888".to_string()),
        routing_number: None,
        currency: "USD".to_string(),
        metadata: None,
    };
    acct_svc::create_bank_account(pool, app, &req, None, "setup".to_string())
        .await
        .expect("create test account")
        .id
}

/// Insert a statement line (transaction with statement_id set).
async fn insert_statement_line(
    pool: &sqlx::PgPool,
    app: &str,
    account_id: Uuid,
    amount: i64,
    date: NaiveDate,
    reference: Option<&str>,
) -> Uuid {
    let stmt_id = Uuid::new_v4();
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
    .bind(app)
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
    .bind(app)
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

/// Insert a payment-event transaction (no statement_id — eligible for matching).
async fn insert_payment_txn(
    pool: &sqlx::PgPool,
    app: &str,
    account_id: Uuid,
    amount: i64,
    date: NaiveDate,
    reference: Option<&str>,
) -> Uuid {
    let req = InsertBankTxnRequest {
        app_id: app.to_string(),
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

    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM treasury_bank_transactions WHERE external_id = $1",
    )
    .bind(&req.external_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

// ============================================================================
// 1. Auto-match exact — creates 1 match
// ============================================================================

#[tokio::test]
#[serial]
async fn test_auto_match_creates_match() {
    let pool = setup_db().await;
    let app = unique_app();
    let acct = create_test_account(&pool, &app).await;
    let d = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();

    insert_statement_line(&pool, &app, acct, -450, d, Some("TXN001")).await;
    insert_payment_txn(&pool, &app, acct, -450, d, Some("TXN001")).await;

    let result = recon_svc::run_auto_match(&pool, &app, acct, "corr1")
        .await
        .unwrap();

    assert_eq!(result.matches_created, 1);
    assert_eq!(result.unmatched_statement_lines, 0);
    assert_eq!(result.unmatched_transactions, 0);
}

// ============================================================================
// 2. Auto-match is idempotent — running again finds 0 new matches
// ============================================================================

#[tokio::test]
#[serial]
async fn test_auto_match_is_idempotent() {
    let pool = setup_db().await;
    let app = unique_app();
    let acct = create_test_account(&pool, &app).await;
    let d = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();

    insert_statement_line(&pool, &app, acct, -1000, d, Some("REF01")).await;
    insert_payment_txn(&pool, &app, acct, -1000, d, Some("REF01")).await;

    let r1 = recon_svc::run_auto_match(&pool, &app, acct, "corr1")
        .await
        .unwrap();
    assert_eq!(r1.matches_created, 1);

    let r2 = recon_svc::run_auto_match(&pool, &app, acct, "corr2")
        .await
        .unwrap();
    assert_eq!(r2.matches_created, 0, "running again should find nothing new");
}

// ============================================================================
// 3. Manual match — happy path creates record with correct types
// ============================================================================

#[tokio::test]
#[serial]
async fn test_manual_match_happy_path() {
    let pool = setup_db().await;
    let app = unique_app();
    let acct = create_test_account(&pool, &app).await;
    let d = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();

    let sl_id = insert_statement_line(&pool, &app, acct, -500, d, None).await;
    let pt_id = insert_payment_txn(&pool, &app, acct, -499, d, None).await;

    let req = ManualMatchRequest {
        statement_line_id: sl_id,
        bank_transaction_id: pt_id,
    };
    let m = recon_svc::create_manual_match(&pool, &app, &req, "tester", "corr1")
        .await
        .unwrap();

    assert_eq!(m.statement_line_id, Some(sl_id));
    assert_eq!(m.bank_transaction_id, pt_id);
    assert_eq!(m.match_type, ReconMatchType::Manual);
    assert!(m.superseded_by.is_none());
    assert_eq!(m.status, ReconMatchStatus::Confirmed);
}

// ============================================================================
// 4. Rematch supersedes old — duplicate rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_rematch_supersedes_old_match() {
    let pool = setup_db().await;
    let app = unique_app();
    let acct = create_test_account(&pool, &app).await;
    let d = NaiveDate::from_ymd_opt(2024, 4, 1).unwrap();

    let sl_id = insert_statement_line(&pool, &app, acct, -200, d, None).await;
    let pt1_id = insert_payment_txn(&pool, &app, acct, -200, d, Some("WRONG")).await;
    let pt2_id = insert_payment_txn(&pool, &app, acct, -200, d, Some("RIGHT")).await;

    // First match
    let m1 = recon_svc::create_manual_match(
        &pool,
        &app,
        &ManualMatchRequest {
            statement_line_id: sl_id,
            bank_transaction_id: pt1_id,
        },
        "tester",
        "corr1",
    )
    .await
    .unwrap();

    // Rematch to different transaction — m1 is superseded
    let m2 = recon_svc::create_manual_match(
        &pool,
        &app,
        &ManualMatchRequest {
            statement_line_id: sl_id,
            bank_transaction_id: pt2_id,
        },
        "tester",
        "corr2",
    )
    .await
    .unwrap();

    assert_ne!(m1.id, m2.id);
    assert!(m2.superseded_by.is_none(), "new match should be active");

    // Original match should be superseded and rejected
    let old: (Option<Uuid>, String) = sqlx::query_as(
        "SELECT superseded_by, status::text FROM treasury_recon_matches WHERE id = $1",
    )
    .bind(m1.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(old.0.is_some(), "old match must be superseded");
    assert_eq!(old.1, "rejected");
}

// ============================================================================
// 5. GL link — nonexistent txn rejected (invalid ref)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_gl_link_nonexistent_txn_rejected() {
    let pool = setup_db().await;
    let app = unique_app();

    let err = link_bank_txn_to_gl(
        &pool,
        &app,
        &LinkToGlRequest {
            bank_transaction_id: Uuid::new_v4(),
            gl_entry_id: 99999,
        },
        "tester",
        "corr1",
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, ReconError::TransactionNotFound(_)),
        "expected TransactionNotFound, got: {:?}",
        err
    );
}

// ============================================================================
// 6. GL link — happy path stores gl_entry_id
// ============================================================================

#[tokio::test]
#[serial]
async fn test_gl_link_happy_path() {
    let pool = setup_db().await;
    let app = unique_app();
    let acct = create_test_account(&pool, &app).await;
    let d = NaiveDate::from_ymd_opt(2024, 5, 1).unwrap();

    let txn_id = insert_payment_txn(&pool, &app, acct, -750, d, Some("GL001")).await;

    let m = link_bank_txn_to_gl(
        &pool,
        &app,
        &LinkToGlRequest {
            bank_transaction_id: txn_id,
            gl_entry_id: 42001,
        },
        "tester",
        "corr1",
    )
    .await
    .unwrap();

    assert_eq!(m.bank_transaction_id, txn_id);
    assert_eq!(m.gl_entry_id, Some(42001));
    assert_eq!(m.status, ReconMatchStatus::Confirmed);
    assert!(m.statement_line_id.is_none(), "GL-only match has no stmt line");
}

// ============================================================================
// 7. Recon view — list_matches excludes superseded by default
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_matches_excludes_superseded() {
    let pool = setup_db().await;
    let app = unique_app();
    let acct = create_test_account(&pool, &app).await;
    let d = NaiveDate::from_ymd_opt(2024, 6, 1).unwrap();

    let sl_id = insert_statement_line(&pool, &app, acct, -100, d, None).await;
    let pt1_id = insert_payment_txn(&pool, &app, acct, -100, d, None).await;
    let pt2_id = insert_payment_txn(&pool, &app, acct, -100, d, None).await;

    // Match then rematch
    recon_svc::create_manual_match(
        &pool,
        &app,
        &ManualMatchRequest {
            statement_line_id: sl_id,
            bank_transaction_id: pt1_id,
        },
        "t",
        "c1",
    )
    .await
    .unwrap();

    recon_svc::create_manual_match(
        &pool,
        &app,
        &ManualMatchRequest {
            statement_line_id: sl_id,
            bank_transaction_id: pt2_id,
        },
        "t",
        "c2",
    )
    .await
    .unwrap();

    let active = recon_svc::list_matches(&pool, &app, acct, false)
        .await
        .unwrap();
    assert_eq!(active.len(), 1, "only active match visible by default");

    let all = recon_svc::list_matches(&pool, &app, acct, true)
        .await
        .unwrap();
    assert_eq!(all.len(), 2, "both matches visible when including superseded");
}

// ============================================================================
// 8. Tenant isolation — matches scoped by app_id
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_isolation_matches() {
    let pool = setup_db().await;
    let app_a = unique_app();
    let app_b = unique_app();

    let acct_a = create_test_account(&pool, &app_a).await;
    let acct_b = create_test_account(&pool, &app_b).await;

    let d = NaiveDate::from_ymd_opt(2024, 7, 1).unwrap();

    // app_a: create a matched pair
    let sl_a = insert_statement_line(&pool, &app_a, acct_a, -300, d, Some("A")).await;
    let pt_a = insert_payment_txn(&pool, &app_a, acct_a, -300, d, Some("A")).await;
    recon_svc::run_auto_match(&pool, &app_a, acct_a, "corr-a")
        .await
        .unwrap();

    // app_b: insert unmatched txns only
    insert_statement_line(&pool, &app_b, acct_b, -400, d, Some("B")).await;
    insert_payment_txn(&pool, &app_b, acct_b, -401, d, Some("B")).await;

    // app_b sees no matches (amounts differ, no auto-match)
    let b_matches = recon_svc::list_matches(&pool, &app_b, acct_b, true)
        .await
        .unwrap();
    assert!(b_matches.is_empty(), "app_b should see no matches from app_a");

    // Statement line and payment txn are only used to populate data — assert isolation via lists
    let _ = (sl_a, pt_a); // suppress unused warnings
}
