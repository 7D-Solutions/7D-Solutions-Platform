//! E2E Integration Proof: Manufacturing Phase A
//!
//! Proves the Phase A "Prove at end" bullets from the manufacturing roadmap:
//! 1. Create part + BOM revision + effectivity → query where-used and explosion
//! 2. Inventory: production receipt with source_type=production + caller-provided unit cost
//! 3. Inventory: issue with source_type tagging (source_type in SourceRef)
//! 4. Inventory: purchase receipt path unchanged (regression)
//! 5. Events emitted with correct EventEnvelope metadata, replay-safe
//!
//! All tests use real Postgres (BOM DB port 5450, Inventory DB port 5442).
//! No mocks, no stubs.

use chrono::Utc;
use inventory_rs::domain::{
    issue_service::{process_issue, IssueRequest},
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    receipt_service::{process_receipt, ReceiptRequest},
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

use bom_rs::domain::{bom_queries, bom_service, models::*};

// ============================================================================
// DB setup
// ============================================================================

async fn get_bom_pool() -> PgPool {
    let url = std::env::var("BOM_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://bom_user:bom_pass@localhost:5450/bom_db".to_string());
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&url)
        .await
        .expect("Failed to connect to BOM DB");

    // Run migration SQL idempotently
    let migration_sql =
        include_str!("../../modules/bom/db/migrations/20260305000001_create_bom_schema.sql");
    // Wrap in DO block to ignore "already exists" errors
    for statement in migration_sql.split(';') {
        let trimmed = statement.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Silently skip if object already exists
        if let Err(e) = sqlx::query(trimmed).execute(&pool).await {
            let msg = e.to_string();
            if !msg.contains("already exists") && !msg.contains("duplicate key") {
                // Log but don't panic — might be extension already loaded
                tracing::warn!("BOM migration statement warning: {}", msg);
            }
        }
    }

    pool
}

async fn get_inventory_pool() -> PgPool {
    let url = std::env::var("INVENTORY_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://inventory_user:inventory_pass@localhost:5442/inventory_db".to_string()
        });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&url)
        .await
        .expect("Failed to connect to Inventory DB");
    sqlx::migrate!("../modules/inventory/db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run Inventory migrations");
    pool
}

// ============================================================================
// Cleanup
// ============================================================================

async fn cleanup_bom_tenant(pool: &PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM bom_outbox WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM bom_lines WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM bom_revisions WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM bom_headers WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

async fn cleanup_inventory_tenant(pool: &PgPool, tenant_id: &str) {
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
    sqlx::query("DELETE FROM inventory_layers WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_ledger WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_on_hand WHERE tenant_id = $1")
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
// Helpers
// ============================================================================

fn test_tenant() -> String {
    format!("mfg-phase-a-{}", Uuid::new_v4())
}

fn create_item_req(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: format!("Phase A Test: {}", sku),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
        make_buy: None,
    }
}

fn purchase_receipt(tenant_id: &str, item_id: Uuid, qty: i64, cost: i64) -> ReceiptRequest {
    ReceiptRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id: Uuid::new_v4(),
        location_id: None,
        quantity: qty,
        unit_cost_minor: cost,
        currency: "usd".to_string(),
        source_type: "purchase".to_string(),
        purchase_order_id: Some(Uuid::new_v4()),
        idempotency_key: format!("pur-{}", Uuid::new_v4()),
        correlation_id: Some("phase-a-test".to_string()),
        causation_id: None,
        lot_code: None,
        serial_codes: None,
        uom_id: None,
    }
}

fn production_receipt(
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    qty: i64,
    cost: i64,
) -> ReceiptRequest {
    ReceiptRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        location_id: None,
        quantity: qty,
        unit_cost_minor: cost,
        currency: "usd".to_string(),
        source_type: "production".to_string(),
        purchase_order_id: None,
        idempotency_key: format!("prod-{}", Uuid::new_v4()),
        correlation_id: Some("phase-a-test".to_string()),
        causation_id: None,
        lot_code: None,
        serial_codes: None,
        uom_id: None,
    }
}

// ============================================================================
// Test 1: BOM structure + effectivity + where-used + explosion
// ============================================================================

#[tokio::test]
#[serial]
async fn bom_revision_effectivity_whereused_explosion() {
    let bom_pool = get_bom_pool().await;
    let tenant_id = test_tenant();

    // Assembly part and two component parts (UUIDs stand in for item IDs)
    let assembly_part = Uuid::new_v4();
    let component_a = Uuid::new_v4();
    let component_b = Uuid::new_v4();

    // Create BOM for the assembly
    let bom = bom_service::create_bom(
        &bom_pool,
        &tenant_id,
        &CreateBomRequest {
            part_id: assembly_part,
            description: Some("Test assembly".to_string()),
        },
        "corr-bom-test",
        None,
    )
    .await
    .expect("create BOM");

    assert_eq!(bom.part_id, assembly_part);
    assert_eq!(bom.tenant_id, tenant_id);

    // Create revision
    let rev = bom_service::create_revision(
        &bom_pool,
        &tenant_id,
        bom.id,
        &CreateRevisionRequest {
            revision_label: "Rev A".to_string(),
        },
        "corr-bom-test",
        None,
    )
    .await
    .expect("create revision");

    assert_eq!(rev.status, "draft");

    // Add component lines
    let line_a = bom_service::add_line(
        &bom_pool,
        &tenant_id,
        rev.id,
        &AddLineRequest {
            component_item_id: component_a,
            quantity: 2.0,
            uom: Some("EA".to_string()),
            scrap_factor: Some(0.05),
            find_number: Some(10),
        },
        "corr-bom-test",
        None,
    )
    .await
    .expect("add line A");

    let line_b = bom_service::add_line(
        &bom_pool,
        &tenant_id,
        rev.id,
        &AddLineRequest {
            component_item_id: component_b,
            quantity: 1.0,
            uom: Some("EA".to_string()),
            scrap_factor: None,
            find_number: Some(20),
        },
        "corr-bom-test",
        None,
    )
    .await
    .expect("add line B");

    assert_eq!(line_a.component_item_id, component_a);
    assert_eq!(line_b.component_item_id, component_b);

    // Set effectivity (now → open-ended)
    let now = Utc::now();
    let eff_rev = bom_service::set_effectivity(
        &bom_pool,
        &tenant_id,
        rev.id,
        &SetEffectivityRequest {
            effective_from: now,
            effective_to: None,
        },
        "corr-bom-test",
        None,
    )
    .await
    .expect("set effectivity");

    assert_eq!(eff_rev.status, "effective");

    // Query where-used for component_a
    let wu = bom_queries::where_used(
        &bom_pool,
        &tenant_id,
        component_a,
        &WhereUsedQuery { date: Some(now) },
    )
    .await
    .expect("where-used");

    assert_eq!(wu.len(), 1, "component_a should appear in 1 assembly");
    assert_eq!(wu[0].bom_id, bom.id);
    assert_eq!(wu[0].part_id, assembly_part);
    assert!((wu[0].quantity - 2.0).abs() < f64::EPSILON);

    // Explosion
    let explosion = bom_queries::explode(
        &bom_pool,
        &tenant_id,
        bom.id,
        &ExplosionQuery {
            date: Some(now),
            max_depth: Some(5),
        },
    )
    .await
    .expect("explosion");

    assert_eq!(explosion.len(), 2, "2 components at level 1");
    assert!(explosion.iter().all(|r| r.level == 1));
    let comp_ids: Vec<Uuid> = explosion.iter().map(|r| r.component_item_id).collect();
    assert!(comp_ids.contains(&component_a));
    assert!(comp_ids.contains(&component_b));

    // Verify outbox events written
    let outbox_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM bom_outbox WHERE tenant_id = $1")
            .bind(&tenant_id)
            .fetch_one(&bom_pool)
            .await
            .expect("count outbox");

    // Expected: bom.created + revision_created + line_added x2 + effectivity_set = 5
    assert_eq!(outbox_count, 5, "5 BOM outbox events expected");

    // Verify event envelope metadata
    let (event_type, payload): (String, serde_json::Value) = sqlx::query_as(
        "SELECT event_type, payload FROM bom_outbox WHERE tenant_id = $1 AND event_type = 'bom.created' LIMIT 1",
    )
    .bind(&tenant_id)
    .fetch_one(&bom_pool)
    .await
    .expect("fetch bom.created outbox event");

    assert_eq!(event_type, "bom.created");
    // EventEnvelope fields in payload
    assert_eq!(payload["source_module"], "bom");
    assert_eq!(payload["replay_safe"], true);
    assert!(payload["event_id"].is_string());
    assert!(payload["correlation_id"].is_string());

    cleanup_bom_tenant(&bom_pool, &tenant_id).await;
}

// ============================================================================
// Test 2: Production receipt with source_type=production + caller-provided cost
// ============================================================================

#[tokio::test]
#[serial]
async fn inventory_production_receipt_source_type() {
    let inv_pool = get_inventory_pool().await;
    let tenant_id = test_tenant();

    let item = ItemRepo::create(&inv_pool, &create_item_req(&tenant_id, "FG-PROD-001"))
        .await
        .expect("create item");

    let wh_id = Uuid::new_v4();
    let req = production_receipt(&tenant_id, item.id, wh_id, 50, 10_00);
    let (result, is_replay) = process_receipt(&inv_pool, &req, None)
        .await
        .expect("production receipt");

    assert!(!is_replay);
    assert_eq!(result.quantity, 50);
    assert_eq!(result.unit_cost_minor, 10_00);

    // Verify outbox event carries source_type=production
    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM inv_outbox WHERE tenant_id = $1 AND event_type = 'inventory.item_received' ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&tenant_id)
    .fetch_one(&inv_pool)
    .await
    .expect("fetch outbox event");

    assert_eq!(payload["payload"]["source_type"], "production");
    assert_eq!(payload["source_module"], "inventory");
    assert_eq!(payload["replay_safe"], true);

    cleanup_inventory_tenant(&inv_pool, &tenant_id).await;
}

// ============================================================================
// Test 3: Issue with source_type tagging
// ============================================================================

#[tokio::test]
#[serial]
async fn inventory_issue_source_type_tagged() {
    let inv_pool = get_inventory_pool().await;
    let tenant_id = test_tenant();

    let item = ItemRepo::create(&inv_pool, &create_item_req(&tenant_id, "COMP-ISSUE-001"))
        .await
        .expect("create item");

    let wh_id = Uuid::new_v4();

    // First receive stock so we can issue
    let rcv_req = ReceiptRequest {
        warehouse_id: wh_id,
        ..purchase_receipt(&tenant_id, item.id, 100, 5_00)
    };
    process_receipt(&inv_pool, &rcv_req, None)
        .await
        .expect("receipt for issue test");

    // Issue with source_type tagging
    let issue_req = IssueRequest {
        tenant_id: tenant_id.clone(),
        item_id: item.id,
        warehouse_id: wh_id,
        location_id: None,
        quantity: 10,
        currency: "usd".to_string(),
        source_module: "production".to_string(),
        source_type: "work_order_issue".to_string(),
        source_id: Uuid::new_v4().to_string(),
        source_line_id: None,
        idempotency_key: format!("issue-{}", Uuid::new_v4()),
        correlation_id: Some("phase-a-issue-test".to_string()),
        causation_id: None,
        uom_id: None,
        lot_code: None,
        serial_codes: None,
    };

    let (result, _is_replay) = process_issue(&inv_pool, &issue_req, None)
        .await
        .expect("issue must succeed");

    assert_eq!(result.quantity, 10);
    assert_eq!(result.source_ref.source_type, "work_order_issue");
    assert_eq!(result.source_ref.source_module, "production");

    // Verify outbox event
    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM inv_outbox WHERE tenant_id = $1 AND event_type = 'inventory.item_issued' ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&tenant_id)
    .fetch_one(&inv_pool)
    .await
    .expect("fetch issue outbox event");

    let inner = &payload["payload"];
    assert_eq!(inner["source_ref"]["source_type"], "work_order_issue");
    assert_eq!(inner["source_ref"]["source_module"], "production");
    assert!(inner["consumed_layers"].is_array());
    assert!(!inner["consumed_layers"].as_array().unwrap().is_empty());

    cleanup_inventory_tenant(&inv_pool, &tenant_id).await;
}

// ============================================================================
// Test 4: Purchase receipt regression — unchanged behavior
// ============================================================================

#[tokio::test]
#[serial]
async fn inventory_purchase_receipt_regression() {
    let inv_pool = get_inventory_pool().await;
    let tenant_id = test_tenant();

    let item = ItemRepo::create(&inv_pool, &create_item_req(&tenant_id, "BUY-REG-001"))
        .await
        .expect("create item");

    let req = purchase_receipt(&tenant_id, item.id, 200, 15_00);
    let (result, is_replay) = process_receipt(&inv_pool, &req, None)
        .await
        .expect("purchase receipt");

    assert!(!is_replay);
    assert_eq!(result.quantity, 200);
    assert_eq!(result.unit_cost_minor, 15_00);

    // source_type defaults to "purchase" when not specified
    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM inv_outbox WHERE tenant_id = $1 AND event_type = 'inventory.item_received' ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&tenant_id)
    .fetch_one(&inv_pool)
    .await
    .expect("fetch purchase outbox event");

    assert_eq!(payload["payload"]["source_type"], "purchase");
    assert!(payload["payload"]["purchase_order_id"].is_string());

    // Verify FIFO layer exists
    let (qty_recv, qty_rem): (i64, i64) = sqlx::query_as(
        "SELECT quantity_received, quantity_remaining FROM inventory_layers WHERE id = $1",
    )
    .bind(result.layer_id)
    .fetch_one(&inv_pool)
    .await
    .expect("FIFO layer");

    assert_eq!(qty_recv, 200);
    assert_eq!(qty_rem, 200);

    // Idempotency: replay returns same result
    let (replay_result, replayed) = process_receipt(&inv_pool, &req, None)
        .await
        .expect("replay receipt");
    assert!(replayed, "must detect replay");
    assert_eq!(replay_result.receipt_line_id, result.receipt_line_id);

    cleanup_inventory_tenant(&inv_pool, &tenant_id).await;
}

// ============================================================================
// Test 5: BOM depth guard prevents infinite recursion
// ============================================================================

#[tokio::test]
#[serial]
async fn bom_explosion_depth_guard() {
    let bom_pool = get_bom_pool().await;
    let tenant_id = test_tenant();

    // Create a simple 2-level BOM: Assembly → SubAssy → Leaf
    let assembly = Uuid::new_v4();
    let sub_assy = Uuid::new_v4();
    let leaf = Uuid::new_v4();

    // Top-level BOM
    let bom1 = bom_service::create_bom(
        &bom_pool,
        &tenant_id,
        &CreateBomRequest {
            part_id: assembly,
            description: Some("Top assembly".to_string()),
        },
        "corr-depth",
        None,
    )
    .await
    .unwrap();

    let rev1 = bom_service::create_revision(
        &bom_pool,
        &tenant_id,
        bom1.id,
        &CreateRevisionRequest {
            revision_label: "R1".to_string(),
        },
        "corr-depth",
        None,
    )
    .await
    .unwrap();

    bom_service::add_line(
        &bom_pool,
        &tenant_id,
        rev1.id,
        &AddLineRequest {
            component_item_id: sub_assy,
            quantity: 1.0,
            uom: None,
            scrap_factor: None,
            find_number: None,
        },
        "corr-depth",
        None,
    )
    .await
    .unwrap();

    let now = Utc::now();
    bom_service::set_effectivity(
        &bom_pool,
        &tenant_id,
        rev1.id,
        &SetEffectivityRequest {
            effective_from: now,
            effective_to: None,
        },
        "corr-depth",
        None,
    )
    .await
    .unwrap();

    // Sub-assembly BOM
    let bom2 = bom_service::create_bom(
        &bom_pool,
        &tenant_id,
        &CreateBomRequest {
            part_id: sub_assy,
            description: Some("Sub assembly".to_string()),
        },
        "corr-depth",
        None,
    )
    .await
    .unwrap();

    let rev2 = bom_service::create_revision(
        &bom_pool,
        &tenant_id,
        bom2.id,
        &CreateRevisionRequest {
            revision_label: "R1".to_string(),
        },
        "corr-depth",
        None,
    )
    .await
    .unwrap();

    bom_service::add_line(
        &bom_pool,
        &tenant_id,
        rev2.id,
        &AddLineRequest {
            component_item_id: leaf,
            quantity: 3.0,
            uom: None,
            scrap_factor: None,
            find_number: None,
        },
        "corr-depth",
        None,
    )
    .await
    .unwrap();

    bom_service::set_effectivity(
        &bom_pool,
        &tenant_id,
        rev2.id,
        &SetEffectivityRequest {
            effective_from: now,
            effective_to: None,
        },
        "corr-depth",
        None,
    )
    .await
    .unwrap();

    // Explode with depth=1 — should only see level 1 (sub_assy)
    let shallow = bom_queries::explode(
        &bom_pool,
        &tenant_id,
        bom1.id,
        &ExplosionQuery {
            date: Some(now),
            max_depth: Some(1),
        },
    )
    .await
    .unwrap();

    assert_eq!(
        shallow.len(),
        1,
        "depth=1 should only return immediate children"
    );
    assert_eq!(shallow[0].component_item_id, sub_assy);

    // Explode with depth=5 — should see both levels
    let deep = bom_queries::explode(
        &bom_pool,
        &tenant_id,
        bom1.id,
        &ExplosionQuery {
            date: Some(now),
            max_depth: Some(5),
        },
    )
    .await
    .unwrap();

    assert_eq!(
        deep.len(),
        2,
        "depth=5 should return 2 rows (level 1 + level 2)"
    );
    let level_1: Vec<_> = deep.iter().filter(|r| r.level == 1).collect();
    let level_2: Vec<_> = deep.iter().filter(|r| r.level == 2).collect();
    assert_eq!(level_1.len(), 1);
    assert_eq!(level_2.len(), 1);
    assert_eq!(level_2[0].component_item_id, leaf);

    cleanup_bom_tenant(&bom_pool, &tenant_id).await;
}
