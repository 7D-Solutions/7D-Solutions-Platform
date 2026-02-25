//! Cross-module E2E: SR inbound close → Inventory receipt → Issue → GL COGS
//! No mocks — real SR DB, real Inventory DB, real GL DB.

mod common;

use anyhow::Result;
use chrono::Utc;
use common::{generate_test_tenant, get_gl_pool};
use gl_rs::consumer::gl_inventory_consumer::{
    process_inventory_cogs_posting, ConsumedLayer as GlConsumedLayer,
    ItemIssuedPayload as GlItemIssuedPayload, SourceRef as GlSourceRef,
};
use inventory_rs::domain::{
    issue_service::{process_issue, IssueRequest},
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    receipt_service::{process_receipt, ReceiptRequest},
};
use serial_test::serial;
use shipping_receiving_rs::{
    db::repository::{InsertLineParams, InsertShipmentParams, ShipmentRepository},
    domain::shipments::{ShipmentService, TransitionRequest},
    InventoryIntegration,
};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn get_sr_pool() -> sqlx::PgPool {
    let url = std::env::var("SHIPPING_RECEIVING_DATABASE_URL")
        .or_else(|_| std::env::var("SR_DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://shipping_receiving_user:shipping_receiving_pass\
             @localhost:5454/shipping_receiving_db"
                .to_string()
        });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to SR DB");
    let _ = sqlx::migrate!("../modules/shipping-receiving/db/migrations")
        .run(&pool)
        .await;
    pool
}

async fn get_inventory_pool() -> sqlx::PgPool {
    let url = std::env::var("INVENTORY_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://inventory_user:inventory_pass\
             @localhost:5442/inventory_db"
                .to_string()
        });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to inventory DB");
    let _ = sqlx::migrate!("../modules/inventory/db/migrations")
        .run(&pool)
        .await;
    pool
}

async fn setup_gl_accounts(pool: &sqlx::PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active)
           VALUES
             (gen_random_uuid(), $1, 'COGS', 'Cost of Goods Sold', 'expense', 'debit', true),
             (gen_random_uuid(), $1, 'INVENTORY', 'Inventory Asset', 'asset', 'debit', true)
           ON CONFLICT (tenant_id, code) DO NOTHING"#,
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn setup_open_period(pool: &sqlx::PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO accounting_periods (tenant_id, period_start, period_end, is_closed)
           VALUES ($1, '2026-01-01', '2026-12-31', false)
           ON CONFLICT DO NOTHING"#,
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn cleanup_sr(pool: &sqlx::PgPool, tenant_uuid: Uuid) {
    let tid = tenant_uuid.to_string();
    for q in [
        "DELETE FROM sr_events_outbox WHERE tenant_id = $1",
        "DELETE FROM shipment_lines WHERE tenant_id = $1",
        "DELETE FROM shipments WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_uuid).execute(pool).await.ok();
    }
    sqlx::query("DELETE FROM sr_events_outbox WHERE tenant_id = $1")
        .bind(&tid).execute(pool).await.ok();
}

async fn cleanup_inventory(pool: &sqlx::PgPool, tenant_id: &str) {
    for q in [
        "DELETE FROM inv_outbox WHERE tenant_id = $1",
        "DELETE FROM inv_idempotency_keys WHERE tenant_id = $1",
        "DELETE FROM layer_consumptions WHERE ledger_entry_id IN \
         (SELECT id FROM inventory_ledger WHERE tenant_id = $1)",
        "DELETE FROM inventory_serial_instances WHERE tenant_id = $1",
        "DELETE FROM item_on_hand WHERE tenant_id = $1",
        "DELETE FROM inventory_reservations WHERE tenant_id = $1",
        "DELETE FROM inv_adjustments WHERE tenant_id = $1",
        "DELETE FROM inventory_layers WHERE tenant_id = $1",
        "DELETE FROM inventory_ledger WHERE tenant_id = $1",
        "DELETE FROM inventory_lots WHERE tenant_id = $1",
        "DELETE FROM items WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(pool).await.ok();
    }
}

async fn cleanup_gl(pool: &sqlx::PgPool, tenant_id: &str) {
    for q in [
        "DELETE FROM journal_lines WHERE journal_entry_id IN \
         (SELECT id FROM journal_entries WHERE tenant_id = $1)",
        "DELETE FROM processed_events WHERE event_id IN \
         (SELECT source_event_id FROM journal_entries WHERE tenant_id = $1)",
        "DELETE FROM journal_entries WHERE tenant_id = $1",
        "DELETE FROM account_balances WHERE tenant_id = $1",
        "DELETE FROM period_summary_snapshots WHERE tenant_id = $1",
        "DELETE FROM accounts WHERE tenant_id = $1",
        "DELETE FROM accounting_periods WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(pool).await.ok();
    }
}

fn to_gl_payload(
    issue: &inventory_rs::domain::issue_service::IssueResult,
    sku: &str,
) -> GlItemIssuedPayload {
    GlItemIssuedPayload {
        issue_line_id: issue.issue_line_id,
        tenant_id: issue.tenant_id.clone(),
        item_id: issue.item_id,
        sku: sku.to_string(),
        warehouse_id: issue.warehouse_id,
        quantity: issue.quantity,
        total_cost_minor: issue.total_cost_minor,
        currency: issue.currency.clone(),
        consumed_layers: issue
            .consumed_layers
            .iter()
            .map(|cl| GlConsumedLayer {
                layer_id: cl.layer_id,
                quantity: cl.quantity,
                unit_cost_minor: cl.unit_cost_minor,
                extended_cost_minor: cl.extended_cost_minor,
            })
            .collect(),
        source_ref: GlSourceRef {
            source_module: issue.source_ref.source_module.clone(),
            source_type: issue.source_ref.source_type.clone(),
            source_id: issue.source_ref.source_id.clone(),
            source_line_id: issue.source_ref.source_line_id.clone(),
        },
        issued_at: issue.issued_at,
    }
}

/// Full pipeline: SR inbound close → inventory receipt → issue → GL COGS posting.
#[tokio::test]
#[serial]
async fn sr_inbound_close_inventory_receipt_gl_cogs_full_pipeline() -> Result<()> {
    let sr_pool = get_sr_pool().await;
    let inv_pool = get_inventory_pool().await;
    let gl_pool = get_gl_pool().await;

    let tenant_id = generate_test_tenant();
    let tenant_uuid = Uuid::new_v4();
    let sku = "SKU-SR-E2E-001";
    let warehouse_id = Uuid::new_v4();

    cleanup_sr(&sr_pool, tenant_uuid).await;
    cleanup_inventory(&inv_pool, &tenant_id).await;
    cleanup_gl(&gl_pool, &tenant_id).await;
    setup_gl_accounts(&gl_pool, &tenant_id).await?;
    setup_open_period(&gl_pool, &tenant_id).await?;

    // Step 1: Create inbound shipment
    let shipment = ShipmentRepository::insert_shipment(
        &sr_pool,
        &InsertShipmentParams {
            tenant_id: tenant_uuid,
            direction: "inbound".to_string(),
            status: "draft".to_string(),
            carrier_party_id: None,
            tracking_number: Some("TRACK-E2E-001".to_string()),
            freight_cost_minor: Some(500),
            currency: Some("usd".to_string()),
            expected_arrival_date: None,
            created_by: None,
            source_ref_type: None,
            source_ref_id: None,
        },
    )
    .await
    .expect("insert shipment");
    assert_eq!(shipment.status, "draft");
    let shipment_id = shipment.id;

    let line = ShipmentRepository::insert_line(
        &sr_pool,
        &InsertLineParams {
            tenant_id: tenant_uuid,
            shipment_id,
            sku: Some(sku.to_string()),
            uom: Some("each".to_string()),
            warehouse_id: Some(warehouse_id),
            qty_expected: 50,
            source_ref_type: None,
            source_ref_id: None,
            po_id: None,
            po_line_id: None,
        },
    )
    .await
    .expect("insert line");
    let line_id = line.id;

    // Step 2: Walk shipment through state machine
    let inventory = InventoryIntegration::deterministic();
    for (target, arrived, closed) in [
        ("confirmed", None, None),
        ("in_transit", None, None),
        ("arrived", Some(Utc::now()), None),
        ("receiving", None, None),
    ] {
        ShipmentService::transition(
            &sr_pool, shipment_id, tenant_uuid,
            &TransitionRequest {
                status: target.to_string(),
                arrived_at: arrived,
                shipped_at: None,
                delivered_at: None,
                closed_at: closed,
            },
            &inventory,
        )
        .await
        .unwrap_or_else(|e| panic!("transition to {target} failed: {e}"));
    }

    // Step 3: Set qty_received/accepted, then close
    sqlx::query(
        "UPDATE shipment_lines SET qty_received = 50, qty_accepted = 50, qty_rejected = 0 \
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(line_id)
    .bind(tenant_uuid)
    .execute(&sr_pool)
    .await
    .expect("update line qty");

    let closed = ShipmentService::transition(
        &sr_pool, shipment_id, tenant_uuid,
        &TransitionRequest {
            status: "closed".to_string(),
            arrived_at: None,
            shipped_at: None,
            delivered_at: None,
            closed_at: Some(Utc::now()),
        },
        &inventory,
    )
    .await
    .expect("receiving → closed");
    assert_eq!(closed.status, "closed");
    assert!(closed.closed_at.is_some());

    // Verify inventory_ref_id was set on the line
    let inv_ref: Option<Uuid> = sqlx::query_scalar(
        "SELECT inventory_ref_id FROM shipment_lines WHERE id = $1 AND tenant_id = $2",
    )
    .bind(line_id)
    .bind(tenant_uuid)
    .fetch_one(&sr_pool)
    .await
    .expect("fetch inv ref");
    assert!(inv_ref.is_some(), "inventory_ref_id must be set after close");

    // Verify outbox event
    let (outbox_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sr_events_outbox \
         WHERE aggregate_id = $1 AND event_type = 'shipping.inbound.closed'",
    )
    .bind(shipment_id.to_string())
    .fetch_one(&sr_pool)
    .await
    .expect("outbox count");
    assert_eq!(outbox_count, 1, "inbound.closed event must be in outbox");

    // Step 4: Create inventory item + receipt
    let item = ItemRepo::create(
        &inv_pool,
        &CreateItemRequest {
            tenant_id: tenant_id.clone(),
            sku: sku.to_string(),
            name: "SR E2E Test Item".to_string(),
            description: None,
            inventory_account_ref: "INVENTORY".to_string(),
            cogs_account_ref: "COGS".to_string(),
            variance_account_ref: "COGS".to_string(),
            uom: None,
            tracking_mode: TrackingMode::None,
        },
    )
    .await
    .expect("create inventory item");

    let (receipt, is_replay) = process_receipt(
        &inv_pool,
        &ReceiptRequest {
            tenant_id: tenant_id.clone(),
            item_id: item.id,
            warehouse_id,
            location_id: None,
            quantity: 50,
            unit_cost_minor: 2000,
            currency: "usd".to_string(),
            purchase_order_id: None,
            idempotency_key: format!("sr-e2e-rcpt-{shipment_id}"),
            correlation_id: Some(format!("sr-e2e-{shipment_id}")),
            causation_id: None,
            lot_code: None,
            serial_codes: None,
            uom_id: None,
        },
    )
    .await
    .expect("inventory receipt");
    assert!(!is_replay, "must be a fresh receipt");
    assert_eq!(receipt.quantity, 50);

    // Step 5: Verify stock levels
    let on_hand: i64 = sqlx::query_scalar(
        "SELECT quantity_on_hand FROM item_on_hand \
         WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .bind(warehouse_id)
    .fetch_one(&inv_pool)
    .await
    .expect("on-hand row");
    assert_eq!(on_hand, 50, "on-hand must be 50 after receipt");

    // Step 6: Issue 50 units
    let (issue, is_replay) = process_issue(
        &inv_pool,
        &IssueRequest {
            tenant_id: tenant_id.clone(),
            item_id: item.id,
            warehouse_id,
            location_id: None,
            quantity: 50,
            currency: "usd".to_string(),
            source_module: "shipping-receiving".to_string(),
            source_type: "shipment".to_string(),
            source_id: shipment_id.to_string(),
            source_line_id: Some(line_id.to_string()),
            idempotency_key: format!("sr-e2e-issue-{shipment_id}"),
            correlation_id: Some(format!("sr-e2e-{shipment_id}")),
            causation_id: None,
            uom_id: None,
            lot_code: None,
            serial_codes: None,
        },
    )
    .await
    .expect("inventory issue");
    assert!(!is_replay);
    assert_eq!(issue.quantity, 50);
    assert_eq!(issue.total_cost_minor, 100_000, "50 × $20.00 = $1000.00");
    assert_eq!(issue.consumed_layers.len(), 1, "single FIFO layer consumed");

    let on_hand_after: i64 = sqlx::query_scalar(
        "SELECT quantity_on_hand FROM item_on_hand \
         WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .bind(warehouse_id)
    .fetch_one(&inv_pool)
    .await
    .expect("on-hand after issue");
    assert_eq!(on_hand_after, 0, "on-hand must be 0 after full issue");

    // Step 7: GL COGS posting
    let gl_payload = to_gl_payload(&issue, sku);
    let gl_event_id = issue.event_id;
    let entry_id = process_inventory_cogs_posting(
        &gl_pool, gl_event_id, &tenant_id, "inventory", &gl_payload,
    )
    .await
    .expect("GL COGS posting");

    // Step 8: Verify GL journal entry
    let (je_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM journal_entries WHERE source_event_id = $1",
    )
    .bind(gl_event_id)
    .fetch_one(&gl_pool)
    .await?;
    assert_eq!(je_count, 1, "exactly one GL journal entry");

    let lines: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT account_ref, COALESCE(debit_minor, 0), COALESCE(credit_minor, 0) \
         FROM journal_lines WHERE journal_entry_id = $1 ORDER BY line_no",
    )
    .bind(entry_id)
    .fetch_all(&gl_pool)
    .await?;
    assert_eq!(lines.len(), 2, "exactly 2 journal lines");

    let cogs = lines.iter().find(|(a, _, _)| a == "COGS").expect("COGS line");
    let inv = lines.iter().find(|(a, _, _)| a == "INVENTORY").expect("INV line");
    assert_eq!(cogs.1, 100_000, "COGS debit = $1000.00");
    assert_eq!(cogs.2, 0);
    assert_eq!(inv.1, 0);
    assert_eq!(inv.2, 100_000, "INVENTORY credit = $1000.00");

    let total_dr: i64 = lines.iter().map(|(_, d, _)| d).sum();
    let total_cr: i64 = lines.iter().map(|(_, _, c)| c).sum();
    assert_eq!(total_dr, total_cr, "journal must be balanced");

    // Exactly-once guard
    let (pe_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM processed_events WHERE event_id = $1",
    )
    .bind(gl_event_id)
    .fetch_one(&gl_pool)
    .await?;
    assert_eq!(pe_count, 1, "exactly one processed_events row");

    // Idempotency check
    let second = process_inventory_cogs_posting(
        &gl_pool, gl_event_id, &tenant_id, "inventory", &gl_payload,
    )
    .await;
    assert!(
        matches!(second, Err(gl_rs::services::journal_service::JournalError::DuplicateEvent(_))),
        "second GL post must return DuplicateEvent, got: {second:?}",
    );

    cleanup_sr(&sr_pool, tenant_uuid).await;
    cleanup_inventory(&inv_pool, &tenant_id).await;
    cleanup_gl(&gl_pool, &tenant_id).await;
    Ok(())
}
