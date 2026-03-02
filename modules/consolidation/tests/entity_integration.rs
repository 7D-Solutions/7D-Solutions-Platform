//! Integration tests for consolidation group-entity CRUD (bd-2fdr).
//!
//! Covers:
//! 1. Create entity — happy path
//! 2. Create entity for non-existent group (invalid ref rejected)
//! 3. Duplicate entity in same group rejected
//! 4. Invalid ownership_pct_bp validation
//! 5. Invalid consolidation_method validation
//! 6. List entities with active filter
//! 7. Update entity
//! 8. Delete entity
//! 9. Tenant isolation — cross-tenant entity access fails

use consolidation::domain::config::{
    service, ConfigError, CreateEntityRequest, CreateGroupRequest, UpdateEntityRequest,
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
    format!("csl-ent-{}", Uuid::new_v4().simple())
}

fn group_req(name: &str) -> CreateGroupRequest {
    CreateGroupRequest {
        name: name.to_string(),
        description: None,
        reporting_currency: "USD".to_string(),
        fiscal_year_end_month: Some(12),
    }
}

fn entity_req(entity_tid: &str, entity_name: &str) -> CreateEntityRequest {
    CreateEntityRequest {
        entity_tenant_id: entity_tid.to_string(),
        entity_name: entity_name.to_string(),
        functional_currency: "USD".to_string(),
        ownership_pct_bp: Some(10000),
        consolidation_method: Some("full".to_string()),
    }
}

// ============================================================================
// 1. Create entity — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_entity_happy_path() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let group = service::create_group(&pool, &tid, &group_req("Entity Test Group"))
        .await
        .unwrap();

    let entity = service::create_entity(&pool, &tid, group.id, &entity_req("sub-us", "US Sub"))
        .await
        .unwrap();

    assert_eq!(entity.entity_tenant_id, "sub-us");
    assert_eq!(entity.entity_name, "US Sub");
    assert_eq!(entity.functional_currency, "USD");
    assert_eq!(entity.ownership_pct_bp, 10000);
    assert_eq!(entity.consolidation_method, "full");
    assert!(entity.is_active);
    assert_eq!(entity.group_id, group.id);
}

// ============================================================================
// 2. Create entity for non-existent group (invalid ref rejected)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_entity_invalid_group_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let ghost_group = Uuid::new_v4();

    let err = service::create_entity(&pool, &tid, ghost_group, &entity_req("sub-x", "Sub X"))
        .await
        .unwrap_err();

    assert!(
        matches!(err, ConfigError::GroupNotFound(_)),
        "expected GroupNotFound for non-existent group, got: {:?}",
        err
    );
}

// ============================================================================
// 3. Duplicate entity in same group rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_entity_duplicate_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let group = service::create_group(&pool, &tid, &group_req("Dup Entity Group"))
        .await
        .unwrap();

    service::create_entity(&pool, &tid, group.id, &entity_req("sub-dup", "Sub Dup"))
        .await
        .unwrap();

    let err = service::create_entity(&pool, &tid, group.id, &entity_req("sub-dup", "Sub Dup 2"))
        .await
        .unwrap_err();

    assert!(
        matches!(err, ConfigError::Conflict(_)),
        "expected Conflict for duplicate entity, got: {:?}",
        err
    );
}

// ============================================================================
// 4. Invalid ownership_pct_bp validation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_entity_invalid_ownership_bp() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let group = service::create_group(&pool, &tid, &group_req("BP Test Group"))
        .await
        .unwrap();

    // Zero bp
    let req_zero = CreateEntityRequest {
        entity_tenant_id: "sub-zero".to_string(),
        entity_name: "Zero Sub".to_string(),
        functional_currency: "USD".to_string(),
        ownership_pct_bp: Some(0),
        consolidation_method: None,
    };
    let err = service::create_entity(&pool, &tid, group.id, &req_zero)
        .await
        .unwrap_err();
    assert!(
        matches!(err, ConfigError::Validation(_)),
        "bp=0 should fail: {:?}",
        err
    );

    // Over 100% bp
    let req_over = CreateEntityRequest {
        entity_tenant_id: "sub-over".to_string(),
        entity_name: "Over Sub".to_string(),
        functional_currency: "USD".to_string(),
        ownership_pct_bp: Some(10001),
        consolidation_method: None,
    };
    let err = service::create_entity(&pool, &tid, group.id, &req_over)
        .await
        .unwrap_err();
    assert!(
        matches!(err, ConfigError::Validation(_)),
        "bp>10000 should fail: {:?}",
        err
    );
}

// ============================================================================
// 5. Invalid consolidation_method validation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_entity_invalid_consolidation_method() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let group = service::create_group(&pool, &tid, &group_req("Method Test Group"))
        .await
        .unwrap();

    let req = CreateEntityRequest {
        entity_tenant_id: "sub-bad".to_string(),
        entity_name: "Bad Method Sub".to_string(),
        functional_currency: "USD".to_string(),
        ownership_pct_bp: None,
        consolidation_method: Some("bogus".to_string()),
    };

    let err = service::create_entity(&pool, &tid, group.id, &req)
        .await
        .unwrap_err();
    assert!(
        matches!(err, ConfigError::Validation(_)),
        "invalid method should fail: {:?}",
        err
    );
}

// ============================================================================
// 6. List entities with active filter
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_entities_active_filter() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let group = service::create_group(&pool, &tid, &group_req("List Entity Group"))
        .await
        .unwrap();

    let e1 = service::create_entity(&pool, &tid, group.id, &entity_req("sub-e1", "Sub E1"))
        .await
        .unwrap();
    service::create_entity(&pool, &tid, group.id, &entity_req("sub-e2", "Sub E2"))
        .await
        .unwrap();

    // Deactivate e1
    service::update_entity(
        &pool,
        &tid,
        e1.id,
        &UpdateEntityRequest {
            entity_name: None,
            functional_currency: None,
            ownership_pct_bp: None,
            consolidation_method: None,
            is_active: Some(false),
        },
    )
    .await
    .unwrap();

    let active = service::list_entities(&pool, &tid, group.id, false)
        .await
        .unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].entity_tenant_id, "sub-e2");

    let all = service::list_entities(&pool, &tid, group.id, true)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
}

// ============================================================================
// 7. Update entity
// ============================================================================

#[tokio::test]
#[serial]
async fn test_update_entity() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let group = service::create_group(&pool, &tid, &group_req("Update Entity Group"))
        .await
        .unwrap();
    let entity = service::create_entity(&pool, &tid, group.id, &entity_req("sub-upd", "Old Name"))
        .await
        .unwrap();

    let updated = service::update_entity(
        &pool,
        &tid,
        entity.id,
        &UpdateEntityRequest {
            entity_name: Some("New Sub Name".to_string()),
            functional_currency: None,
            ownership_pct_bp: Some(7500),
            consolidation_method: Some("proportional".to_string()),
            is_active: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.entity_name, "New Sub Name");
    assert_eq!(updated.ownership_pct_bp, 7500);
    assert_eq!(updated.consolidation_method, "proportional");
    assert_eq!(updated.functional_currency, "USD"); // unchanged
}

// ============================================================================
// 8. Delete entity
// ============================================================================

#[tokio::test]
#[serial]
async fn test_delete_entity() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let group = service::create_group(&pool, &tid, &group_req("Delete Entity Group"))
        .await
        .unwrap();
    let entity = service::create_entity(&pool, &tid, group.id, &entity_req("sub-del", "Del Sub"))
        .await
        .unwrap();

    service::delete_entity(&pool, &tid, entity.id)
        .await
        .unwrap();

    let remaining = service::list_entities(&pool, &tid, group.id, true)
        .await
        .unwrap();
    assert!(remaining.is_empty());
}

// ============================================================================
// 9. Tenant isolation — cross-tenant entity access fails
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_isolation_entities() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();

    let group_a = service::create_group(&pool, &tid_a, &group_req("A's Group"))
        .await
        .unwrap();
    let group_b = service::create_group(&pool, &tid_b, &group_req("B's Group"))
        .await
        .unwrap();

    let entity_a =
        service::create_entity(&pool, &tid_a, group_a.id, &entity_req("sub-a1", "A Sub"))
            .await
            .unwrap();

    // Tenant B cannot see tenant A's entities by group
    let list_b = service::list_entities(&pool, &tid_b, group_b.id, true)
        .await
        .unwrap();
    assert!(list_b.is_empty());

    // Tenant B cannot delete tenant A's entity (group ownership check)
    let err = service::delete_entity(&pool, &tid_b, entity_a.id)
        .await
        .unwrap_err();
    assert!(
        matches!(err, ConfigError::GroupNotFound(_)),
        "cross-tenant delete should fail: {:?}",
        err
    );
}
