//! Integrated tests for external refs CRUD (bd-3lwh).
//!
//! Covers:
//! 1. Create external ref — happy path (create + get)
//! 2. Idempotent upsert (same system+external_id returns same row)
//! 3. List by entity
//! 4. Get by external system
//! 5. Update label/metadata
//! 6. Delete
//! 7. Get not found — error case
//! 8. Update not found — error case
//! 9. Tenant isolation (cross-tenant access fails)

use integrations_rs::domain::external_refs::{
    service::{
        create_external_ref, delete_external_ref, get_by_external, get_external_ref,
        list_by_entity, update_external_ref,
    },
    CreateExternalRefRequest, ExternalRefError, UpdateExternalRefRequest,
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://integrations_user:integrations_pass@localhost:5449/integrations_db"
            .to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to integrations test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run integrations migrations");
    pool
}

fn unique_tenant() -> String {
    format!("ext-ref-{}", Uuid::new_v4().simple())
}

fn corr() -> String {
    Uuid::new_v4().to_string()
}

fn make_req(
    entity_type: &str,
    entity_id: &str,
    system: &str,
    external_id: &str,
) -> CreateExternalRefRequest {
    CreateExternalRefRequest {
        entity_type: entity_type.to_string(),
        entity_id: entity_id.to_string(),
        system: system.to_string(),
        external_id: external_id.to_string(),
        label: Some("test-label".to_string()),
        metadata: None,
    }
}

// ============================================================================
// 1. Create + get — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_external_ref_create_and_get() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let req = make_req("invoice", "inv-001", "stripe", "in_abc123");
    let created = create_external_ref(&pool, &tid, &req, corr())
        .await
        .expect("create_external_ref failed");

    assert_eq!(created.app_id, tid);
    assert_eq!(created.entity_type, "invoice");
    assert_eq!(created.entity_id, "inv-001");
    assert_eq!(created.system, "stripe");
    assert_eq!(created.external_id, "in_abc123");
    assert_eq!(created.label.as_deref(), Some("test-label"));

    let fetched = get_external_ref(&pool, &tid, created.id)
        .await
        .expect("get_external_ref failed");
    assert!(fetched.is_some());
    assert_eq!(fetched.unwrap().id, created.id);
}

// ============================================================================
// 2. Idempotent upsert — same (app_id, system, external_id) returns same row
// ============================================================================

#[tokio::test]
#[serial]
async fn test_external_ref_idempotent_upsert() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let req = make_req("invoice", "inv-002", "stripe", "in_idem456");

    let first = create_external_ref(&pool, &tid, &req, corr())
        .await
        .expect("first create failed");

    let second = create_external_ref(&pool, &tid, &req, corr())
        .await
        .expect("second create (upsert) failed");

    assert_eq!(first.id, second.id, "upsert should return same row id");
    assert_eq!(second.external_id, "in_idem456");
}

// ============================================================================
// 3. List by entity
// ============================================================================

#[tokio::test]
#[serial]
async fn test_external_ref_list_by_entity() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let r1 = make_req("customer", "cust-1", "stripe", "cus_111");
    let r2 = make_req("customer", "cust-1", "quickbooks", "QB-222");
    let r3 = make_req("customer", "cust-2", "stripe", "cus_999");

    create_external_ref(&pool, &tid, &r1, corr())
        .await
        .expect("create r1");
    create_external_ref(&pool, &tid, &r2, corr())
        .await
        .expect("create r2");
    create_external_ref(&pool, &tid, &r3, corr())
        .await
        .expect("create r3");

    let refs = list_by_entity(&pool, &tid, "customer", "cust-1")
        .await
        .expect("list_by_entity failed");

    assert_eq!(refs.len(), 2, "expected 2 refs for cust-1");
    assert!(refs.iter().all(|r| r.entity_id == "cust-1"));
    assert!(refs.iter().all(|r| r.app_id == tid));
}

// ============================================================================
// 4. Get by external system
// ============================================================================

#[tokio::test]
#[serial]
async fn test_external_ref_get_by_external() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let req = make_req("order", "ord-55", "salesforce", "SF-LEAD-789");
    let created = create_external_ref(&pool, &tid, &req, corr())
        .await
        .expect("create failed");

    let found = get_by_external(&pool, &tid, "salesforce", "SF-LEAD-789")
        .await
        .expect("get_by_external failed");

    assert!(found.is_some());
    assert_eq!(found.unwrap().id, created.id);

    // Missing external_id returns None
    let none = get_by_external(&pool, &tid, "salesforce", "NONEXISTENT")
        .await
        .expect("get_by_external (missing) failed");
    assert!(none.is_none());
}

// ============================================================================
// 5. Update
// ============================================================================

#[tokio::test]
#[serial]
async fn test_external_ref_update() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let req = make_req("invoice", "inv-010", "xero", "XERO-INV-1");
    let created = create_external_ref(&pool, &tid, &req, corr())
        .await
        .expect("create failed");

    let upd_req = UpdateExternalRefRequest {
        label: Some("Updated Label".to_string()),
        metadata: Some(serde_json::json!({"key": "value"})),
    };
    let updated = update_external_ref(&pool, &tid, created.id, &upd_req, corr())
        .await
        .expect("update failed");

    assert_eq!(updated.label.as_deref(), Some("Updated Label"));
    assert_eq!(updated.external_id, "XERO-INV-1");
    assert!(updated.metadata.is_some());
}

// ============================================================================
// 6. Delete
// ============================================================================

#[tokio::test]
#[serial]
async fn test_external_ref_delete() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let req = make_req("party", "party-42", "hubspot", "HS-CONTACT-42");
    let created = create_external_ref(&pool, &tid, &req, corr())
        .await
        .expect("create failed");

    delete_external_ref(&pool, &tid, created.id, corr())
        .await
        .expect("delete failed");

    let gone = get_external_ref(&pool, &tid, created.id)
        .await
        .expect("get after delete failed");
    assert!(gone.is_none());
}

// ============================================================================
// 7. Get not found — error case
// ============================================================================

#[tokio::test]
#[serial]
async fn test_external_ref_get_not_found() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let result = get_external_ref(&pool, &tid, i64::MAX)
        .await
        .expect("get should not return Err for missing row");

    assert!(result.is_none(), "expected None for non-existent ref");
}

// ============================================================================
// 8. Update not found — error case
// ============================================================================

#[tokio::test]
#[serial]
async fn test_external_ref_update_not_found() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let upd_req = UpdateExternalRefRequest {
        label: Some("Ghost".to_string()),
        metadata: None,
    };

    let err = update_external_ref(&pool, &tid, i64::MAX, &upd_req, corr()).await;
    assert!(err.is_err(), "update on non-existent ref should fail");
    match err.unwrap_err() {
        ExternalRefError::NotFound(_) => {}
        other => panic!("expected NotFound, got: {:?}", other),
    }
}

// ============================================================================
// 9. Tenant isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_external_ref_tenant_isolation() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();

    // Tenant A creates a ref
    let req = make_req("invoice", "inv-iso", "stripe", "in_iso_999");
    let created = create_external_ref(&pool, &tid_a, &req, corr())
        .await
        .expect("create failed");

    // Tenant B cannot see it by id
    let not_found = get_external_ref(&pool, &tid_b, created.id)
        .await
        .expect("tenant-b get should not DB-error");
    assert!(not_found.is_none(), "tenant B must not see tenant A's ref");

    // Tenant B cannot see it by external lookup
    let not_found_ext = get_by_external(&pool, &tid_b, "stripe", "in_iso_999")
        .await
        .expect("tenant-b get_by_external should not DB-error");
    assert!(
        not_found_ext.is_none(),
        "tenant B must not find tenant A's external ref"
    );

    // Tenant B cannot update it
    let upd_req = UpdateExternalRefRequest {
        label: Some("Hacked".to_string()),
        metadata: None,
    };
    let err = update_external_ref(&pool, &tid_b, created.id, &upd_req, corr()).await;
    assert!(err.is_err(), "tenant B must not update tenant A's ref");

    // Tenant B cannot delete it
    let del_err = delete_external_ref(&pool, &tid_b, created.id, corr()).await;
    assert!(del_err.is_err(), "tenant B must not delete tenant A's ref");

    // Tenant A's ref is still intact
    let still_there = get_external_ref(&pool, &tid_a, created.id)
        .await
        .expect("tenant-a get should succeed")
        .expect("tenant A's ref should still exist");
    assert_eq!(still_there.id, created.id);
}
