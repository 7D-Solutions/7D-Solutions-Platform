use super::*;
use crate::domain::po::queries::get_po;
use serial_test::serial;

const TEST_TENANT: &str = "test-tenant-pos";

fn test_db_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://ap_user:ap_pass@localhost:5443/ap_db".to_string())
}

async fn test_pool() -> PgPool {
    PgPool::connect(&test_db_url())
        .await
        .expect("Failed to connect to AP test database")
}

async fn create_test_vendor(pool: &PgPool) -> Uuid {
    let vendor_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO vendors (vendor_id, tenant_id, name, currency, payment_terms_days,
           is_active, created_at, updated_at)
           VALUES ($1, $2, $3, 'USD', 30, TRUE, NOW(), NOW())"#,
    )
    .bind(vendor_id)
    .bind(TEST_TENANT)
    .bind(format!("Test Vendor PO {}", vendor_id))
    .execute(pool)
    .await
    .expect("insert test vendor failed");
    vendor_id
}

async fn cleanup(pool: &PgPool) {
    sqlx::query(
        "DELETE FROM events_outbox WHERE aggregate_type = 'po' \
         AND aggregate_id IN (SELECT po_id::TEXT FROM purchase_orders WHERE tenant_id = $1)",
    )
    .bind(TEST_TENANT)
    .execute(pool)
    .await
    .ok();

    sqlx::query(
        "DELETE FROM po_status WHERE po_id IN \
         (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
    )
    .bind(TEST_TENANT)
    .execute(pool)
    .await
    .ok();

    sqlx::query(
        "DELETE FROM po_lines WHERE po_id IN \
         (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
    )
    .bind(TEST_TENANT)
    .execute(pool)
    .await
    .ok();

    sqlx::query("DELETE FROM purchase_orders WHERE tenant_id = $1")
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();

    sqlx::query(
        "DELETE FROM events_outbox WHERE aggregate_type = 'vendor' \
         AND aggregate_id IN (SELECT vendor_id::TEXT FROM vendors WHERE tenant_id = $1)",
    )
    .bind(TEST_TENANT)
    .execute(pool)
    .await
    .ok();

    sqlx::query("DELETE FROM vendors WHERE tenant_id = $1")
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();
}

fn sample_req(vendor_id: Uuid) -> CreatePoRequest {
    CreatePoRequest {
        vendor_id,
        currency: "USD".to_string(),
        created_by: "user-ap".to_string(),
        expected_delivery_date: None,
        lines: vec![
            CreatePoLineRequest {
                item_id: None,
                description: Some("Office chairs".to_string()),
                quantity: 10.0,
                unit_of_measure: "each".to_string(),
                unit_price_minor: 45_000,
                gl_account_code: "6100".to_string(),
            },
            CreatePoLineRequest {
                item_id: Some(Uuid::new_v4()),
                description: None,
                quantity: 5.0,
                unit_of_measure: "each".to_string(),
                unit_price_minor: 10_000,
                gl_account_code: "6200".to_string(),
            },
        ],
    }
}

#[tokio::test]
#[serial]
async fn test_create_po_draft_with_lines() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let vendor_id = create_test_vendor(&pool).await;

    let req = sample_req(vendor_id);
    let result = create_po(&pool, TEST_TENANT, &req, "corr-1".to_string())
        .await
        .expect("create_po failed");

    assert_eq!(result.po.vendor_id, vendor_id);
    assert_eq!(result.po.status, "draft");
    assert_eq!(result.po.currency, "USD");
    // total = 10*45000 + 5*10000 = 500000
    assert_eq!(result.po.total_minor, 500_000);
    assert_eq!(result.lines.len(), 2);
    assert!(result.po.po_number.starts_with("PO-"));

    // first line: description-only, item_id absent
    assert_eq!(result.lines[0].description, "Office chairs");
    assert!(result.lines[0].item_id.is_none());

    // second line: item_id present, description is empty (not encoded into description)
    assert!(result.lines[1].item_id.is_some());
    assert!(
        !result.lines[1].description.starts_with("item:"),
        "item_id must not be encoded into description"
    );

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_create_po_vendor_not_found() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let req = sample_req(Uuid::new_v4());
    let result = create_po(&pool, TEST_TENANT, &req, "corr-x".to_string()).await;
    assert!(
        matches!(result, Err(PoError::VendorNotFound(_))),
        "expected VendorNotFound, got {:?}",
        result
    );
}

#[tokio::test]
#[serial]
async fn test_get_po_returns_with_lines() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let vendor_id = create_test_vendor(&pool).await;

    let created = create_po(
        &pool,
        TEST_TENANT,
        &sample_req(vendor_id),
        "corr-2".to_string(),
    )
    .await
    .expect("create failed");

    let fetched = get_po(&pool, TEST_TENANT, created.po.po_id)
        .await
        .expect("get_po failed");

    let powi = fetched.expect("PO should be found");
    assert_eq!(powi.po.po_id, created.po.po_id);
    assert_eq!(powi.lines.len(), 2);

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_get_po_wrong_tenant_returns_none() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let vendor_id = create_test_vendor(&pool).await;

    let created = create_po(
        &pool,
        TEST_TENANT,
        &sample_req(vendor_id),
        "corr-3".to_string(),
    )
    .await
    .expect("create failed");

    let result = get_po(&pool, "other-tenant", created.po.po_id)
        .await
        .expect("get_po error");
    assert!(result.is_none());

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_update_po_lines_replaces_all_idempotent() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let vendor_id = create_test_vendor(&pool).await;

    let created = create_po(
        &pool,
        TEST_TENANT,
        &sample_req(vendor_id),
        "corr-4".to_string(),
    )
    .await
    .expect("create failed");

    let update_req = UpdatePoLinesRequest {
        updated_by: "user-ap".to_string(),
        lines: vec![CreatePoLineRequest {
            item_id: None,
            description: Some("Replacement item".to_string()),
            quantity: 2.0,
            unit_of_measure: "each".to_string(),
            unit_price_minor: 20_000,
            gl_account_code: "6300".to_string(),
        }],
    };

    let updated = update_po_lines(&pool, TEST_TENANT, created.po.po_id, &update_req)
        .await
        .expect("update_po_lines failed");

    assert_eq!(updated.lines.len(), 1);
    assert_eq!(updated.po.total_minor, 40_000); // 2 * 20000
    assert_eq!(updated.lines[0].description, "Replacement item");

    // Calling again with same request is idempotent
    let updated2 = update_po_lines(&pool, TEST_TENANT, created.po.po_id, &update_req)
        .await
        .expect("second update failed");
    assert_eq!(updated2.lines.len(), 1);
    assert_eq!(updated2.po.total_minor, 40_000);

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_update_po_lines_rejected_for_non_draft() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let vendor_id = create_test_vendor(&pool).await;

    let created = create_po(
        &pool,
        TEST_TENANT,
        &sample_req(vendor_id),
        "corr-5".to_string(),
    )
    .await
    .expect("create failed");

    // Manually advance status to approved
    sqlx::query("UPDATE purchase_orders SET status = 'approved' WHERE po_id = $1")
        .bind(created.po.po_id)
        .execute(&pool)
        .await
        .expect("status update failed");

    let update_req = UpdatePoLinesRequest {
        updated_by: "user-ap".to_string(),
        lines: vec![CreatePoLineRequest {
            item_id: None,
            description: Some("New item".to_string()),
            quantity: 1.0,
            unit_of_measure: "each".to_string(),
            unit_price_minor: 5_000,
            gl_account_code: "6100".to_string(),
        }],
    };

    let result = update_po_lines(&pool, TEST_TENANT, created.po.po_id, &update_req).await;
    assert!(
        matches!(result, Err(PoError::NotDraft(_))),
        "expected NotDraft, got {:?}",
        result
    );

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_po_item_id_roundtrip() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let vendor_id = create_test_vendor(&pool).await;

    let known_item_id = Uuid::new_v4();
    let req = CreatePoRequest {
        vendor_id,
        currency: "USD".to_string(),
        created_by: "user-ap".to_string(),
        expected_delivery_date: None,
        lines: vec![
            // Line with item_id and no description
            CreatePoLineRequest {
                item_id: Some(known_item_id),
                description: None,
                quantity: 3.0,
                unit_of_measure: "ea".to_string(),
                unit_price_minor: 5_000,
                gl_account_code: "6100".to_string(),
            },
            // Line with both item_id and description — description kept, item_id persisted
            CreatePoLineRequest {
                item_id: Some(Uuid::new_v4()),
                description: Some("Custom probe".to_string()),
                quantity: 1.0,
                unit_of_measure: "ea".to_string(),
                unit_price_minor: 2_000,
                gl_account_code: "6200".to_string(),
            },
            // Description-only line — item_id stays None
            CreatePoLineRequest {
                item_id: None,
                description: Some("Misc supplies".to_string()),
                quantity: 10.0,
                unit_of_measure: "ea".to_string(),
                unit_price_minor: 100,
                gl_account_code: "6300".to_string(),
            },
        ],
    };

    let result = create_po(&pool, TEST_TENANT, &req, "corr-rt".to_string())
        .await
        .expect("create_po failed");

    // Line 0: item_id round-trips; description is empty (not "item:{uuid}")
    assert_eq!(result.lines[0].item_id, Some(known_item_id));
    assert!(
        !result.lines[0].description.starts_with("item:"),
        "item_id must not be encoded into description"
    );

    // Line 1: both item_id and description preserved
    assert!(result.lines[1].item_id.is_some());
    assert_eq!(result.lines[1].description, "Custom probe");

    // Line 2: description-only path unaffected
    assert!(result.lines[2].item_id.is_none());
    assert_eq!(result.lines[2].description, "Misc supplies");

    // Verify item_id also present in outbox event payload
    let row: (serde_json::Value,) = sqlx::query_as(
        "SELECT payload FROM events_outbox WHERE aggregate_type = 'po' AND aggregate_id = $1",
    )
    .bind(result.po.po_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("outbox query failed");

    let event_lines = &row.0["payload"]["lines"];
    assert_eq!(
        event_lines[0]["item_id"]
            .as_str()
            .and_then(|s| s.parse::<Uuid>().ok()),
        Some(known_item_id),
        "ap.po_created event must carry item_id on each line"
    );
    assert!(
        event_lines[2]["item_id"].is_null(),
        "description-only line must have null item_id in event"
    );

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_po_created_event_enqueued_in_outbox() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let vendor_id = create_test_vendor(&pool).await;

    let created = create_po(
        &pool,
        TEST_TENANT,
        &sample_req(vendor_id),
        "corr-outbox".to_string(),
    )
    .await
    .expect("create failed");

    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox WHERE aggregate_type = 'po' AND aggregate_id = $1",
    )
    .bind(created.po.po_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("outbox query failed");

    assert!(count.0 >= 1, "expected >=1 outbox event, got {}", count.0);

    cleanup(&pool).await;
}
