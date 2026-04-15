//! Integration tests for elimination rules + posting idempotency (bd-2fdr).
//!
//! Covers:
//! 1. Create elimination rule — happy path
//! 2. Create rule for non-existent group (invalid ref rejected)
//! 3. Invalid rule_type validation (invalid ref rejected)
//! 4. Blank rule_name validation
//! 5. Duplicate rule name rejected
//! 6. Update elimination rule
//! 7. Delete elimination rule
//! 8. Elimination posting idempotency (exactly-once, ON CONFLICT DO NOTHING)
//! 9. Tenant isolation — cross-tenant rule access fails

use consolidation::domain::config::{
    service, service_rules, ConfigError, CreateEliminationRuleRequest, CreateGroupRequest,
    UpdateEliminationRuleRequest,
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://consolidation_user:consolidation_pass@localhost:5446/consolidation_db"
            .to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to consolidation test DB");

    let table_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_name = 'csl_groups')",
    )
    .fetch_one(&pool)
    .await
    .unwrap_or(false);

    if !table_exists {
        sqlx::migrate!("db/migrations")
            .run(&pool)
            .await
            .expect("Failed to run consolidation migrations");
    }

    pool
}

fn unique_tenant() -> String {
    format!("csl-elim-{}", Uuid::new_v4().simple())
}

fn group_req(name: &str) -> CreateGroupRequest {
    CreateGroupRequest {
        name: name.to_string(),
        description: None,
        reporting_currency: "USD".to_string(),
        fiscal_year_end_month: Some(12),
    }
}

fn ic_rule_req(name: &str) -> CreateEliminationRuleRequest {
    CreateEliminationRuleRequest {
        rule_name: name.to_string(),
        rule_type: "intercompany_receivable_payable".to_string(),
        debit_account_code: "1200".to_string(),
        credit_account_code: "2100".to_string(),
        description: Some("IC receivable vs payable".to_string()),
    }
}

// ============================================================================
// 1. Create elimination rule — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_elimination_rule_happy_path() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let group = service::create_group(&pool, &tid, &group_req("Elim Rule Group"))
        .await
        .unwrap();

    let rule =
        service_rules::create_elimination_rule(&pool, &tid, group.id, &ic_rule_req("IC Rule A"))
            .await
            .unwrap();

    assert_eq!(rule.rule_name, "IC Rule A");
    assert_eq!(rule.rule_type, "intercompany_receivable_payable");
    assert_eq!(rule.debit_account_code, "1200");
    assert_eq!(rule.credit_account_code, "2100");
    assert!(rule.is_active);
    assert_eq!(rule.group_id, group.id);

    let rules = service_rules::list_elimination_rules(&pool, &tid, group.id, false)
        .await
        .unwrap();
    assert_eq!(rules.len(), 1);
}

// ============================================================================
// 2. Create rule for non-existent group (invalid ref rejected)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_elimination_rule_invalid_group() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let ghost_group = Uuid::new_v4();

    let err = service_rules::create_elimination_rule(
        &pool,
        &tid,
        ghost_group,
        &ic_rule_req("Ghost Rule"),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, ConfigError::GroupNotFound(_)),
        "non-existent group ref should be rejected: {:?}",
        err
    );
}

// ============================================================================
// 3. Invalid rule_type validation (invalid ref rejected)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_elimination_rule_invalid_rule_type() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let group = service::create_group(&pool, &tid, &group_req("Rule Type Test Group"))
        .await
        .unwrap();

    let req = CreateEliminationRuleRequest {
        rule_name: "Bad Type Rule".to_string(),
        rule_type: "totally_invalid_type".to_string(),
        debit_account_code: "1200".to_string(),
        credit_account_code: "2100".to_string(),
        description: None,
    };

    let err = service_rules::create_elimination_rule(&pool, &tid, group.id, &req)
        .await
        .unwrap_err();

    assert!(
        matches!(err, ConfigError::Validation(_)),
        "invalid rule_type should be rejected: {:?}",
        err
    );
}

// ============================================================================
// 4. Blank rule_name validation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_elimination_rule_blank_name() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let group = service::create_group(&pool, &tid, &group_req("Blank Name Group"))
        .await
        .unwrap();

    let req = CreateEliminationRuleRequest {
        rule_name: "  ".to_string(),
        rule_type: "custom".to_string(),
        debit_account_code: "1200".to_string(),
        credit_account_code: "2100".to_string(),
        description: None,
    };

    let err = service_rules::create_elimination_rule(&pool, &tid, group.id, &req)
        .await
        .unwrap_err();

    assert!(
        matches!(err, ConfigError::Validation(_)),
        "blank name should fail: {:?}",
        err
    );
}

// ============================================================================
// 5. Duplicate rule name rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_elimination_rule_duplicate_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let group = service::create_group(&pool, &tid, &group_req("Dup Rule Group"))
        .await
        .unwrap();

    service_rules::create_elimination_rule(&pool, &tid, group.id, &ic_rule_req("Dup Rule"))
        .await
        .unwrap();

    let err =
        service_rules::create_elimination_rule(&pool, &tid, group.id, &ic_rule_req("Dup Rule"))
            .await
            .unwrap_err();

    assert!(
        matches!(err, ConfigError::Conflict(_)),
        "duplicate rule name should be rejected: {:?}",
        err
    );
}

// ============================================================================
// 6. Update elimination rule
// ============================================================================

#[tokio::test]
#[serial]
async fn test_update_elimination_rule() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let group = service::create_group(&pool, &tid, &group_req("Update Rule Group"))
        .await
        .unwrap();
    let rule =
        service_rules::create_elimination_rule(&pool, &tid, group.id, &ic_rule_req("Upd Rule"))
            .await
            .unwrap();

    let updated = service_rules::update_elimination_rule(
        &pool,
        &tid,
        rule.id,
        &UpdateEliminationRuleRequest {
            rule_name: Some("Upd Rule v2".to_string()),
            rule_type: None,
            debit_account_code: Some("1300".to_string()),
            credit_account_code: None,
            description: None,
            is_active: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.rule_name, "Upd Rule v2");
    assert_eq!(updated.debit_account_code, "1300");
    assert_eq!(updated.credit_account_code, "2100"); // unchanged
    assert_eq!(updated.rule_type, "intercompany_receivable_payable"); // unchanged
}

// ============================================================================
// 7. Delete elimination rule
// ============================================================================

#[tokio::test]
#[serial]
async fn test_delete_elimination_rule() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let group = service::create_group(&pool, &tid, &group_req("Del Rule Group"))
        .await
        .unwrap();
    let rule =
        service_rules::create_elimination_rule(&pool, &tid, group.id, &ic_rule_req("Del Rule"))
            .await
            .unwrap();

    service_rules::delete_elimination_rule(&pool, &tid, rule.id)
        .await
        .unwrap();

    let remaining = service_rules::list_elimination_rules(&pool, &tid, group.id, true)
        .await
        .unwrap();
    assert!(remaining.is_empty());
}

// ============================================================================
// 8. Elimination posting idempotency — exactly-once per group+period+key
// ============================================================================

#[tokio::test]
#[serial]
async fn test_elimination_posting_idempotency() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let group = service::create_group(&pool, &tid, &group_req("Idem Group"))
        .await
        .unwrap();
    let period_id = Uuid::new_v4();
    let idempotency_key = format!("elim-key-{}", Uuid::new_v4().simple());
    let ids_json = serde_json::json!([Uuid::new_v4().to_string()]);

    // First insert — should succeed
    sqlx::query(
        "INSERT INTO csl_elimination_postings
            (group_id, period_id, idempotency_key, journal_entry_ids,
             suggestion_count, total_amount_minor, posted_at)
         VALUES ($1, $2, $3, $4, $5, $6, NOW())",
    )
    .bind(group.id)
    .bind(period_id)
    .bind(&idempotency_key)
    .bind(&ids_json)
    .bind(1_i32)
    .bind(50000_i64)
    .execute(&pool)
    .await
    .unwrap();

    // Second insert with same key — ON CONFLICT DO NOTHING (idempotent)
    let result = sqlx::query(
        "INSERT INTO csl_elimination_postings
            (group_id, period_id, idempotency_key, journal_entry_ids,
             suggestion_count, total_amount_minor, posted_at)
         VALUES ($1, $2, $3, $4, $5, $6, NOW())
         ON CONFLICT (group_id, period_id, idempotency_key) DO NOTHING",
    )
    .bind(group.id)
    .bind(period_id)
    .bind(&idempotency_key)
    .bind(&ids_json)
    .bind(1_i32)
    .bind(50000_i64)
    .execute(&pool)
    .await
    .unwrap();

    // 0 rows affected — conflict was ignored (exactly-once)
    assert_eq!(result.rows_affected(), 0);

    // Verify only one row exists
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM csl_elimination_postings
         WHERE group_id = $1 AND period_id = $2 AND idempotency_key = $3",
    )
    .bind(group.id)
    .bind(period_id)
    .bind(&idempotency_key)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count.0, 1);
}

// ============================================================================
// 9. Tenant isolation — cross-tenant rule access fails
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_isolation_elimination_rules() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();

    let group_a = service::create_group(&pool, &tid_a, &group_req("A's Elim Group"))
        .await
        .unwrap();
    let rule_a =
        service_rules::create_elimination_rule(&pool, &tid_a, group_a.id, &ic_rule_req("A Rule"))
            .await
            .unwrap();

    // Tenant B cannot delete tenant A's rule — tenant-scoped fetch returns RuleNotFound (404)
    let err = service_rules::delete_elimination_rule(&pool, &tid_b, rule_a.id)
        .await
        .unwrap_err();
    assert!(
        matches!(err, ConfigError::RuleNotFound(_)),
        "cross-tenant delete should fail with RuleNotFound, got: {:?}",
        err
    );

    // The rule still exists for tenant A
    let rules = service_rules::list_elimination_rules(&pool, &tid_a, group_a.id, true)
        .await
        .unwrap();
    assert_eq!(rules.len(), 1);
}
