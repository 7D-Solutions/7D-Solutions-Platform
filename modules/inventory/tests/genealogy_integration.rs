//! Integration tests for lot genealogy split/merge (bd-2tjuy).
//!
//! Tests run against a real PostgreSQL database.
//! Set DATABASE_URL to the inventory database connection string.
//!
//! Coverage:
//! 1. Split: one parent → two children, edges created, event in outbox
//! 2. Merge: two parents → one child, edges created, event in outbox
//! 3. Split idempotency replay returns same result
//! 4. Idempotency key conflict returns error
//! 5. Lot-not-found returns error
//! 6. Tenant isolation: edges from other tenants are excluded
//! 7. Self-referencing edge rejected (child == parent)
//! 8. children_of / parents_of queries return correct edges

use inventory_rs::domain::{
    genealogy::{
        children_of, parents_of, process_merge, process_split, GenealogyError, LotMergeRequest,
        LotSplitRequest, MergeParent, SplitChild,
    },
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    receipt_service::{process_receipt, ReceiptRequest},
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// Helpers
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

async fn create_lot_item(pool: &sqlx::PgPool, tenant_id: &str) -> Uuid {
    ItemRepo::create(
        pool,
        &CreateItemRequest {
            tenant_id: tenant_id.to_string(),
            sku: format!("GEN-{}", Uuid::new_v4()),
            name: "Genealogy Widget".to_string(),
            description: None,
            inventory_account_ref: "1200".to_string(),
            cogs_account_ref: "5000".to_string(),
            variance_account_ref: "5010".to_string(),
            uom: None,
            tracking_mode: TrackingMode::Lot,
        make_buy: None,
        },
    )
    .await
    .expect("create lot item")
    .id
}

async fn receive_lot(
    pool: &sqlx::PgPool,
    tenant: &str,
    item_id: Uuid,
    lot_code: &str,
    qty: i64,
) {
    process_receipt(
        pool,
        &ReceiptRequest {
            tenant_id: tenant.to_string(),
            item_id,
            warehouse_id: Uuid::new_v4(),
            quantity: qty,
            unit_cost_minor: 1000,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: format!("rc-gen-{}-{}", lot_code, Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
            lot_code: Some(lot_code.to_string()),
            serial_codes: None,
            location_id: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("receipt");
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
    sqlx::query("DELETE FROM inv_lot_genealogy WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM layer_consumptions WHERE EXISTS (SELECT 1 FROM inventory_layers il WHERE il.id = layer_id AND il.tenant_id = $1)")
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
    sqlx::query("DELETE FROM inventory_lots WHERE tenant_id = $1")
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
// Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_split_creates_edges_and_outbox_event() {
    let pool = setup_db().await;
    let tenant = format!("t-gen-split-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    let item_id = create_lot_item(&pool, &tenant).await;
    receive_lot(&pool, &tenant, item_id, "PARENT-1", 100).await;

    let (result, is_replay) = process_split(
        &pool,
        &LotSplitRequest {
            tenant_id: tenant.clone(),
            item_id,
            parent_lot_code: "PARENT-1".to_string(),
            children: vec![
                SplitChild {
                    lot_code: "CHILD-A".to_string(),
                    quantity: 60,
                },
                SplitChild {
                    lot_code: "CHILD-B".to_string(),
                    quantity: 40,
                },
            ],
            actor_id: None,
            notes: Some("split test".to_string()),
            idempotency_key: "split-test-001".to_string(),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect("split should succeed");

    assert!(!is_replay);
    assert_eq!(result.edge_count, 2);

    // Verify edges in database
    let parent_lot_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM inventory_lots WHERE tenant_id = $1 AND lot_code = 'PARENT-1'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("parent lot exists");

    let children = children_of(&pool, &tenant, parent_lot_id)
        .await
        .expect("query children");
    assert_eq!(children.len(), 2);
    assert!(children.iter().all(|e| e.transformation == "split"));
    assert_eq!(
        children.iter().map(|e| e.quantity).sum::<i64>(),
        100
    );

    // Verify outbox event
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inv_outbox WHERE tenant_id = $1 AND event_type = 'inventory.lot_split.v1'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("outbox count");
    assert_eq!(outbox_count, 1);

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn test_merge_creates_edges_and_outbox_event() {
    let pool = setup_db().await;
    let tenant = format!("t-gen-merge-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    let item_id = create_lot_item(&pool, &tenant).await;
    receive_lot(&pool, &tenant, item_id, "SRC-1", 30).await;
    receive_lot(&pool, &tenant, item_id, "SRC-2", 20).await;

    let (result, is_replay) = process_merge(
        &pool,
        &LotMergeRequest {
            tenant_id: tenant.clone(),
            item_id,
            parents: vec![
                MergeParent {
                    lot_code: "SRC-1".to_string(),
                    quantity: 30,
                },
                MergeParent {
                    lot_code: "SRC-2".to_string(),
                    quantity: 20,
                },
            ],
            child_lot_code: "MERGED-1".to_string(),
            actor_id: None,
            notes: None,
            idempotency_key: "merge-test-001".to_string(),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect("merge should succeed");

    assert!(!is_replay);
    assert_eq!(result.edge_count, 2);

    // Verify edges: child should have 2 parents
    let child_lot_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM inventory_lots WHERE tenant_id = $1 AND lot_code = 'MERGED-1'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("child lot exists");

    let parents = parents_of(&pool, &tenant, child_lot_id)
        .await
        .expect("query parents");
    assert_eq!(parents.len(), 2);
    assert!(parents.iter().all(|e| e.transformation == "merge"));
    assert_eq!(parents.iter().map(|e| e.quantity).sum::<i64>(), 50);

    // Verify outbox event
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inv_outbox WHERE tenant_id = $1 AND event_type = 'inventory.lot_merged.v1'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("outbox count");
    assert_eq!(outbox_count, 1);

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn test_split_idempotency_replay() {
    let pool = setup_db().await;
    let tenant = format!("t-gen-idem-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    let item_id = create_lot_item(&pool, &tenant).await;
    receive_lot(&pool, &tenant, item_id, "IDEM-PARENT", 50).await;

    let req = LotSplitRequest {
        tenant_id: tenant.clone(),
        item_id,
        parent_lot_code: "IDEM-PARENT".to_string(),
        children: vec![SplitChild {
            lot_code: "IDEM-CHILD".to_string(),
            quantity: 50,
        }],
        actor_id: None,
        notes: None,
        idempotency_key: "idem-split-001".to_string(),
        correlation_id: None,
        causation_id: None,
    };

    let (result1, replay1) = process_split(&pool, &req).await.expect("first split");
    assert!(!replay1);

    let (result2, replay2) = process_split(&pool, &req).await.expect("replay split");
    assert!(replay2);
    assert_eq!(result1.operation_id, result2.operation_id);
    assert_eq!(result1.event_id, result2.event_id);

    // Only one outbox event should exist (not two)
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inv_outbox WHERE tenant_id = $1 AND event_type = 'inventory.lot_split.v1'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("outbox count");
    assert_eq!(outbox_count, 1);

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn test_idempotency_key_conflict() {
    let pool = setup_db().await;
    let tenant = format!("t-gen-conflict-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    let item_id = create_lot_item(&pool, &tenant).await;
    receive_lot(&pool, &tenant, item_id, "CONF-PARENT", 80).await;

    // First request
    process_split(
        &pool,
        &LotSplitRequest {
            tenant_id: tenant.clone(),
            item_id,
            parent_lot_code: "CONF-PARENT".to_string(),
            children: vec![SplitChild {
                lot_code: "CONF-C1".to_string(),
                quantity: 80,
            }],
            actor_id: None,
            notes: None,
            idempotency_key: "conflict-key-001".to_string(),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect("first split");

    // Second request with same key but different body
    let err = process_split(
        &pool,
        &LotSplitRequest {
            tenant_id: tenant.clone(),
            item_id,
            parent_lot_code: "CONF-PARENT".to_string(),
            children: vec![SplitChild {
                lot_code: "CONF-DIFFERENT".to_string(),
                quantity: 80,
            }],
            actor_id: None,
            notes: None,
            idempotency_key: "conflict-key-001".to_string(),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect_err("should return conflict");

    assert!(matches!(err, GenealogyError::ConflictingIdempotencyKey));

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn test_split_lot_not_found() {
    let pool = setup_db().await;
    let tenant = format!("t-gen-notfound-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    let item_id = create_lot_item(&pool, &tenant).await;

    let err = process_split(
        &pool,
        &LotSplitRequest {
            tenant_id: tenant.clone(),
            item_id,
            parent_lot_code: "DOES-NOT-EXIST".to_string(),
            children: vec![SplitChild {
                lot_code: "CHILD-X".to_string(),
                quantity: 10,
            }],
            actor_id: None,
            notes: None,
            idempotency_key: "notfound-001".to_string(),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect_err("should fail with lot not found");

    assert!(matches!(err, GenealogyError::LotNotFound(_)));

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn test_tenant_isolation() {
    let pool = setup_db().await;
    let tenant_a = format!("t-gen-iso-a-{}", Uuid::new_v4());
    let tenant_b = format!("t-gen-iso-b-{}", Uuid::new_v4());
    cleanup(&pool, &tenant_a).await;
    cleanup(&pool, &tenant_b).await;

    let item_a = create_lot_item(&pool, &tenant_a).await;
    let item_b = create_lot_item(&pool, &tenant_b).await;
    receive_lot(&pool, &tenant_a, item_a, "ISO-PARENT", 50).await;
    receive_lot(&pool, &tenant_b, item_b, "ISO-PARENT", 50).await;

    // Split in tenant A
    process_split(
        &pool,
        &LotSplitRequest {
            tenant_id: tenant_a.clone(),
            item_id: item_a,
            parent_lot_code: "ISO-PARENT".to_string(),
            children: vec![SplitChild {
                lot_code: "ISO-CHILD".to_string(),
                quantity: 50,
            }],
            actor_id: None,
            notes: None,
            idempotency_key: "iso-split-a".to_string(),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect("split in tenant A");

    // Query children in tenant B — should see nothing
    let parent_b_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM inventory_lots WHERE tenant_id = $1 AND lot_code = 'ISO-PARENT'",
    )
    .bind(&tenant_b)
    .fetch_one(&pool)
    .await
    .expect("parent lot in tenant B");

    let children_b = children_of(&pool, &tenant_b, parent_b_id)
        .await
        .expect("query children in tenant B");
    assert!(
        children_b.is_empty(),
        "tenant B should see no edges from tenant A"
    );

    // Query children in tenant A — should see the edge
    let parent_a_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM inventory_lots WHERE tenant_id = $1 AND lot_code = 'ISO-PARENT'",
    )
    .bind(&tenant_a)
    .fetch_one(&pool)
    .await
    .expect("parent lot in tenant A");

    let children_a = children_of(&pool, &tenant_a, parent_a_id)
        .await
        .expect("query children in tenant A");
    assert_eq!(children_a.len(), 1);

    cleanup(&pool, &tenant_a).await;
    cleanup(&pool, &tenant_b).await;
}

#[tokio::test]
#[serial]
async fn test_self_reference_rejected() {
    let pool = setup_db().await;
    let tenant = format!("t-gen-selfref-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    let item_id = create_lot_item(&pool, &tenant).await;
    receive_lot(&pool, &tenant, item_id, "SELF-LOT", 10).await;

    let err = process_split(
        &pool,
        &LotSplitRequest {
            tenant_id: tenant.clone(),
            item_id,
            parent_lot_code: "SELF-LOT".to_string(),
            children: vec![SplitChild {
                lot_code: "SELF-LOT".to_string(),
                quantity: 10,
            }],
            actor_id: None,
            notes: None,
            idempotency_key: "selfref-001".to_string(),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect_err("should reject self-referencing edge");

    assert!(matches!(err, GenealogyError::Validation(_)));

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn test_graph_integrity_split_then_merge() {
    let pool = setup_db().await;
    let tenant = format!("t-gen-graph-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    let item_id = create_lot_item(&pool, &tenant).await;
    receive_lot(&pool, &tenant, item_id, "ORIG", 100).await;

    // Split ORIG → A(60), B(40)
    process_split(
        &pool,
        &LotSplitRequest {
            tenant_id: tenant.clone(),
            item_id,
            parent_lot_code: "ORIG".to_string(),
            children: vec![
                SplitChild {
                    lot_code: "A".to_string(),
                    quantity: 60,
                },
                SplitChild {
                    lot_code: "B".to_string(),
                    quantity: 40,
                },
            ],
            actor_id: None,
            notes: None,
            idempotency_key: "graph-split".to_string(),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect("split");

    // Merge A + B → RECOMBINED
    process_merge(
        &pool,
        &LotMergeRequest {
            tenant_id: tenant.clone(),
            item_id,
            parents: vec![
                MergeParent {
                    lot_code: "A".to_string(),
                    quantity: 60,
                },
                MergeParent {
                    lot_code: "B".to_string(),
                    quantity: 40,
                },
            ],
            child_lot_code: "RECOMBINED".to_string(),
            actor_id: None,
            notes: None,
            idempotency_key: "graph-merge".to_string(),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect("merge");

    // Verify full graph:
    // ORIG → A, ORIG → B (split)
    // A → RECOMBINED, B → RECOMBINED (merge)
    let orig_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM inventory_lots WHERE tenant_id = $1 AND lot_code = 'ORIG'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .unwrap();

    let recombined_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM inventory_lots WHERE tenant_id = $1 AND lot_code = 'RECOMBINED'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .unwrap();

    let orig_children = children_of(&pool, &tenant, orig_id).await.unwrap();
    assert_eq!(orig_children.len(), 2);

    let recombined_parents = parents_of(&pool, &tenant, recombined_id).await.unwrap();
    assert_eq!(recombined_parents.len(), 2);
    assert_eq!(
        recombined_parents.iter().map(|e| e.quantity).sum::<i64>(),
        100
    );

    // Total edge count should be 4
    let total_edges: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inv_lot_genealogy WHERE tenant_id = $1",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(total_edges, 4);

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn test_quantity_conservation_guard() {
    let pool = setup_db().await;
    let tenant = format!("t-gen-conserve-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    let item_id = create_lot_item(&pool, &tenant).await;
    // Parent has 100 on hand
    receive_lot(&pool, &tenant, item_id, "CONSERVE-PARENT", 100).await;

    // Try to split into children that sum to 90 (not 100) — should fail
    let err = process_split(
        &pool,
        &LotSplitRequest {
            tenant_id: tenant.clone(),
            item_id,
            parent_lot_code: "CONSERVE-PARENT".to_string(),
            children: vec![
                SplitChild {
                    lot_code: "CONSERVE-A".to_string(),
                    quantity: 50,
                },
                SplitChild {
                    lot_code: "CONSERVE-B".to_string(),
                    quantity: 40,
                },
            ],
            actor_id: None,
            notes: None,
            idempotency_key: "conserve-001".to_string(),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect_err("should reject non-conserving split");

    assert!(
        matches!(
            err,
            GenealogyError::QuantityConservation {
                children_sum: 90,
                parent_qty: 100
            }
        ),
        "expected QuantityConservation error, got: {:?}",
        err,
    );

    // Correct split that sums to 100 should succeed
    process_split(
        &pool,
        &LotSplitRequest {
            tenant_id: tenant.clone(),
            item_id,
            parent_lot_code: "CONSERVE-PARENT".to_string(),
            children: vec![
                SplitChild {
                    lot_code: "CONSERVE-A".to_string(),
                    quantity: 60,
                },
                SplitChild {
                    lot_code: "CONSERVE-B".to_string(),
                    quantity: 40,
                },
            ],
            actor_id: None,
            notes: None,
            idempotency_key: "conserve-002".to_string(),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect("correct split should succeed");

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn test_multilevel_genealogy_trace() {
    let pool = setup_db().await;
    let tenant = format!("t-gen-trace-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    let item_id = create_lot_item(&pool, &tenant).await;
    receive_lot(&pool, &tenant, item_id, "ROOT", 100).await;

    // Level 1: ROOT(100) → LEVEL1-A(60), LEVEL1-B(40)
    process_split(
        &pool,
        &LotSplitRequest {
            tenant_id: tenant.clone(),
            item_id,
            parent_lot_code: "ROOT".to_string(),
            children: vec![
                SplitChild {
                    lot_code: "LEVEL1-A".to_string(),
                    quantity: 60,
                },
                SplitChild {
                    lot_code: "LEVEL1-B".to_string(),
                    quantity: 40,
                },
            ],
            actor_id: None,
            notes: None,
            idempotency_key: "trace-split-1".to_string(),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect("first split");

    // Receive into LEVEL1-A so it has on-hand for the second split
    receive_lot(&pool, &tenant, item_id, "LEVEL1-A", 60).await;

    // Level 2: LEVEL1-A(60) → LEAF-1(30), LEAF-2(30)
    process_split(
        &pool,
        &LotSplitRequest {
            tenant_id: tenant.clone(),
            item_id,
            parent_lot_code: "LEVEL1-A".to_string(),
            children: vec![
                SplitChild {
                    lot_code: "LEAF-1".to_string(),
                    quantity: 30,
                },
                SplitChild {
                    lot_code: "LEAF-2".to_string(),
                    quantity: 30,
                },
            ],
            actor_id: None,
            notes: None,
            idempotency_key: "trace-split-2".to_string(),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect("second split");

    // Trace forward from ROOT: should have 2 direct children (LEVEL1-A, LEVEL1-B)
    let root_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM inventory_lots WHERE tenant_id = $1 AND lot_code = 'ROOT'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .unwrap();

    let root_children = children_of(&pool, &tenant, root_id).await.unwrap();
    assert_eq!(root_children.len(), 2, "ROOT should have 2 direct children");

    // Trace forward from LEVEL1-A: should have 2 children (LEAF-1, LEAF-2)
    let level1a_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM inventory_lots WHERE tenant_id = $1 AND lot_code = 'LEVEL1-A'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .unwrap();

    let level1a_children = children_of(&pool, &tenant, level1a_id).await.unwrap();
    assert_eq!(
        level1a_children.len(),
        2,
        "LEVEL1-A should have 2 children"
    );

    // Trace backward from LEAF-1: should have 1 parent (LEVEL1-A)
    let leaf1_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM inventory_lots WHERE tenant_id = $1 AND lot_code = 'LEAF-1'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .unwrap();

    let leaf1_parents = parents_of(&pool, &tenant, leaf1_id).await.unwrap();
    assert_eq!(leaf1_parents.len(), 1, "LEAF-1 should have 1 parent");
    assert_eq!(leaf1_parents[0].parent_lot_id, level1a_id);

    // Full lineage: trace LEAF-1 → LEVEL1-A → ROOT (two hops)
    let level1a_parents = parents_of(&pool, &tenant, level1a_id).await.unwrap();
    assert_eq!(
        level1a_parents.len(),
        1,
        "LEVEL1-A should have 1 parent (ROOT)"
    );
    assert_eq!(level1a_parents[0].parent_lot_id, root_id);

    // Total edge count: 2 (ROOT split) + 2 (LEVEL1-A split) = 4
    let total_edges: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inv_lot_genealogy WHERE tenant_id = $1",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(total_edges, 4, "should have 4 genealogy edges total");

    cleanup(&pool, &tenant).await;
}
