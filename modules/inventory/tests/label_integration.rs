//! Integration tests for barcode/label generation (bd-3o5mg).
//!
//! Tests run against a real PostgreSQL database.
//! Set DATABASE_URL to the inventory database connection string.
//!
//! Coverage:
//! 1. Happy path: generate label from revision context
//! 2. Determinism: same inputs produce same payload
//! 3. Idempotency: duplicate key returns stored result
//! 4. Idempotency conflict: same key, different body rejected
//! 5. Tenant isolation: labels scoped per tenant
//! 6. Event emission: outbox row written atomically
//! 7. Guard: inactive item rejected
//! 8. Guard: mismatched revision rejected
//! 9. Lot label with extra data

use inventory_rs::domain::{
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    labels::{generate_label, get_label, list_labels, GenerateLabelRequest, LabelError},
    revisions::{create_revision, CreateRevisionRequest},
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
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://inventory_user:inventory_pass@localhost:5442/inventory_db?sslmode=disable".to_string());

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

fn make_revision(tenant_id: &str, item_id: Uuid) -> CreateRevisionRequest {
    CreateRevisionRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        name: "Widget Rev".to_string(),
        description: Some("Updated specifications".to_string()),
        uom: "ea".to_string(),
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        traceability_level: "none".to_string(),
        inspection_required: false,
        shelf_life_days: None,
        shelf_life_enforced: false,
        change_reason: "Spec update".to_string(),
        idempotency_key: format!("idem-rev-{}", Uuid::new_v4()),
        correlation_id: Some("corr-test".to_string()),
        causation_id: None,
        actor_id: None,
    }
}

fn make_label_req(
    tenant_id: &str,
    item_id: Uuid,
    revision_id: Uuid,
    idem: &str,
) -> GenerateLabelRequest {
    GenerateLabelRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        revision_id,
        label_type: "item_label".to_string(),
        barcode_format: "code128".to_string(),
        extra: None,
        idempotency_key: idem.to_string(),
        actor_id: Some(Uuid::new_v4()),
        correlation_id: Some("corr-label".to_string()),
        causation_id: None,
    }
}

async fn cleanup(pool: &sqlx::PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM inv_labels WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
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

/// Helper: create item + revision, return (item_id, revision_id, sku)
async fn create_item_with_revision(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    sku: &str,
) -> (Uuid, Uuid, String) {
    let item = ItemRepo::create(pool, &make_item(tenant_id, sku))
        .await
        .expect("create item");
    let (rev, _) = create_revision(pool, &make_revision(tenant_id, item.id))
        .await
        .expect("create revision");
    (item.id, rev.id, item.sku)
}

// ============================================================================
// 1. Happy path: generate label from revision context
// ============================================================================

#[tokio::test]
#[serial]
async fn label_generate_happy_path() {
    let pool = setup_db().await;
    let tenant = format!("test-lbl-{}", Uuid::new_v4());

    let (item_id, rev_id, _sku) = create_item_with_revision(&pool, &tenant, "SKU-LBL-001").await;

    let idem = format!("idem-{}", Uuid::new_v4());
    let req = make_label_req(&tenant, item_id, rev_id, &idem);
    let (label, is_replay) = generate_label(&pool, &req).await.expect("generate label");

    assert!(!is_replay);
    assert_eq!(label.tenant_id, tenant);
    assert_eq!(label.item_id, item_id);
    assert_eq!(label.revision_id, rev_id);
    assert_eq!(label.label_type, "item_label");
    assert_eq!(label.barcode_format, "code128");
    assert!(label.actor_id.is_some());

    // Payload should contain expected fields
    let p = &label.payload;
    assert_eq!(p["item_sku"], "SKU-LBL-001");
    assert_eq!(p["item_name"], "Widget Rev");
    assert_eq!(p["uom"], "ea");
    assert_eq!(p["revision_number"], 1);
    assert_eq!(p["barcode_value"], "SKU-LBL-001-R1");
    assert_eq!(p["label_type"], "item_label");

    // Should be fetchable by ID
    let fetched = get_label(&pool, &tenant, label.id)
        .await
        .expect("get label")
        .expect("label should exist");
    assert_eq!(fetched.id, label.id);
    assert_eq!(fetched.payload, label.payload);

    // Should appear in list
    let labels = list_labels(&pool, &tenant, item_id)
        .await
        .expect("list labels");
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0].id, label.id);

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 2. Determinism: same inputs produce same payload
// ============================================================================

#[tokio::test]
#[serial]
async fn label_payload_is_deterministic() {
    let pool = setup_db().await;
    let tenant = format!("test-lbl-{}", Uuid::new_v4());

    let (item_id, rev_id, _) = create_item_with_revision(&pool, &tenant, "SKU-LBL-DET").await;

    // Generate two labels with different idempotency keys but same context
    let idem1 = format!("idem-{}", Uuid::new_v4());
    let idem2 = format!("idem-{}", Uuid::new_v4());

    let req1 = make_label_req(&tenant, item_id, rev_id, &idem1);
    let mut req2 = make_label_req(&tenant, item_id, rev_id, &idem2);
    req2.actor_id = req1.actor_id; // same actor for determinism check

    let (label1, _) = generate_label(&pool, &req1).await.expect("label 1");
    let (label2, _) = generate_label(&pool, &req2).await.expect("label 2");

    // Different label IDs but same payload content
    assert_ne!(label1.id, label2.id);
    assert_eq!(
        label1.payload, label2.payload,
        "same inputs must produce same payload"
    );

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 3. Idempotency: duplicate key returns stored result
// ============================================================================

#[tokio::test]
#[serial]
async fn label_idempotent_replay() {
    let pool = setup_db().await;
    let tenant = format!("test-lbl-{}", Uuid::new_v4());

    let (item_id, rev_id, _) = create_item_with_revision(&pool, &tenant, "SKU-LBL-IDEM").await;

    let idem = format!("idem-{}", Uuid::new_v4());
    let req = make_label_req(&tenant, item_id, rev_id, &idem);

    // First call
    let (label1, replay1) = generate_label(&pool, &req).await.expect("first generate");
    assert!(!replay1);

    // Second call with same key and body
    let (label2, replay2) = generate_label(&pool, &req).await.expect("second generate");
    assert!(replay2, "second call must be a replay");
    assert_eq!(label1.id, label2.id);
    assert_eq!(label1.payload, label2.payload);

    // Only one label row in DB
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM inv_labels WHERE tenant_id = $1 AND item_id = $2")
            .bind(&tenant)
            .bind(item_id)
            .fetch_one(&pool)
            .await
            .expect("count query");
    assert_eq!(count, 1);

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 4. Idempotency conflict: same key, different body rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn label_idempotency_conflict() {
    let pool = setup_db().await;
    let tenant = format!("test-lbl-{}", Uuid::new_v4());

    let (item_id, rev_id, _) = create_item_with_revision(&pool, &tenant, "SKU-LBL-CONF").await;

    let idem = format!("idem-{}", Uuid::new_v4());
    let req1 = make_label_req(&tenant, item_id, rev_id, &idem);
    generate_label(&pool, &req1).await.expect("first generate");

    // Same key, different label_type
    let mut req2 = make_label_req(&tenant, item_id, rev_id, &idem);
    req2.label_type = "lot_label".to_string();

    let err = generate_label(&pool, &req2)
        .await
        .expect_err("conflict must fail");
    assert!(
        matches!(err, LabelError::ConflictingIdempotencyKey),
        "expected ConflictingIdempotencyKey, got: {:?}",
        err
    );

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 5. Tenant isolation: labels scoped per tenant
// ============================================================================

#[tokio::test]
#[serial]
async fn label_tenant_isolation() {
    let pool = setup_db().await;
    let tenant_a = format!("test-lbl-a-{}", Uuid::new_v4());
    let tenant_b = format!("test-lbl-b-{}", Uuid::new_v4());

    let (item_a, rev_a, _) = create_item_with_revision(&pool, &tenant_a, "SKU-ISO-LBL").await;
    let (item_b, _, _) = create_item_with_revision(&pool, &tenant_b, "SKU-ISO-LBL").await;

    // Generate label for tenant A
    let idem = format!("idem-{}", Uuid::new_v4());
    let req = make_label_req(&tenant_a, item_a, rev_a, &idem);
    let (label_a, _) = generate_label(&pool, &req).await.expect("generate for A");

    // Tenant B should see no labels for their item
    let labels_b = list_labels(&pool, &tenant_b, item_b)
        .await
        .expect("list for B");
    assert!(labels_b.is_empty(), "tenant B should have no labels");

    // Tenant B cannot fetch tenant A's label
    let cross = get_label(&pool, &tenant_b, label_a.id)
        .await
        .expect("cross-tenant get");
    assert!(cross.is_none(), "cross-tenant fetch must return nothing");

    cleanup(&pool, &tenant_a).await;
    cleanup(&pool, &tenant_b).await;
}

// ============================================================================
// 6. Event emission: outbox row written atomically
// ============================================================================

#[tokio::test]
#[serial]
async fn label_event_emitted_to_outbox() {
    let pool = setup_db().await;
    let tenant = format!("test-lbl-{}", Uuid::new_v4());

    let (item_id, rev_id, _) = create_item_with_revision(&pool, &tenant, "SKU-LBL-EVT").await;

    let idem = format!("idem-{}", Uuid::new_v4());
    let req = make_label_req(&tenant, item_id, rev_id, &idem);
    let (label, _) = generate_label(&pool, &req).await.expect("generate label");

    // Check outbox event
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inv_outbox WHERE tenant_id = $1 AND event_type = 'inventory.label_generated.v1'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("outbox count");
    assert_eq!(outbox_count, 1);

    // Verify the outbox payload contains our label_id
    let payload_json: String = sqlx::query_scalar(
        "SELECT payload::TEXT FROM inv_outbox WHERE tenant_id = $1 AND event_type = 'inventory.label_generated.v1'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("outbox payload");

    let envelope: serde_json::Value = serde_json::from_str(&payload_json).expect("parse envelope");
    assert_eq!(envelope["payload"]["label_id"], label.id.to_string());
    assert_eq!(envelope["event_type"], "inventory.label_generated.v1");
    assert_eq!(envelope["source_module"], "inventory");

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 7. Guard: inactive item rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn label_guard_rejects_inactive_item() {
    let pool = setup_db().await;
    let tenant = format!("test-lbl-{}", Uuid::new_v4());

    let (item_id, rev_id, _) = create_item_with_revision(&pool, &tenant, "SKU-LBL-INACT").await;

    ItemRepo::deactivate(&pool, item_id, &tenant)
        .await
        .expect("deactivate");

    let idem = format!("idem-{}", Uuid::new_v4());
    let req = make_label_req(&tenant, item_id, rev_id, &idem);
    let err = generate_label(&pool, &req)
        .await
        .expect_err("inactive item must fail");

    assert!(
        matches!(err, LabelError::ItemInactive),
        "expected ItemInactive, got: {:?}",
        err
    );

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 8. Guard: mismatched revision rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn label_guard_rejects_mismatched_revision() {
    let pool = setup_db().await;
    let tenant = format!("test-lbl-{}", Uuid::new_v4());

    let (item_a, _, _) = create_item_with_revision(&pool, &tenant, "SKU-LBL-MIS-A").await;
    let (_, rev_b, _) = create_item_with_revision(&pool, &tenant, "SKU-LBL-MIS-B").await;

    // Try to generate label for item_a using item_b's revision
    let idem = format!("idem-{}", Uuid::new_v4());
    let req = make_label_req(&tenant, item_a, rev_b, &idem);
    let err = generate_label(&pool, &req)
        .await
        .expect_err("mismatched revision must fail");

    assert!(
        matches!(err, LabelError::RevisionNotFound),
        "expected RevisionNotFound, got: {:?}",
        err
    );

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 9. Lot label with extra data
// ============================================================================

#[tokio::test]
#[serial]
async fn label_lot_with_extra_data() {
    let pool = setup_db().await;
    let tenant = format!("test-lbl-{}", Uuid::new_v4());

    let (item_id, rev_id, _) = create_item_with_revision(&pool, &tenant, "SKU-LBL-LOT").await;

    let idem = format!("idem-{}", Uuid::new_v4());
    let mut req = make_label_req(&tenant, item_id, rev_id, &idem);
    req.label_type = "lot_label".to_string();
    req.extra = Some(serde_json::json!({
        "lot_code": "LOT-2026-001",
        "quantity": 50,
        "expiry_date": "2026-09-01"
    }));

    let (label, _) = generate_label(&pool, &req)
        .await
        .expect("generate lot label");

    assert_eq!(label.label_type, "lot_label");
    assert_eq!(label.payload["lot_code"], "LOT-2026-001");
    assert_eq!(label.payload["quantity"], 50);
    assert_eq!(label.payload["expiry_date"], "2026-09-01");
    // Standard fields still present
    assert_eq!(label.payload["label_type"], "lot_label");
    assert!(label.payload["barcode_value"].as_str().is_some());

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 10. Multiple labels for same item
// ============================================================================

#[tokio::test]
#[serial]
async fn label_multiple_for_same_item() {
    let pool = setup_db().await;
    let tenant = format!("test-lbl-{}", Uuid::new_v4());

    let (item_id, rev_id, _) = create_item_with_revision(&pool, &tenant, "SKU-LBL-MULTI").await;

    // Generate three labels with different idempotency keys
    for i in 0..3 {
        let idem = format!("idem-multi-{}-{}", i, Uuid::new_v4());
        let mut req = make_label_req(&tenant, item_id, rev_id, &idem);
        if i == 1 {
            req.label_type = "lot_label".to_string();
        }
        if i == 2 {
            req.barcode_format = "qr".to_string();
        }
        generate_label(&pool, &req).await.expect("generate");
    }

    let labels = list_labels(&pool, &tenant, item_id)
        .await
        .expect("list labels");
    assert_eq!(labels.len(), 3);

    cleanup(&pool, &tenant).await;
}
