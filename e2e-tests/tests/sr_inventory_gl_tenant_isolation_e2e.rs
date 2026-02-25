//! Cross-tenant isolation: inventory + GL data for tenant A must not leak to tenant B.

mod common;

use anyhow::Result;
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
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

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

/// Tenant A's inventory + GL data must not be visible to tenant B.
#[tokio::test]
#[serial]
async fn sr_inventory_gl_cross_tenant_isolation() -> Result<()> {
    let inv_pool = get_inventory_pool().await;
    let gl_pool = get_gl_pool().await;

    let tenant_a = generate_test_tenant();
    let tenant_b = generate_test_tenant();

    cleanup_inventory(&inv_pool, &tenant_a).await;
    cleanup_inventory(&inv_pool, &tenant_b).await;
    cleanup_gl(&gl_pool, &tenant_a).await;
    cleanup_gl(&gl_pool, &tenant_b).await;

    setup_gl_accounts(&gl_pool, &tenant_a).await?;
    setup_open_period(&gl_pool, &tenant_a).await?;

    let warehouse_id = Uuid::new_v4();
    let item = ItemRepo::create(
        &inv_pool,
        &CreateItemRequest {
            tenant_id: tenant_a.clone(),
            sku: "SKU-ISO-001".to_string(),
            name: "Isolation Test Item".to_string(),
            description: None,
            inventory_account_ref: "INVENTORY".to_string(),
            cogs_account_ref: "COGS".to_string(),
            variance_account_ref: "COGS".to_string(),
            uom: None,
            tracking_mode: TrackingMode::None,
        },
    )
    .await
    .expect("create item");

    process_receipt(
        &inv_pool,
        &ReceiptRequest {
            tenant_id: tenant_a.clone(),
            item_id: item.id,
            warehouse_id,
            location_id: None,
            quantity: 20,
            unit_cost_minor: 1000,
            currency: "usd".to_string(),
            purchase_order_id: None,
            idempotency_key: format!("iso-rcpt-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
            lot_code: None,
            serial_codes: None,
            uom_id: None,
        },
    )
    .await
    .expect("receipt");

    let (issue, _) = process_issue(
        &inv_pool,
        &IssueRequest {
            tenant_id: tenant_a.clone(),
            item_id: item.id,
            warehouse_id,
            location_id: None,
            quantity: 20,
            currency: "usd".to_string(),
            source_module: "shipping-receiving".to_string(),
            source_type: "shipment".to_string(),
            source_id: "ISO-SHIP-001".to_string(),
            source_line_id: None,
            idempotency_key: format!("iso-issue-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
            uom_id: None,
            lot_code: None,
            serial_codes: None,
        },
    )
    .await
    .expect("issue");

    let gl_payload = to_gl_payload(&issue, "SKU-ISO-001");
    process_inventory_cogs_posting(
        &gl_pool, issue.event_id, &tenant_a, "inventory", &gl_payload,
    )
    .await
    .expect("GL posting for tenant A");

    // Tenant B must have zero journal entries
    let (b_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1",
    )
    .bind(&tenant_b)
    .fetch_one(&gl_pool)
    .await?;
    assert_eq!(b_count, 0, "tenant B must not see tenant A's GL entries");

    // Tenant B must have zero inventory items
    let (b_items,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM items WHERE tenant_id = $1",
    )
    .bind(&tenant_b)
    .fetch_one(&inv_pool)
    .await?;
    assert_eq!(b_items, 0, "tenant B must not see tenant A's items");

    cleanup_inventory(&inv_pool, &tenant_a).await;
    cleanup_inventory(&inv_pool, &tenant_b).await;
    cleanup_gl(&gl_pool, &tenant_a).await;
    cleanup_gl(&gl_pool, &tenant_b).await;
    Ok(())
}
