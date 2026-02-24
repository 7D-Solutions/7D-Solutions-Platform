//! Integrated tests for Treasury bank account CRUD (bd-2ztc).
//!
//! Covers:
//! 1. Create bank account — happy path
//! 2. Create CC account — happy path
//! 3. Validation error (empty name)
//! 4. List active-only (deactivated excluded)
//! 5. Update account fields
//! 6. Deactivate nonexistent account — error case
//! 7. Tenant isolation — account not visible to other app_id

use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use treasury::domain::accounts::{
    service as acct_svc, AccountError, CreateBankAccountRequest, CreateCreditCardAccountRequest,
    UpdateAccountRequest,
};

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
        sqlx::query("SELECT 1 FROM treasury_bank_accounts LIMIT 0")
            .execute(&pool)
            .await
            .expect("treasury_bank_accounts not accessible after migration fallback");
    }

    pool
}

fn unique_app() -> String {
    format!("acct-test-{}", Uuid::new_v4().simple())
}

fn sample_bank() -> CreateBankAccountRequest {
    CreateBankAccountRequest {
        account_name: "Main Checking".to_string(),
        institution: Some("Test Bank".to_string()),
        account_number_last4: Some("1234".to_string()),
        routing_number: None,
        currency: "USD".to_string(),
        metadata: None,
    }
}

fn sample_cc() -> CreateCreditCardAccountRequest {
    CreateCreditCardAccountRequest {
        account_name: "Corp Visa".to_string(),
        institution: Some("Chase".to_string()),
        account_number_last4: Some("9876".to_string()),
        currency: "USD".to_string(),
        credit_limit_minor: Some(500_000),
        statement_closing_day: Some(15),
        cc_network: Some("Visa".to_string()),
        metadata: None,
    }
}

// ============================================================================
// 1. Create bank account — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_bank_account_happy_path() {
    let pool = setup_db().await;
    let app = unique_app();

    let acct = acct_svc::create_bank_account(&pool, &app, &sample_bank(), None, "c1".to_string())
        .await
        .expect("create bank account failed");

    assert_eq!(acct.account_name, "Main Checking");
    assert_eq!(acct.currency, "USD");
    assert_eq!(acct.current_balance_minor, 0);
    assert!(acct.credit_limit_minor.is_none());

    // Verify persisted via read-back
    let fetched = acct_svc::get_account(&pool, &app, acct.id)
        .await
        .expect("get failed");
    assert!(fetched.is_some());
    assert_eq!(fetched.unwrap().id, acct.id);
}

// ============================================================================
// 2. Create CC account — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_cc_account_happy_path() {
    let pool = setup_db().await;
    let app = unique_app();

    let acct =
        acct_svc::create_credit_card_account(&pool, &app, &sample_cc(), None, "c1".to_string())
            .await
            .expect("create CC account failed");

    assert_eq!(acct.credit_limit_minor, Some(500_000));
    assert_eq!(acct.statement_closing_day, Some(15));
    assert_eq!(acct.cc_network.as_deref(), Some("Visa"));
    assert!(acct.routing_number.is_none());
}

// ============================================================================
// 3. Validation error — empty account name rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_bank_account_empty_name_rejected() {
    let pool = setup_db().await;
    let app = unique_app();

    let req = CreateBankAccountRequest {
        account_name: "   ".to_string(),
        institution: None,
        account_number_last4: None,
        routing_number: None,
        currency: "USD".to_string(),
        metadata: None,
    };

    let err = acct_svc::create_bank_account(&pool, &app, &req, None, "c1".to_string())
        .await
        .unwrap_err();

    assert!(
        matches!(err, AccountError::Validation(_)),
        "expected Validation error, got: {:?}",
        err
    );
}

// ============================================================================
// 4. List active-only — deactivated accounts excluded
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_active_only_excludes_deactivated() {
    let pool = setup_db().await;
    let app = unique_app();

    let a1 = acct_svc::create_bank_account(&pool, &app, &sample_bank(), None, "c1".to_string())
        .await
        .expect("create a1");

    let mut req2 = sample_bank();
    req2.account_name = "Savings".to_string();
    let a2 = acct_svc::create_bank_account(&pool, &app, &req2, None, "c2".to_string())
        .await
        .expect("create a2");

    acct_svc::deactivate_account(&pool, &app, a2.id, "system", "c3".to_string())
        .await
        .expect("deactivate failed");

    let active = acct_svc::list_accounts(&pool, &app, false)
        .await
        .expect("list active failed");
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, a1.id);

    let all = acct_svc::list_accounts(&pool, &app, true)
        .await
        .expect("list all failed");
    assert_eq!(all.len(), 2);
}

// ============================================================================
// 5. Update account fields
// ============================================================================

#[tokio::test]
#[serial]
async fn test_update_account_fields() {
    let pool = setup_db().await;
    let app = unique_app();

    let created =
        acct_svc::create_bank_account(&pool, &app, &sample_bank(), None, "c1".to_string())
            .await
            .expect("create failed");

    let updated = acct_svc::update_account(
        &pool,
        &app,
        created.id,
        &UpdateAccountRequest {
            account_name: Some("Updated Name".to_string()),
            institution: None,
            account_number_last4: None,
            routing_number: None,
            credit_limit_minor: None,
            statement_closing_day: None,
            cc_network: None,
            metadata: None,
        },
        "c2".to_string(),
    )
    .await
    .expect("update failed");

    assert_eq!(updated.account_name, "Updated Name");
    // Institution carries over when not explicitly cleared
    assert_eq!(updated.institution.as_deref(), Some("Test Bank"));
}

// ============================================================================
// 6. Deactivate nonexistent account — error case
// ============================================================================

#[tokio::test]
#[serial]
async fn test_deactivate_nonexistent_returns_not_found() {
    let pool = setup_db().await;
    let app = unique_app();

    let err =
        acct_svc::deactivate_account(&pool, &app, Uuid::new_v4(), "system", "c1".to_string())
            .await
            .unwrap_err();

    assert!(
        matches!(err, AccountError::NotFound(_)),
        "expected NotFound, got: {:?}",
        err
    );
}

// ============================================================================
// 7. Tenant isolation — account not visible to other app_id
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_isolation() {
    let pool = setup_db().await;
    let app_a = unique_app();
    let app_b = unique_app();

    let acct =
        acct_svc::create_bank_account(&pool, &app_a, &sample_bank(), None, "c1".to_string())
            .await
            .expect("create failed");

    // app_b cannot see app_a's account by ID
    let result = acct_svc::get_account(&pool, &app_b, acct.id)
        .await
        .expect("get failed");
    assert!(result.is_none(), "account should not be visible to other tenant");

    // app_b's list is empty
    let list = acct_svc::list_accounts(&pool, &app_b, true)
        .await
        .expect("list failed");
    assert!(list.is_empty(), "other tenant should see no accounts");
}
