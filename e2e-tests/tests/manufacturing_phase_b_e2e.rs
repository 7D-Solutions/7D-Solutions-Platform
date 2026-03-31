//! E2E Integration Proof: Manufacturing Phase B
//!
//! Proves the Phase B "Prove at end" bullets from the manufacturing roadmap:
//! 1. WO created → components issued (FIFO consumed) → operations completed → FG receipt at rolled-up cost
//! 2. Cost rollup arithmetic spot-check: sum of component FIFO costs ≤ FG receipt unit cost
//! 3. Workcenter definitions used by operations
//! 4. Audit trace: correlation_id chains WO → issue → receipt events
//!
//! All tests use real Postgres (Production DB port 5461, Inventory DB port 5442).
//! No mocks, no stubs.

use inventory_rs::consumers::component_issue_consumer::{
    process_component_issue_request, ComponentIssueItem, ComponentIssueRequestedPayload,
};
use inventory_rs::consumers::fg_receipt_consumer::{
    process_fg_receipt_request, FgReceiptRequestedPayload,
};
use inventory_rs::domain::{
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    receipt_service::{process_receipt, ReceiptRequest},
};
use production_rs::domain::component_issue::{
    request_component_issue, ComponentIssueItemInput, RequestComponentIssueRequest,
};
use production_rs::domain::fg_receipt::{request_fg_receipt, RequestFgReceiptRequest};
use production_rs::domain::operations::OperationRepo;
use production_rs::domain::routings::{AddRoutingStepRequest, CreateRoutingRequest, RoutingRepo};
use production_rs::domain::work_orders::{CreateWorkOrderRequest, WorkOrderRepo};
use production_rs::domain::workcenters::{CreateWorkcenterRequest, WorkcenterRepo};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

// ============================================================================
// DB setup
// ============================================================================

async fn get_production_pool() -> PgPool {
    let url = std::env::var("PRODUCTION_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://production_user:production_pass@localhost:5461/production_db".to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to Production DB");

    sqlx::migrate!("../modules/production/db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run Production migrations");

    pool
}

async fn get_inventory_pool() -> PgPool {
    let url = std::env::var("INVENTORY_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgres://inventory_user:inventory_pass@localhost:5442/inventory_db".to_string()
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
// Helpers
// ============================================================================

fn test_tenant() -> String {
    format!("mfg-phase-b-{}", Uuid::new_v4())
}

fn create_item_req(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: format!("Phase B Test: {}", sku),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
        make_buy: None,
    }
}

async fn stock_item(
    pool: &PgPool,
    tenant: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    qty: i64,
    cost_minor: i64,
) {
    let req = ReceiptRequest {
        tenant_id: tenant.to_string(),
        item_id,
        warehouse_id,
        quantity: qty,
        unit_cost_minor: cost_minor,
        currency: "usd".to_string(),
        source_type: "purchase".to_string(),
        purchase_order_id: None,
        idempotency_key: format!("stock-{}", Uuid::new_v4()),
        correlation_id: Some("phase-b-stock".to_string()),
        causation_id: None,
        lot_code: None,
        serial_codes: None,
        location_id: None,
        uom_id: None,
    };
    process_receipt(pool, &req, None)
        .await
        .expect("stock item via purchase receipt");
}

// ============================================================================
// Test 1: Full floor loop — WO → issue → ops → FG receipt
// ============================================================================

#[tokio::test]
#[serial]
async fn full_floor_loop_wo_issue_ops_fg_receipt() {
    let prod_pool = get_production_pool().await;
    let inv_pool = get_inventory_pool().await;
    let tenant = test_tenant();
    let warehouse_id = Uuid::new_v4();
    let corr_id = format!("phase-b-e2e-{}", Uuid::new_v4());

    // ---- Step 1: Create workcenters ----
    let wc_machining = WorkcenterRepo::create(
        &prod_pool,
        &CreateWorkcenterRequest {
            tenant_id: tenant.clone(),
            code: format!("MC-{}", &Uuid::new_v4().to_string()[..8]),
            name: "Machining Center".to_string(),
            description: None,
            capacity: Some(1),
            cost_rate_minor: Some(5000),
            idempotency_key: None,
        },
        &corr_id,
        None,
    )
    .await
    .expect("create machining workcenter");

    let wc_assembly = WorkcenterRepo::create(
        &prod_pool,
        &CreateWorkcenterRequest {
            tenant_id: tenant.clone(),
            code: format!("AS-{}", &Uuid::new_v4().to_string()[..8]),
            name: "Assembly Station".to_string(),
            description: None,
            capacity: Some(2),
            cost_rate_minor: Some(3000),
            idempotency_key: None,
        },
        &corr_id,
        None,
    )
    .await
    .expect("create assembly workcenter");

    // ---- Step 2: Create routing template with 2 steps ----
    let fg_item_id = Uuid::new_v4(); // will be created in inventory below
    let bom_rev_id = Uuid::new_v4(); // reference only

    let routing = RoutingRepo::create(
        &prod_pool,
        &CreateRoutingRequest {
            tenant_id: tenant.clone(),
            name: "Assembly Routing Rev A".to_string(),
            description: Some("Two-step: machine then assemble".to_string()),
            item_id: Some(fg_item_id),
            bom_revision_id: Some(bom_rev_id),
            revision: Some("A".to_string()),
            effective_from_date: None,
            idempotency_key: None,
        },
        &corr_id,
        None,
    )
    .await
    .expect("create routing template");

    RoutingRepo::add_step(
        &prod_pool,
        routing.routing_template_id,
        &AddRoutingStepRequest {
            tenant_id: tenant.clone(),
            sequence_number: 10,
            workcenter_id: wc_machining.workcenter_id,
            operation_name: "Machine housing".to_string(),
            description: Some("CNC machining of housing part".to_string()),
            setup_time_minutes: Some(15),
            run_time_minutes: Some(30),
            is_required: Some(true),
            idempotency_key: None,
        },
        &corr_id,
        None,
    )
    .await
    .expect("add machining step");

    RoutingRepo::add_step(
        &prod_pool,
        routing.routing_template_id,
        &AddRoutingStepRequest {
            tenant_id: tenant.clone(),
            sequence_number: 20,
            workcenter_id: wc_assembly.workcenter_id,
            operation_name: "Final assembly".to_string(),
            description: Some("Assemble components into finished good".to_string()),
            setup_time_minutes: Some(5),
            run_time_minutes: Some(20),
            is_required: Some(true),
            idempotency_key: None,
        },
        &corr_id,
        None,
    )
    .await
    .expect("add assembly step");

    RoutingRepo::release(&prod_pool, routing.routing_template_id, &tenant, &corr_id, None)
        .await
        .expect("release routing");

    // ---- Step 3: Create inventory items and stock components ----
    let comp_a = ItemRepo::create(
        &inv_pool,
        &create_item_req(&tenant, &format!("COMP-A-{}", &Uuid::new_v4().to_string()[..8])),
    )
    .await
    .expect("create component A");

    let comp_b = ItemRepo::create(
        &inv_pool,
        &create_item_req(&tenant, &format!("COMP-B-{}", &Uuid::new_v4().to_string()[..8])),
    )
    .await
    .expect("create component B");

    let fg_item = ItemRepo::create(
        &inv_pool,
        &create_item_req(&tenant, &format!("FG-{}", &Uuid::new_v4().to_string()[..8])),
    )
    .await
    .expect("create FG item");

    // Stock component A: 100 @ $3.00 (300 minor)
    stock_item(&inv_pool, &tenant, comp_a.id, warehouse_id, 100, 300).await;

    // Stock component B: two FIFO layers
    //   Layer 1: 20 @ $8.00 (800 minor)
    //   Layer 2: 30 @ $12.00 (1200 minor)
    stock_item(&inv_pool, &tenant, comp_b.id, warehouse_id, 20, 800).await;
    stock_item(&inv_pool, &tenant, comp_b.id, warehouse_id, 30, 1200).await;

    // ---- Step 4: Create and release work order ----
    let wo = WorkOrderRepo::create(
        &prod_pool,
        &CreateWorkOrderRequest {
            tenant_id: tenant.clone(),
            order_number: format!("WO-E2E-{}", &Uuid::new_v4().to_string()[..8]),
            item_id: fg_item.id,
            bom_revision_id: bom_rev_id,
            routing_template_id: Some(routing.routing_template_id),
            planned_quantity: 10,
            planned_start: None,
            planned_end: None,
            correlation_id: Some(corr_id.clone()),
        },
        &corr_id,
        None,
    )
    .await
    .expect("create work order");

    assert_eq!(wo.status, "draft");

    WorkOrderRepo::release(&prod_pool, wo.work_order_id, &tenant, &corr_id, None)
        .await
        .expect("release work order");

    // ---- Step 5: Initialize and complete operations ----
    let ops = OperationRepo::initialize(
        &prod_pool,
        wo.work_order_id,
        &tenant,
        &corr_id,
        None,
    )
    .await
    .expect("initialize operations from routing");

    assert_eq!(ops.len(), 2, "2 operations from 2 routing steps");
    assert_eq!(ops[0].workcenter_id, wc_machining.workcenter_id);
    assert_eq!(ops[1].workcenter_id, wc_assembly.workcenter_id);
    assert_eq!(ops[0].operation_name, "Machine housing");
    assert_eq!(ops[1].operation_name, "Final assembly");

    // Start and complete op 1 (machining)
    OperationRepo::start(
        &prod_pool,
        wo.work_order_id,
        ops[0].operation_id,
        &tenant,
        &corr_id,
        None,
    )
    .await
    .expect("start machining op");

    OperationRepo::complete(
        &prod_pool,
        wo.work_order_id,
        ops[0].operation_id,
        &tenant,
        &corr_id,
        None,
    )
    .await
    .expect("complete machining op");

    // Start and complete op 2 (assembly)
    OperationRepo::start(
        &prod_pool,
        wo.work_order_id,
        ops[1].operation_id,
        &tenant,
        &corr_id,
        None,
    )
    .await
    .expect("start assembly op");

    OperationRepo::complete(
        &prod_pool,
        wo.work_order_id,
        ops[1].operation_id,
        &tenant,
        &corr_id,
        None,
    )
    .await
    .expect("complete assembly op");

    // Verify all ops completed
    let final_ops = OperationRepo::list(&prod_pool, wo.work_order_id, &tenant)
        .await
        .expect("list operations");
    assert!(
        final_ops.iter().all(|op| op.status == "completed"),
        "All operations must be completed"
    );

    // ---- Step 6: Component issue (Production → Inventory via event) ----
    // Production side: emit the request event via outbox
    request_component_issue(
        &prod_pool,
        wo.work_order_id,
        &RequestComponentIssueRequest {
            tenant_id: tenant.clone(),
            items: vec![
                ComponentIssueItemInput {
                    item_id: comp_a.id,
                    warehouse_id,
                    quantity: 20,
                    currency: "usd".to_string(),
                },
                ComponentIssueItemInput {
                    item_id: comp_b.id,
                    warehouse_id,
                    quantity: 25,
                    currency: "usd".to_string(),
                },
            ],
            correlation_id: Some(corr_id.clone()),
            causation_id: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("production: request component issue");

    // Inventory side: process the event (simulating NATS consumer)
    let issue_event_id = Uuid::new_v4();
    let issue_results = process_component_issue_request(
        &inv_pool,
        issue_event_id,
        &ComponentIssueRequestedPayload {
            work_order_id: wo.work_order_id,
            tenant_id: tenant.clone(),
            order_number: wo.order_number.clone(),
            items: vec![
                ComponentIssueItem {
                    item_id: comp_a.id,
                    warehouse_id,
                    quantity: 20,
                    currency: "usd".to_string(),
                },
                ComponentIssueItem {
                    item_id: comp_b.id,
                    warehouse_id,
                    quantity: 25,
                    currency: "usd".to_string(),
                },
            ],
        },
        Some(&corr_id),
        None,
    )
    .await
    .expect("inventory: process component issue");

    assert_eq!(issue_results.len(), 2);

    // Component A: 20 * 300 = 6000
    let cost_a = issue_results[0].total_cost_minor;
    assert_eq!(cost_a, 6000, "Comp A: 20 units @ $3.00 = $60.00");

    // Component B: FIFO → 20*800 + 5*1200 = 16000 + 6000 = 22000
    let cost_b = issue_results[1].total_cost_minor;
    assert_eq!(cost_b, 22000, "Comp B: FIFO 20@$8 + 5@$12 = $220.00");

    let total_component_cost = cost_a + cost_b; // 28000

    // ---- Step 7: FG receipt (Production → Inventory via event) ----
    let fg_qty: i64 = 10;

    // Production side: emit the request event via outbox
    request_fg_receipt(
        &prod_pool,
        wo.work_order_id,
        &RequestFgReceiptRequest {
            tenant_id: tenant.clone(),
            item_id: fg_item.id,
            warehouse_id,
            quantity: fg_qty,
            currency: "usd".to_string(),
            correlation_id: Some(corr_id.clone()),
            causation_id: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("production: request FG receipt");

    // Inventory side: process the event (simulating NATS consumer)
    let fg_event_id = Uuid::new_v4();
    let fg_result = process_fg_receipt_request(
        &inv_pool,
        fg_event_id,
        &FgReceiptRequestedPayload {
            work_order_id: wo.work_order_id,
            tenant_id: tenant.clone(),
            order_number: wo.order_number.clone(),
            item_id: fg_item.id,
            warehouse_id,
            quantity: fg_qty,
            currency: "usd".to_string(),
        },
        Some(&corr_id),
        None,
    )
    .await
    .expect("inventory: process FG receipt");

    // ---- Step 8: Assertions ----

    // 8a. Cost rollup arithmetic spot-check
    let expected_unit_cost = total_component_cost / fg_qty; // 28000 / 10 = 2800
    assert_eq!(
        fg_result.unit_cost_minor, expected_unit_cost,
        "FG unit cost must equal total component cost / FG qty"
    );
    assert_eq!(fg_result.unit_cost_minor, 2800);
    assert_eq!(fg_result.source_type, "production");
    assert_eq!(fg_result.quantity, fg_qty);

    // sum(component FIFO costs) >= FG receipt extended cost (integer division truncation)
    let receipt_extended = fg_result.unit_cost_minor * fg_result.quantity;
    assert!(
        total_component_cost >= receipt_extended,
        "Component cost {} must be >= receipt extended cost {}",
        total_component_cost,
        receipt_extended
    );

    // 8b. Inventory ledger: issued entries exist
    let issued_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1 AND entry_type = 'issued'",
    )
    .bind(&tenant)
    .fetch_one(&inv_pool)
    .await
    .expect("count issued ledger entries");
    assert!(issued_count >= 2, "At least 2 issued ledger entries (one per component)");

    // 8c. Inventory ledger: received entry for FG
    let received_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1 AND item_id = $2 AND entry_type = 'received' AND source_type = 'production'",
    )
    .bind(&tenant)
    .bind(fg_item.id)
    .fetch_one(&inv_pool)
    .await
    .expect("count FG received ledger entries");
    assert_eq!(received_count, 1, "Exactly 1 production receipt for FG item");

    // 8d. Correlation chain on production outbox events
    let prod_events: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT event_type, payload FROM production_outbox WHERE tenant_id = $1 ORDER BY created_at",
    )
    .bind(&tenant)
    .fetch_all(&prod_pool)
    .await
    .expect("fetch production outbox events");

    // Must have: WO created, WO released, routing events, op events, component issue, FG receipt
    let event_types: Vec<&str> = prod_events.iter().map(|(t, _)| t.as_str()).collect();
    assert!(
        event_types.contains(&"production.work_order_created"),
        "Missing WO created event"
    );
    assert!(
        event_types.contains(&"production.work_order_released"),
        "Missing WO released event"
    );
    assert!(
        event_types.contains(&"production.operation_started"),
        "Missing operation started event"
    );
    assert!(
        event_types.contains(&"production.operation_completed"),
        "Missing operation completed event"
    );
    assert!(
        event_types.contains(&"production.component_issue.requested"),
        "Missing component issue requested event"
    );
    assert!(
        event_types.contains(&"production.fg_receipt.requested"),
        "Missing FG receipt requested event"
    );

    // Verify correlation_id is present on all production events for this WO
    for (event_type, payload) in &prod_events {
        let corr = payload
            .get("correlation_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        assert!(
            !corr.is_empty(),
            "Event '{}' must have a correlation_id in envelope",
            event_type
        );
    }

    // 8e. Inventory outbox: item_issued events carry production source_ref
    let inv_issue_events: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT payload FROM inv_outbox WHERE tenant_id = $1 AND event_type = 'inventory.item_issued' ORDER BY created_at",
    )
    .bind(&tenant)
    .fetch_all(&inv_pool)
    .await
    .expect("fetch inventory issue outbox");

    assert!(
        inv_issue_events.len() >= 2,
        "At least 2 inventory issue events"
    );
    for payload in &inv_issue_events {
        let src = &payload["payload"]["source_ref"];
        assert_eq!(
            src["source_module"].as_str(),
            Some("production"),
            "Issue event must reference production module"
        );
        assert_eq!(
            src["source_id"].as_str(),
            Some(wo.work_order_id.to_string().as_str()),
            "Issue event must reference work order ID"
        );
    }

    // 8f. Inventory outbox: item_received event for FG with source_type=production
    let inv_receipt_payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM inv_outbox WHERE tenant_id = $1 AND event_type = 'inventory.item_received' ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&tenant)
    .fetch_one(&inv_pool)
    .await
    .expect("fetch FG receipt outbox event");

    assert_eq!(
        inv_receipt_payload["payload"]["source_type"].as_str(),
        Some("production")
    );
    assert_eq!(
        inv_receipt_payload["payload"]["unit_cost_minor"].as_i64(),
        Some(2800)
    );
}

// ============================================================================
// Test 2: Workcenter definitions are used by operations
// ============================================================================

#[tokio::test]
#[serial]
async fn workcenter_definitions_used_by_operations() {
    let prod_pool = get_production_pool().await;
    let tenant = test_tenant();
    let corr_id = format!("wc-ops-{}", Uuid::new_v4());

    // Create a workcenter
    let wc = WorkcenterRepo::create(
        &prod_pool,
        &CreateWorkcenterRequest {
            tenant_id: tenant.clone(),
            code: format!("WC-{}", &Uuid::new_v4().to_string()[..8]),
            name: "Test Workcenter".to_string(),
            description: Some("Verifies WC → operation linkage".to_string()),
            capacity: Some(5),
            cost_rate_minor: Some(7500),
            idempotency_key: None,
        },
        &corr_id,
        None,
    )
    .await
    .expect("create workcenter");

    // Create routing with one step using this workcenter
    let routing = RoutingRepo::create(
        &prod_pool,
        &CreateRoutingRequest {
            tenant_id: tenant.clone(),
            name: "WC Test Routing".to_string(),
            description: None,
            item_id: None,
            bom_revision_id: None,
            revision: Some("1".to_string()),
            effective_from_date: None,
            idempotency_key: None,
        },
        &corr_id,
        None,
    )
    .await
    .expect("create routing");

    RoutingRepo::add_step(
        &prod_pool,
        routing.routing_template_id,
        &AddRoutingStepRequest {
            tenant_id: tenant.clone(),
            sequence_number: 10,
            workcenter_id: wc.workcenter_id,
            operation_name: "WC Test Op".to_string(),
            description: None,
            setup_time_minutes: None,
            run_time_minutes: None,
            is_required: Some(true),
            idempotency_key: None,
        },
        &corr_id,
        None,
    )
    .await
    .expect("add step");

    RoutingRepo::release(&prod_pool, routing.routing_template_id, &tenant, &corr_id, None)
        .await
        .expect("release routing");

    // Create WO with this routing
    let wo = WorkOrderRepo::create(
        &prod_pool,
        &CreateWorkOrderRequest {
            tenant_id: tenant.clone(),
            order_number: format!("WO-WC-{}", &Uuid::new_v4().to_string()[..8]),
            item_id: Uuid::new_v4(),
            bom_revision_id: Uuid::new_v4(),
            routing_template_id: Some(routing.routing_template_id),
            planned_quantity: 1,
            planned_start: None,
            planned_end: None,
            correlation_id: None,
        },
        &corr_id,
        None,
    )
    .await
    .expect("create WO");

    WorkOrderRepo::release(&prod_pool, wo.work_order_id, &tenant, &corr_id, None)
        .await
        .expect("release WO");

    let ops = OperationRepo::initialize(&prod_pool, wo.work_order_id, &tenant, &corr_id, None)
        .await
        .expect("initialize ops");

    assert_eq!(ops.len(), 1);
    assert_eq!(
        ops[0].workcenter_id, wc.workcenter_id,
        "Operation must reference the workcenter from routing step"
    );
    assert_eq!(ops[0].operation_name, "WC Test Op");

    // Verify the workcenter still exists and is active
    let found = WorkcenterRepo::find_by_id(&prod_pool, wc.workcenter_id, &tenant)
        .await
        .expect("find workcenter");
    assert!(found.is_some(), "Workcenter must exist");
    assert!(found.unwrap().is_active, "Workcenter must be active");
}

// ============================================================================
// Test 3: Correlation chain integrity across WO → ops → issue → receipt
// ============================================================================

#[tokio::test]
#[serial]
async fn correlation_chain_wo_through_issue_and_receipt() {
    let prod_pool = get_production_pool().await;
    let inv_pool = get_inventory_pool().await;
    let tenant = test_tenant();
    let warehouse_id = Uuid::new_v4();
    let corr_id = format!("corr-chain-{}", Uuid::new_v4());

    // Create workcenter + routing (minimal: 1 step)
    let wc = WorkcenterRepo::create(
        &prod_pool,
        &CreateWorkcenterRequest {
            tenant_id: tenant.clone(),
            code: format!("CC-{}", &Uuid::new_v4().to_string()[..8]),
            name: "Corr Chain WC".to_string(),
            description: None,
            capacity: None,
            cost_rate_minor: None,
            idempotency_key: None,
        },
        &corr_id,
        None,
    )
    .await
    .expect("create WC");

    let routing = RoutingRepo::create(
        &prod_pool,
        &CreateRoutingRequest {
            tenant_id: tenant.clone(),
            name: "Corr Chain Routing".to_string(),
            description: None,
            item_id: None,
            bom_revision_id: None,
            revision: Some("1".to_string()),
            effective_from_date: None,
            idempotency_key: None,
        },
        &corr_id,
        None,
    )
    .await
    .expect("create routing");

    RoutingRepo::add_step(
        &prod_pool,
        routing.routing_template_id,
        &AddRoutingStepRequest {
            tenant_id: tenant.clone(),
            sequence_number: 10,
            workcenter_id: wc.workcenter_id,
            operation_name: "Single Op".to_string(),
            description: None,
            setup_time_minutes: None,
            run_time_minutes: None,
            is_required: Some(true),
            idempotency_key: None,
        },
        &corr_id,
        None,
    )
    .await
    .expect("add step");

    RoutingRepo::release(&prod_pool, routing.routing_template_id, &tenant, &corr_id, None)
        .await
        .expect("release routing");

    // Inventory: create items + stock
    let comp = ItemRepo::create(
        &inv_pool,
        &create_item_req(&tenant, &format!("CC-COMP-{}", &Uuid::new_v4().to_string()[..8])),
    )
    .await
    .expect("create component");

    let fg = ItemRepo::create(
        &inv_pool,
        &create_item_req(&tenant, &format!("CC-FG-{}", &Uuid::new_v4().to_string()[..8])),
    )
    .await
    .expect("create FG");

    stock_item(&inv_pool, &tenant, comp.id, warehouse_id, 50, 1000).await;

    // Create + release WO
    let wo = WorkOrderRepo::create(
        &prod_pool,
        &CreateWorkOrderRequest {
            tenant_id: tenant.clone(),
            order_number: format!("WO-CC-{}", &Uuid::new_v4().to_string()[..8]),
            item_id: fg.id,
            bom_revision_id: Uuid::new_v4(),
            routing_template_id: Some(routing.routing_template_id),
            planned_quantity: 5,
            planned_start: None,
            planned_end: None,
            correlation_id: Some(corr_id.clone()),
        },
        &corr_id,
        None,
    )
    .await
    .expect("create WO");

    WorkOrderRepo::release(&prod_pool, wo.work_order_id, &tenant, &corr_id, None)
        .await
        .expect("release WO");

    // Initialize + complete single operation
    let ops = OperationRepo::initialize(&prod_pool, wo.work_order_id, &tenant, &corr_id, None)
        .await
        .expect("init ops");

    OperationRepo::start(&prod_pool, wo.work_order_id, ops[0].operation_id, &tenant, &corr_id, None)
        .await
        .expect("start op");

    OperationRepo::complete(&prod_pool, wo.work_order_id, ops[0].operation_id, &tenant, &corr_id, None)
        .await
        .expect("complete op");

    // Component issue
    request_component_issue(
        &prod_pool,
        wo.work_order_id,
        &RequestComponentIssueRequest {
            tenant_id: tenant.clone(),
            items: vec![ComponentIssueItemInput {
                item_id: comp.id,
                warehouse_id,
                quantity: 10,
                currency: "usd".to_string(),
            }],
            correlation_id: Some(corr_id.clone()),
            causation_id: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("request component issue");

    // FG receipt
    request_fg_receipt(
        &prod_pool,
        wo.work_order_id,
        &RequestFgReceiptRequest {
            tenant_id: tenant.clone(),
            item_id: fg.id,
            warehouse_id,
            quantity: 5,
            currency: "usd".to_string(),
            correlation_id: Some(corr_id.clone()),
            causation_id: None,
            idempotency_key: None,
        },
    )
    .await
    .expect("request FG receipt");

    // Verify: ALL production outbox events for this tenant carry the same correlation_id
    let events: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT event_type, payload FROM production_outbox WHERE tenant_id = $1 ORDER BY created_at",
    )
    .bind(&tenant)
    .fetch_all(&prod_pool)
    .await
    .expect("fetch events");

    let expected_event_types = [
        "production.workcenter_created",
        "production.routing_created",
        "production.routing_updated", // from add_step
        "production.routing_released",
        "production.work_order_created",
        "production.work_order_released",
        "production.operation_started",
        "production.operation_completed",
        "production.component_issue.requested",
        "production.fg_receipt.requested",
    ];

    for expected in &expected_event_types {
        assert!(
            events.iter().any(|(t, _)| t == expected),
            "Missing expected event type: {}",
            expected
        );
    }

    // All events must carry correlation_id = corr_id
    for (event_type, payload) in &events {
        let event_corr = payload
            .get("correlation_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(
            event_corr, &corr_id,
            "Event '{}' correlation_id mismatch: expected '{}', got '{}'",
            event_type, corr_id, event_corr
        );
    }
}

// ============================================================================
// Test 4: Cost arithmetic with exact FIFO layer verification
// ============================================================================

#[tokio::test]
#[serial]
async fn cost_arithmetic_fifo_layers_spot_check() {
    let inv_pool = get_inventory_pool().await;
    let tenant = test_tenant();
    let warehouse_id = Uuid::new_v4();
    let wo_id = Uuid::new_v4();

    // Create items
    let comp = ItemRepo::create(
        &inv_pool,
        &create_item_req(&tenant, &format!("FIFO-{}", &Uuid::new_v4().to_string()[..8])),
    )
    .await
    .expect("create component");

    let fg = ItemRepo::create(
        &inv_pool,
        &create_item_req(&tenant, &format!("FG-{}", &Uuid::new_v4().to_string()[..8])),
    )
    .await
    .expect("create FG item");

    // Stock 3 FIFO layers:
    //   L1: 10 @ $1.00 (100 minor)
    //   L2: 10 @ $2.00 (200 minor)
    //   L3: 10 @ $3.00 (300 minor)
    stock_item(&inv_pool, &tenant, comp.id, warehouse_id, 10, 100).await;
    stock_item(&inv_pool, &tenant, comp.id, warehouse_id, 10, 200).await;
    stock_item(&inv_pool, &tenant, comp.id, warehouse_id, 10, 300).await;

    // Issue 25 units via consumer: FIFO → 10*100 + 10*200 + 5*300 = 1000 + 2000 + 1500 = 4500
    let issue_event_id = Uuid::new_v4();
    let issue_results = process_component_issue_request(
        &inv_pool,
        issue_event_id,
        &ComponentIssueRequestedPayload {
            work_order_id: wo_id,
            tenant_id: tenant.clone(),
            order_number: "WO-FIFO-SPOT".to_string(),
            items: vec![ComponentIssueItem {
                item_id: comp.id,
                warehouse_id,
                quantity: 25,
                currency: "usd".to_string(),
            }],
        },
        Some("fifo-spot-corr"),
        None,
    )
    .await
    .expect("process issue");

    let total_cost = issue_results[0].total_cost_minor;
    assert_eq!(total_cost, 4500, "FIFO: 10*100 + 10*200 + 5*300 = 4500");
    assert_eq!(
        issue_results[0].consumed_layers.len(),
        3,
        "Must consume from 3 FIFO layers"
    );

    // FG receipt: 5 units
    let fg_qty: i64 = 5;
    let fg_event_id = Uuid::new_v4();
    let fg_result = process_fg_receipt_request(
        &inv_pool,
        fg_event_id,
        &FgReceiptRequestedPayload {
            work_order_id: wo_id,
            tenant_id: tenant.clone(),
            order_number: "WO-FIFO-SPOT".to_string(),
            item_id: fg.id,
            warehouse_id,
            quantity: fg_qty,
            currency: "usd".to_string(),
        },
        Some("fifo-spot-corr"),
        None,
    )
    .await
    .expect("process FG receipt");

    // 4500 / 5 = 900 per unit
    assert_eq!(fg_result.unit_cost_minor, 900);
    assert_eq!(fg_result.quantity, 5);

    // Invariant: component cost == receipt extended (no truncation in this case)
    let receipt_extended = fg_result.unit_cost_minor * fg_result.quantity;
    assert_eq!(receipt_extended, total_cost);
}
