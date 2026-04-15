use super::*;
use serial_test::serial;

const TEST_APP: &str = "test-external-refs";

fn test_db_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db".to_string()
    })
}

async fn test_pool() -> PgPool {
    let pool = sqlx::PgPool::connect(&test_db_url())
        .await
        .expect("Failed to connect to integrations test database");
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Migrations failed");
    pool
}

async fn cleanup(pool: &PgPool) {
    sqlx::query("DELETE FROM integrations_outbox WHERE app_id = $1")
        .bind(TEST_APP)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_external_refs WHERE app_id = $1")
        .bind(TEST_APP)
        .execute(pool)
        .await
        .ok();
}

fn sample_req(
    entity_type: &str,
    entity_id: &str,
    system: &str,
    ext_id: &str,
) -> CreateExternalRefRequest {
    CreateExternalRefRequest {
        entity_type: entity_type.to_string(),
        entity_id: entity_id.to_string(),
        system: system.to_string(),
        external_id: ext_id.to_string(),
        label: Some("Test Label".to_string()),
        metadata: None,
    }
}

#[tokio::test]
#[serial]
async fn test_external_refs_create_and_get() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let req = sample_req("invoice", "inv-001", "stripe", "in_abc123");
    let created = create_external_ref(&pool, TEST_APP, &req, "corr-1".to_string())
        .await
        .expect("create_external_ref failed");

    assert_eq!(created.app_id, TEST_APP);
    assert_eq!(created.entity_type, "invoice");
    assert_eq!(created.entity_id, "inv-001");
    assert_eq!(created.system, "stripe");
    assert_eq!(created.external_id, "in_abc123");
    assert_eq!(created.label.as_deref(), Some("Test Label"));

    let fetched = get_external_ref(&pool, TEST_APP, created.id)
        .await
        .expect("get_external_ref failed");
    assert!(fetched.is_some());
    assert_eq!(fetched.unwrap().id, created.id);

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_external_refs_idempotent_create() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let req = sample_req("invoice", "inv-002", "stripe", "in_idem456");

    let first = create_external_ref(&pool, TEST_APP, &req, "corr-1".to_string())
        .await
        .expect("first create failed");

    // Same request again — should return same id (upsert, no error)
    let second = create_external_ref(&pool, TEST_APP, &req, "corr-2".to_string())
        .await
        .expect("second create failed");

    assert_eq!(first.id, second.id);
    assert_eq!(second.external_id, "in_idem456");

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_external_refs_list_by_entity() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let r1 = sample_req("customer", "cust-1", "stripe", "cus_111");
    let r2 = sample_req("customer", "cust-1", "quickbooks", "QB-222");
    let r3 = sample_req("customer", "cust-2", "stripe", "cus_999");

    create_external_ref(&pool, TEST_APP, &r1, "c1".to_string())
        .await
        .expect("create r1");
    create_external_ref(&pool, TEST_APP, &r2, "c2".to_string())
        .await
        .expect("create r2");
    create_external_ref(&pool, TEST_APP, &r3, "c3".to_string())
        .await
        .expect("create r3");

    let refs = list_by_entity(&pool, TEST_APP, "customer", "cust-1")
        .await
        .expect("list_by_entity failed");

    assert_eq!(refs.len(), 2);
    assert!(refs.iter().all(|r| r.entity_id == "cust-1"));

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_external_refs_get_by_external() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let req = sample_req("order", "ord-55", "salesforce", "SF-LEAD-789");
    let created = create_external_ref(&pool, TEST_APP, &req, "corr-x".to_string())
        .await
        .expect("create failed");

    let found = get_by_external(&pool, TEST_APP, "salesforce", "SF-LEAD-789")
        .await
        .expect("get_by_external failed");

    assert!(found.is_some());
    assert_eq!(found.unwrap().id, created.id);

    // Wrong app_id returns None
    let not_found = get_by_external(&pool, "other-app", "salesforce", "SF-LEAD-789")
        .await
        .expect("get_by_external failed");
    assert!(not_found.is_none());

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_external_refs_update() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let req = sample_req("invoice", "inv-010", "xero", "XERO-INV-1");
    let created = create_external_ref(&pool, TEST_APP, &req, "corr-1".to_string())
        .await
        .expect("create failed");

    let upd_req = UpdateExternalRefRequest {
        label: Some("Updated Label".to_string()),
        metadata: None,
    };
    let updated = update_external_ref(&pool, TEST_APP, created.id, &upd_req, "corr-2".to_string())
        .await
        .expect("update failed");

    assert_eq!(updated.label.as_deref(), Some("Updated Label"));
    assert_eq!(updated.external_id, "XERO-INV-1");

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_external_refs_delete() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let req = sample_req("party", "party-42", "hubspot", "HS-CONTACT-42");
    let created = create_external_ref(&pool, TEST_APP, &req, "corr-1".to_string())
        .await
        .expect("create failed");

    delete_external_ref(&pool, TEST_APP, created.id, "corr-2".to_string())
        .await
        .expect("delete failed");

    let gone = get_external_ref(&pool, TEST_APP, created.id)
        .await
        .expect("get after delete failed");
    assert!(gone.is_none());

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_external_refs_outbox_event_on_create() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let req = sample_req("invoice", "inv-outbox", "stripe", "in_outbox_test");
    let created = create_external_ref(&pool, TEST_APP, &req, "corr-out".to_string())
        .await
        .expect("create failed");

    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox WHERE aggregate_type = 'external_ref' AND aggregate_id = $1 AND app_id = $2",
    )
    .bind(created.id.to_string())
    .bind(TEST_APP)
    .fetch_one(&pool)
    .await
    .expect("outbox query failed");

    assert!(count.0 >= 1, "expected outbox event for created ref");

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_external_refs_tenant_isolation() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let req = sample_req("invoice", "inv-iso", "stripe", "in_iso_999");
    let created = create_external_ref(&pool, TEST_APP, &req, "corr-1".to_string())
        .await
        .expect("create failed");

    // Different app_id cannot see this ref
    let not_found = get_external_ref(&pool, "other-tenant", created.id)
        .await
        .expect("tenant isolation get failed");
    assert!(not_found.is_none());

    // Update from wrong tenant should fail
    let upd_req = UpdateExternalRefRequest {
        label: Some("Hacked".to_string()),
        metadata: None,
    };
    let err =
        update_external_ref(&pool, "other-tenant", created.id, &upd_req, "c".to_string()).await;
    assert!(err.is_err());

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_external_refs_delete_wrong_tenant_fails() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let req = sample_req("invoice", "inv-del-iso", "stripe", "in_del_iso");
    let created = create_external_ref(&pool, TEST_APP, &req, "corr-1".to_string())
        .await
        .expect("create failed");

    let err = delete_external_ref(&pool, "other-tenant", created.id, "corr-2".to_string()).await;
    assert!(err.is_err());

    // Still exists for correct tenant
    let still_there = get_external_ref(&pool, TEST_APP, created.id)
        .await
        .expect("get failed");
    assert!(still_there.is_some());

    cleanup(&pool).await;
}
