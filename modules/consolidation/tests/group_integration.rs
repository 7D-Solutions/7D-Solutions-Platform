//! Integration tests for consolidation group CRUD (bd-2fdr).
//!
//! Covers:
//! 1. Create group — happy path
//! 2. Duplicate name rejection
//! 3. Blank name validation
//! 4. Invalid currency validation
//! 5. List groups with active filter
//! 6. Get group not found
//! 7. Update group
//! 8. Delete group
//! 9. Tenant isolation

use consolidation::domain::config::{service, ConfigError, CreateGroupRequest, UpdateGroupRequest};
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

    // Only run migrations if tables don't already exist.
    // The test DB may have been provisioned via direct SQL rather than sqlx migrate.
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
    format!("csl-grp-{}", Uuid::new_v4().simple())
}

fn base_group_req(name: &str) -> CreateGroupRequest {
    CreateGroupRequest {
        name: name.to_string(),
        description: Some("Integration test group".to_string()),
        reporting_currency: "USD".to_string(),
        fiscal_year_end_month: Some(12),
    }
}

// ============================================================================
// 1. Create group — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_group_happy_path() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let group = service::create_group(&pool, &tid, &base_group_req("Acme Holdings"))
        .await
        .unwrap();

    assert_eq!(group.name, "Acme Holdings");
    assert_eq!(group.tenant_id, tid);
    assert_eq!(group.reporting_currency, "USD");
    assert_eq!(group.fiscal_year_end_month, 12);
    assert!(group.is_active);

    let fetched = service::get_group(&pool, &tid, group.id).await.unwrap();
    assert_eq!(fetched.id, group.id);
    assert_eq!(fetched.name, "Acme Holdings");
}

// ============================================================================
// 2. Duplicate name rejection
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_group_duplicate_name_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    service::create_group(&pool, &tid, &base_group_req("Dup Group"))
        .await
        .unwrap();

    let err = service::create_group(&pool, &tid, &base_group_req("Dup Group"))
        .await
        .unwrap_err();

    assert!(
        matches!(err, ConfigError::Conflict(_)),
        "expected Conflict, got: {:?}",
        err
    );
}

// ============================================================================
// 3. Blank name validation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_group_blank_name_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let req = CreateGroupRequest {
        name: "   ".to_string(),
        description: None,
        reporting_currency: "USD".to_string(),
        fiscal_year_end_month: None,
    };

    let err = service::create_group(&pool, &tid, &req).await.unwrap_err();
    assert!(
        matches!(err, ConfigError::Validation(_)),
        "expected Validation, got: {:?}",
        err
    );
}

// ============================================================================
// 4. Invalid currency validation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_group_invalid_currency_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let req = CreateGroupRequest {
        name: "Bad Currency Group".to_string(),
        description: None,
        reporting_currency: "USDX".to_string(), // 4 chars, not ISO 4217
        fiscal_year_end_month: None,
    };

    let err = service::create_group(&pool, &tid, &req).await.unwrap_err();
    assert!(
        matches!(err, ConfigError::Validation(_)),
        "expected Validation, got: {:?}",
        err
    );
}

// ============================================================================
// 5. List groups with active filter
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_groups_active_filter() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let g1 = service::create_group(&pool, &tid, &base_group_req("Active One"))
        .await
        .unwrap();
    service::create_group(&pool, &tid, &base_group_req("Active Two"))
        .await
        .unwrap();

    // Deactivate g1
    service::update_group(
        &pool,
        &tid,
        g1.id,
        &UpdateGroupRequest {
            name: None,
            description: None,
            reporting_currency: None,
            fiscal_year_end_month: None,
            is_active: Some(false),
        },
    )
    .await
    .unwrap();

    let active = service::list_groups(&pool, &tid, false).await.unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].name, "Active Two");

    let all = service::list_groups(&pool, &tid, true).await.unwrap();
    assert_eq!(all.len(), 2);
}

// ============================================================================
// 6. Get group not found
// ============================================================================

#[tokio::test]
#[serial]
async fn test_get_group_not_found() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let err = service::get_group(&pool, &tid, Uuid::new_v4())
        .await
        .unwrap_err();
    assert!(matches!(err, ConfigError::GroupNotFound(_)));
}

// ============================================================================
// 7. Update group
// ============================================================================

#[tokio::test]
#[serial]
async fn test_update_group() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let group = service::create_group(&pool, &tid, &base_group_req("Old Name"))
        .await
        .unwrap();

    let updated = service::update_group(
        &pool,
        &tid,
        group.id,
        &UpdateGroupRequest {
            name: Some("New Name".to_string()),
            description: None,
            reporting_currency: None,
            fiscal_year_end_month: Some(6),
            is_active: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.name, "New Name");
    assert_eq!(updated.fiscal_year_end_month, 6);
    assert_eq!(updated.reporting_currency, "USD"); // unchanged
    assert!(updated.is_active); // unchanged
}

// ============================================================================
// 8. Delete group
// ============================================================================

#[tokio::test]
#[serial]
async fn test_delete_group() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let group = service::create_group(&pool, &tid, &base_group_req("Delete Me"))
        .await
        .unwrap();

    service::delete_group(&pool, &tid, group.id).await.unwrap();

    let err = service::get_group(&pool, &tid, group.id).await.unwrap_err();
    assert!(matches!(err, ConfigError::GroupNotFound(_)));
}

// ============================================================================
// 9. Tenant isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_isolation_groups() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();

    let group = service::create_group(&pool, &tid_a, &base_group_req("A's Group"))
        .await
        .unwrap();

    // Tenant B cannot read tenant A's group
    let err = service::get_group(&pool, &tid_b, group.id)
        .await
        .unwrap_err();
    assert!(matches!(err, ConfigError::GroupNotFound(_)));

    // Tenant B's list is empty
    let list = service::list_groups(&pool, &tid_b, true).await.unwrap();
    assert!(list.is_empty());

    // Tenant B cannot delete tenant A's group
    let err = service::delete_group(&pool, &tid_b, group.id)
        .await
        .unwrap_err();
    assert!(matches!(err, ConfigError::GroupNotFound(_)));
}
