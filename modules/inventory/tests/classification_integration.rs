//! Integration tests for item classifications and commodity codes (bd-2h1ng).
//!
//! Tests run against a real PostgreSQL database on port 5442.
//! No mocks, no stubs — all real Postgres.
//!
//! Required test categories:
//! 1. Happy path E2E: assign classification + commodity code, query back
//! 2. Tenant isolation: tenant_A assignment invisible to tenant_B
//! 3. Idempotency: same key returns stored result, no duplicates
//! 4. Outbox verification: event row exists after assignment
//! 5. Query/filter: filter items by classification

use inventory_rs::domain::{
    classifications::{
        assign_classification, list_classifications, list_items_by_classification,
        AssignClassificationRequest, ClassificationError,
    },
    items::{CreateItemRequest, ItemRepo, TrackingMode},
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// Test DB helpers
// ============================================================================

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://inventory_user:inventory_pass@localhost:5442/inventory_db".to_string());

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to inventory test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run inventory migrations");

    pool
}

fn make_item(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: "Test Widget".to_string(),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
        make_buy: None,
    }
}

fn make_assign(
    tenant_id: &str,
    item_id: Uuid,
    system: &str,
    code: &str,
    idem: &str,
) -> AssignClassificationRequest {
    AssignClassificationRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        revision_id: None,
        classification_system: system.to_string(),
        classification_code: code.to_string(),
        classification_label: Some(format!("Label for {}", code)),
        commodity_system: Some("UNSPSC".to_string()),
        commodity_code: Some("31162800".to_string()),
        assigned_by: "test-user".to_string(),
        idempotency_key: idem.to_string(),
        correlation_id: Some("corr-test".to_string()),
        causation_id: None,
    }
}

async fn cleanup(pool: &sqlx::PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM inv_outbox WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inv_idempotency_keys WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_change_history WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_classifications WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_revisions WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM items WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// 1. Happy path E2E: assign classification + commodity code, query back
// ============================================================================

#[tokio::test]
#[serial]
async fn classification_happy_path_e2e() {
    let pool = setup_db().await;
    let tenant = format!("test-cls-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &make_item(&tenant, "SKU-CLS-001"))
        .await
        .expect("create item");

    // Assign classification with commodity code
    let idem = format!("idem-{}", Uuid::new_v4());
    let req = make_assign(&tenant, item.id, "product_line", "aerospace", &idem);
    let (cls, is_replay) = assign_classification(&pool, &req)
        .await
        .expect("assign classification");

    assert!(!is_replay);
    assert_eq!(cls.tenant_id, tenant);
    assert_eq!(cls.item_id, item.id);
    assert_eq!(cls.classification_system, "product_line");
    assert_eq!(cls.classification_code, "aerospace");
    assert_eq!(cls.classification_label.as_deref(), Some("Label for aerospace"));
    assert_eq!(cls.commodity_system.as_deref(), Some("UNSPSC"));
    assert_eq!(cls.commodity_code.as_deref(), Some("31162800"));
    assert_eq!(cls.assigned_by, "test-user");

    // Query back
    let all = list_classifications(&pool, &tenant, item.id)
        .await
        .expect("list classifications");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, cls.id);
    assert_eq!(all[0].classification_system, "product_line");
    assert_eq!(all[0].classification_code, "aerospace");

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 2. Tenant isolation: tenant_A assignment invisible to tenant_B
// ============================================================================

#[tokio::test]
#[serial]
async fn classification_tenant_isolation() {
    let pool = setup_db().await;
    let tenant_a = format!("test-a-{}", Uuid::new_v4());
    let tenant_b = format!("test-b-{}", Uuid::new_v4());

    let item_a = ItemRepo::create(&pool, &make_item(&tenant_a, "SKU-ISO-CLS"))
        .await
        .expect("create item A");
    let item_b = ItemRepo::create(&pool, &make_item(&tenant_b, "SKU-ISO-CLS"))
        .await
        .expect("create item B");

    // Assign classification to tenant A's item
    let idem_a = format!("idem-{}", Uuid::new_v4());
    assign_classification(
        &pool,
        &make_assign(&tenant_a, item_a.id, "department", "engineering", &idem_a),
    )
    .await
    .expect("assign to A");

    // Tenant B querying A's item should see nothing
    let b_sees_a = list_classifications(&pool, &tenant_b, item_a.id)
        .await
        .expect("query cross-tenant");
    assert!(
        b_sees_a.is_empty(),
        "tenant B must not see tenant A's classifications"
    );

    // Tenant B querying their own item should see nothing (no assignment made)
    let b_sees_own = list_classifications(&pool, &tenant_b, item_b.id)
        .await
        .expect("query own");
    assert!(
        b_sees_own.is_empty(),
        "tenant B has no classifications yet"
    );

    // Filter by classification: tenant B should find zero results
    let b_filter = list_items_by_classification(&pool, &tenant_b, "department", "engineering")
        .await
        .expect("filter cross-tenant");
    assert!(
        b_filter.is_empty(),
        "tenant B must not find A's items by classification"
    );

    // Filter by classification: tenant A should find their item
    let a_filter = list_items_by_classification(&pool, &tenant_a, "department", "engineering")
        .await
        .expect("filter own");
    assert_eq!(a_filter.len(), 1);
    assert_eq!(a_filter[0].item_id, item_a.id);

    cleanup(&pool, &tenant_a).await;
    cleanup(&pool, &tenant_b).await;
}

// ============================================================================
// 3. Idempotency: same key returns stored result, no duplicates
// ============================================================================

#[tokio::test]
#[serial]
async fn classification_idempotency() {
    let pool = setup_db().await;
    let tenant = format!("test-idem-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &make_item(&tenant, "SKU-IDEM-CLS"))
        .await
        .expect("create item");

    let idem = format!("idem-{}", Uuid::new_v4());
    let req = make_assign(&tenant, item.id, "NAICS", "332722", &idem);

    // First call
    let (cls1, replay1) = assign_classification(&pool, &req)
        .await
        .expect("first assign");
    assert!(!replay1, "first call is not a replay");

    // Second call with same idempotency key and same body
    let (cls2, replay2) = assign_classification(&pool, &req)
        .await
        .expect("second assign");
    assert!(replay2, "second call must be a replay");
    assert_eq!(cls1.id, cls2.id, "replay must return same record");

    // Only one row in the table
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM item_classifications WHERE tenant_id = $1 AND item_id = $2",
    )
    .bind(&tenant)
    .bind(item.id)
    .fetch_one(&pool)
    .await
    .expect("count query");
    assert_eq!(count, 1, "idempotent replay must not create duplicates");

    // Conflicting body with same key must fail
    let mut conflicting = make_assign(&tenant, item.id, "NAICS", "999999", &idem);
    conflicting.classification_label = Some("Different label".to_string());
    let err = assign_classification(&pool, &conflicting)
        .await
        .expect_err("conflicting payload must fail");
    assert!(
        matches!(err, ClassificationError::ConflictingIdempotencyKey),
        "expected ConflictingIdempotencyKey, got: {:?}",
        err
    );

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 4. Outbox verification: event row exists after assignment
// ============================================================================

#[tokio::test]
#[serial]
async fn classification_outbox_verification() {
    let pool = setup_db().await;
    let tenant = format!("test-outbox-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &make_item(&tenant, "SKU-OUTBOX-CLS"))
        .await
        .expect("create item");

    let idem = format!("idem-{}", Uuid::new_v4());
    assign_classification(
        &pool,
        &make_assign(&tenant, item.id, "HS", "7318150000", &idem),
    )
    .await
    .expect("assign classification");

    // Query the outbox directly
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inv_outbox WHERE tenant_id = $1 AND event_type = 'inventory.classification_assigned.v1'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("outbox query");
    assert_eq!(
        outbox_count, 1,
        "exactly one outbox event for classification assignment"
    );

    // Verify event has correct tenant_id
    let outbox_tenant: String = sqlx::query_scalar(
        "SELECT tenant_id FROM inv_outbox WHERE tenant_id = $1 AND event_type = 'inventory.classification_assigned.v1'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("outbox tenant query");
    assert_eq!(outbox_tenant, tenant);

    // Verify change history was also recorded
    let history_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM item_change_history WHERE tenant_id = $1 AND change_type = 'classification_assigned'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("history query");
    assert_eq!(
        history_count, 1,
        "change history must record the classification assignment"
    );

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 5. Query/filter: items with different classifications filter correctly
// ============================================================================

#[tokio::test]
#[serial]
async fn classification_query_filter() {
    let pool = setup_db().await;
    let tenant = format!("test-filter-{}", Uuid::new_v4());

    // Create three items
    let item1 = ItemRepo::create(&pool, &make_item(&tenant, "SKU-FILT-001"))
        .await
        .expect("create item 1");
    let item2 = ItemRepo::create(&pool, &make_item(&tenant, "SKU-FILT-002"))
        .await
        .expect("create item 2");
    let item3 = ItemRepo::create(&pool, &make_item(&tenant, "SKU-FILT-003"))
        .await
        .expect("create item 3");

    // Assign: item1 → aerospace, item2 → aerospace, item3 → automotive
    assign_classification(
        &pool,
        &make_assign(
            &tenant,
            item1.id,
            "product_line",
            "aerospace",
            &format!("idem-{}", Uuid::new_v4()),
        ),
    )
    .await
    .expect("assign item1");

    assign_classification(
        &pool,
        &make_assign(
            &tenant,
            item2.id,
            "product_line",
            "aerospace",
            &format!("idem-{}", Uuid::new_v4()),
        ),
    )
    .await
    .expect("assign item2");

    assign_classification(
        &pool,
        &make_assign(
            &tenant,
            item3.id,
            "product_line",
            "automotive",
            &format!("idem-{}", Uuid::new_v4()),
        ),
    )
    .await
    .expect("assign item3");

    // Filter for aerospace — should return items 1 and 2
    let aerospace = list_items_by_classification(&pool, &tenant, "product_line", "aerospace")
        .await
        .expect("filter aerospace");
    assert_eq!(aerospace.len(), 2, "two items in aerospace");
    let aerospace_ids: Vec<Uuid> = aerospace.iter().map(|c| c.item_id).collect();
    assert!(aerospace_ids.contains(&item1.id));
    assert!(aerospace_ids.contains(&item2.id));
    assert!(!aerospace_ids.contains(&item3.id));

    // Filter for automotive — should return only item 3
    let automotive = list_items_by_classification(&pool, &tenant, "product_line", "automotive")
        .await
        .expect("filter automotive");
    assert_eq!(automotive.len(), 1, "one item in automotive");
    assert_eq!(automotive[0].item_id, item3.id);

    // Filter for nonexistent classification — empty
    let none = list_items_by_classification(&pool, &tenant, "product_line", "marine")
        .await
        .expect("filter marine");
    assert!(none.is_empty(), "no items in marine");

    // Each item can have multiple classifications
    assign_classification(
        &pool,
        &make_assign(
            &tenant,
            item1.id,
            "ECCN",
            "EAR99",
            &format!("idem-{}", Uuid::new_v4()),
        ),
    )
    .await
    .expect("assign ECCN to item1");

    let item1_cls = list_classifications(&pool, &tenant, item1.id)
        .await
        .expect("list item1 classifications");
    assert_eq!(
        item1_cls.len(),
        2,
        "item1 should have 2 classifications (product_line + ECCN)"
    );

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 6. Duplicate assignment rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn classification_duplicate_assignment_rejected() {
    let pool = setup_db().await;
    let tenant = format!("test-dup-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &make_item(&tenant, "SKU-DUP-CLS"))
        .await
        .expect("create item");

    // First assignment
    let idem1 = format!("idem-{}", Uuid::new_v4());
    assign_classification(
        &pool,
        &make_assign(&tenant, item.id, "department", "QA", &idem1),
    )
    .await
    .expect("first assign");

    // Same classification_system + classification_code, different idempotency key
    let idem2 = format!("idem-{}", Uuid::new_v4());
    let err = assign_classification(
        &pool,
        &make_assign(&tenant, item.id, "department", "QA", &idem2),
    )
    .await
    .expect_err("duplicate assignment must fail");

    assert!(
        matches!(err, ClassificationError::DuplicateAssignment),
        "expected DuplicateAssignment, got: {:?}",
        err
    );

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 7. Classification without commodity code (classification-only)
// ============================================================================

#[tokio::test]
#[serial]
async fn classification_without_commodity_code() {
    let pool = setup_db().await;
    let tenant = format!("test-nocmdy-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &make_item(&tenant, "SKU-NOCMDY"))
        .await
        .expect("create item");

    let idem = format!("idem-{}", Uuid::new_v4());
    let req = AssignClassificationRequest {
        tenant_id: tenant.clone(),
        item_id: item.id,
        revision_id: None,
        classification_system: "internal".to_string(),
        classification_code: "raw_material".to_string(),
        classification_label: Some("Raw Material".to_string()),
        commodity_system: None,
        commodity_code: None,
        assigned_by: "admin".to_string(),
        idempotency_key: idem,
        correlation_id: None,
        causation_id: None,
    };

    let (cls, _) = assign_classification(&pool, &req)
        .await
        .expect("assign classification only");
    assert!(cls.commodity_system.is_none());
    assert!(cls.commodity_code.is_none());
    assert_eq!(cls.classification_system, "internal");
    assert_eq!(cls.classification_code, "raw_material");

    cleanup(&pool, &tenant).await;
}
