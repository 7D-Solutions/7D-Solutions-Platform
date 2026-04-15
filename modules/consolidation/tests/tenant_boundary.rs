//! Tenant-boundary integration tests for consolidation read routes (bd-fznlw).
//!
//! Verifies that `get_entity` and `get_elimination_rule` return 404 when a
//! caller from tenant B requests a row owned by tenant A — closing the
//! cross-tenant read leak confirmed in bead bd-fznlw.
//!
//! Test matrix:
//! 1. Tenant B GET entity owned by tenant A → EntityNotFound (404)
//! 2. Tenant B GET elimination rule owned by tenant A → RuleNotFound (404)
//! 3. Tenant A GET own entity → 200 with expected payload (regression guard)
//! 4. Tenant A GET own elimination rule → 200 with expected payload (regression guard)

use consolidation::domain::config::{
    service, service_rules, ConfigError, CreateEliminationRuleRequest, CreateEntityRequest,
    CreateGroupRequest,
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

fn unique_tenant(prefix: &str) -> String {
    format!("tb-{}-{}", prefix, Uuid::new_v4().simple())
}

fn group_req(name: &str) -> CreateGroupRequest {
    CreateGroupRequest {
        name: name.to_string(),
        description: None,
        reporting_currency: "USD".to_string(),
        fiscal_year_end_month: Some(12),
    }
}

fn entity_req(entity_tid: &str) -> CreateEntityRequest {
    CreateEntityRequest {
        entity_tenant_id: entity_tid.to_string(),
        entity_name: "Boundary Test Sub".to_string(),
        functional_currency: "USD".to_string(),
        ownership_pct_bp: Some(10000),
        consolidation_method: Some("full".to_string()),
    }
}

fn rule_req() -> CreateEliminationRuleRequest {
    CreateEliminationRuleRequest {
        rule_name: "Boundary IC Rule".to_string(),
        rule_type: "intercompany_receivable_payable".to_string(),
        debit_account_code: "1200".to_string(),
        credit_account_code: "2100".to_string(),
        description: None,
    }
}

// ============================================================================
// 1. Cross-tenant entity read → EntityNotFound (404)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_cross_tenant_get_entity_returns_not_found() {
    let pool = setup_db().await;
    let tid_a = unique_tenant("a");
    let tid_b = unique_tenant("b");

    // Tenant A creates a group + entity
    let group_a = service::create_group(&pool, &tid_a, &group_req("A Boundary Group"))
        .await
        .unwrap();
    let entity_a = service::create_entity(&pool, &tid_a, group_a.id, &entity_req("sub-a"))
        .await
        .unwrap();

    // Tenant B requests tenant A's entity by UUID — must get EntityNotFound, not the row
    let err = service::get_entity(&pool, &tid_b, entity_a.id)
        .await
        .unwrap_err();

    assert!(
        matches!(err, ConfigError::EntityNotFound(_)),
        "cross-tenant entity read must return EntityNotFound, got: {:?}",
        err
    );
}

// ============================================================================
// 2. Cross-tenant elimination rule read → RuleNotFound (404)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_cross_tenant_get_elimination_rule_returns_not_found() {
    let pool = setup_db().await;
    let tid_a = unique_tenant("a");
    let tid_b = unique_tenant("b");

    // Tenant A creates a group + elimination rule
    let group_a = service::create_group(&pool, &tid_a, &group_req("A Elim Boundary Group"))
        .await
        .unwrap();
    let rule_a =
        service_rules::create_elimination_rule(&pool, &tid_a, group_a.id, &rule_req())
            .await
            .unwrap();

    // Tenant B requests tenant A's rule by UUID — must get RuleNotFound, not the row
    let err = service_rules::get_elimination_rule(&pool, &tid_b, rule_a.id)
        .await
        .unwrap_err();

    assert!(
        matches!(err, ConfigError::RuleNotFound(_)),
        "cross-tenant rule read must return RuleNotFound, got: {:?}",
        err
    );
}

// ============================================================================
// 3. Tenant A can still read its own entity (regression guard)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_owner_tenant_can_read_own_entity() {
    let pool = setup_db().await;
    let tid_a = unique_tenant("a");

    let group_a = service::create_group(&pool, &tid_a, &group_req("A Own Entity Group"))
        .await
        .unwrap();
    let entity_a = service::create_entity(&pool, &tid_a, group_a.id, &entity_req("sub-own"))
        .await
        .unwrap();

    let fetched = service::get_entity(&pool, &tid_a, entity_a.id)
        .await
        .unwrap();

    assert_eq!(fetched.id, entity_a.id);
    assert_eq!(fetched.entity_tenant_id, "sub-own");
    assert_eq!(fetched.group_id, group_a.id);
}

// ============================================================================
// 4. Tenant A can still read its own elimination rule (regression guard)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_owner_tenant_can_read_own_elimination_rule() {
    let pool = setup_db().await;
    let tid_a = unique_tenant("a");

    let group_a = service::create_group(&pool, &tid_a, &group_req("A Own Rule Group"))
        .await
        .unwrap();
    let rule_a =
        service_rules::create_elimination_rule(&pool, &tid_a, group_a.id, &rule_req())
            .await
            .unwrap();

    let fetched = service_rules::get_elimination_rule(&pool, &tid_a, rule_a.id)
        .await
        .unwrap();

    assert_eq!(fetched.id, rule_a.id);
    assert_eq!(fetched.rule_name, "Boundary IC Rule");
    assert_eq!(fetched.group_id, group_a.id);
}
