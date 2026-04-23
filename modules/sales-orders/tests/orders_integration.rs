//! Integration tests for the sales-orders module.
//!
//! Requires: 7d-sales-orders-postgres running on localhost:5467
//! Optional: 7d-inventory running on localhost:8092 (for reservation tests)
//!
//! Run: DATABASE_URL=... cargo test -p sales-orders-rs --test orders_integration

use sales_orders_rs::domain::blankets::{
    service as blanket_service, CreateBlanketLineRequest, CreateBlanketRequest,
    CreateReleaseRequest,
};
use sales_orders_rs::domain::orders::{
    service, CancelOrderRequest, CreateOrderLineRequest, CreateOrderRequest, SoStatus,
    UpdateOrderLineRequest,
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://so_user:so_pass@localhost:5467/so_db".to_string());
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
        .expect("Failed to connect to SO test DB — is 7d-sales-orders-postgres running on :5467?");
    MIGRATOR
        .run(&pool)
        .await
        .expect("Failed to run SO migrations");
    pool
}

fn unique_tenant() -> String {
    format!("so-test-{}", Uuid::new_v4().simple())
}

fn base_order_req() -> CreateOrderRequest {
    CreateOrderRequest {
        customer_id: Some(Uuid::new_v4()),
        party_id: None,
        currency: "USD".to_string(),
        order_date: None,
        required_date: None,
        promised_date: None,
        external_quote_ref: None,
        notes: None,
    }
}

fn line_req(qty: f64, unit_price_cents: i64) -> CreateOrderLineRequest {
    CreateOrderLineRequest {
        item_id: None,
        part_number: None,
        description: "Test Widget".to_string(),
        uom: Some("EA".to_string()),
        quantity: qty,
        unit_price_cents,
        required_date: None,
        promised_date: None,
        warehouse_id: None,
        notes: None,
    }
}

fn corr() -> String {
    Uuid::new_v4().to_string()
}

// ── Test 1: Book order with 0 lines is rejected ───────────────────────────────

#[tokio::test]
#[serial]
async fn book_order_empty_lines_rejected() {
    let pool = setup_db().await;
    let tenant = unique_tenant();

    let order = service::create_order(&pool, &tenant, "test-user", base_order_req())
        .await
        .expect("create order");

    let result = service::book_order(&pool, &tenant, order.id, corr(), None).await;
    assert!(
        matches!(
            result,
            Err(sales_orders_rs::domain::orders::OrderError::EmptyLines)
        ),
        "Expected EmptyLines error, got: {:?}",
        result
    );

    // Order status unchanged
    let with_lines = service::get_order_with_lines(&pool, &tenant, order.id)
        .await
        .expect("get order");
    assert_eq!(with_lines.order.status, "draft");
}

// ── Test 2: Line total invariants ─────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn line_total_invariants() {
    let pool = setup_db().await;
    let tenant = unique_tenant();

    let order = service::create_order(&pool, &tenant, "test-user", base_order_req())
        .await
        .expect("create order");

    // qty=3, price=$10 → line_total=3000
    let line = service::add_line(&pool, &tenant, order.id, line_req(3.0, 1000))
        .await
        .expect("add line");
    assert_eq!(line.line_total_cents, 3000, "3 * 1000 = 3000");

    // update qty to 5 → line_total=5000
    let updated = service::update_line(
        &pool,
        &tenant,
        order.id,
        line.id,
        UpdateOrderLineRequest {
            item_id: None,
            part_number: None,
            description: None,
            uom: None,
            quantity: Some(5.0),
            unit_price_cents: None,
            required_date: None,
            promised_date: None,
            warehouse_id: None,
            notes: None,
        },
    )
    .await
    .expect("update line");
    assert_eq!(updated.line_total_cents, 5000, "5 * 1000 = 5000");

    // Header totals should reflect the line
    let with_lines = service::get_order_with_lines(&pool, &tenant, order.id)
        .await
        .expect("get order");
    assert_eq!(with_lines.order.subtotal_cents, 5000);
    assert_eq!(with_lines.order.total_cents, 5000);
}

// ── Test 3: Book order without inventory → status=booked ─────────────────────

#[tokio::test]
#[serial]
async fn book_order_no_inventory_transitions_to_booked() {
    let pool = setup_db().await;
    let tenant = unique_tenant();

    let order = service::create_order(&pool, &tenant, "test-user", base_order_req())
        .await
        .expect("create order");
    service::add_line(&pool, &tenant, order.id, line_req(2.0, 500))
        .await
        .expect("add line");

    let booked = service::book_order(&pool, &tenant, order.id, corr(), None)
        .await
        .expect("book order");
    assert_eq!(booked.status, "booked");
}

// ── Test 4: Line edit rejected after booking ──────────────────────────────────

#[tokio::test]
#[serial]
async fn line_edit_rejected_after_booking() {
    let pool = setup_db().await;
    let tenant = unique_tenant();

    let order = service::create_order(&pool, &tenant, "test-user", base_order_req())
        .await
        .expect("create order");
    let line = service::add_line(&pool, &tenant, order.id, line_req(1.0, 2000))
        .await
        .expect("add line");
    service::book_order(&pool, &tenant, order.id, corr(), None)
        .await
        .expect("book order");

    let result = service::update_line(
        &pool,
        &tenant,
        order.id,
        line.id,
        UpdateOrderLineRequest {
            item_id: None,
            part_number: None,
            description: None,
            uom: None,
            quantity: Some(5.0),
            unit_price_cents: None,
            required_date: None,
            promised_date: None,
            warehouse_id: None,
            notes: None,
        },
    )
    .await;
    assert!(
        matches!(
            result,
            Err(sales_orders_rs::domain::orders::OrderError::NotDraft(_))
        ),
        "Expected NotDraft error, got: {:?}",
        result
    );
}

// ── Test 5: Cancel draft order ────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn cancel_draft_order() {
    let pool = setup_db().await;
    let tenant = unique_tenant();

    let order = service::create_order(&pool, &tenant, "test-user", base_order_req())
        .await
        .expect("create order");
    service::add_line(&pool, &tenant, order.id, line_req(1.0, 100))
        .await
        .expect("add line");

    let cancelled = service::cancel_order(
        &pool,
        &tenant,
        order.id,
        CancelOrderRequest {
            reason: Some("test cancel".to_string()),
        },
        corr(),
        None,
    )
    .await
    .expect("cancel order");
    assert_eq!(cancelled.status, "cancelled");
}

// ── Test 6: Invalid state transitions rejected ────────────────────────────────

#[tokio::test]
#[serial]
async fn invalid_transition_rejected() {
    let pool = setup_db().await;
    let tenant = unique_tenant();

    let order = service::create_order(&pool, &tenant, "test-user", base_order_req())
        .await
        .expect("create order");

    // Cannot transition Draft → Shipped
    let result = service::transition_order(&pool, &tenant, order.id, SoStatus::Shipped).await;
    assert!(
        matches!(
            result,
            Err(sales_orders_rs::domain::orders::OrderError::InvalidTransition { .. })
        ),
        "Expected InvalidTransition, got: {:?}",
        result
    );
}

// ── Test 7: Tenant isolation ──────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn tenant_isolation() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    let order = service::create_order(&pool, &tenant_a, "user-a", base_order_req())
        .await
        .expect("create order in tenant A");

    // Tenant B cannot see tenant A's order
    let result = service::get_order_with_lines(&pool, &tenant_b, order.id).await;
    assert!(
        matches!(
            result,
            Err(sales_orders_rs::domain::orders::OrderError::NotFound(_))
        ),
        "Expected NotFound for cross-tenant access, got: {:?}",
        result
    );
}

// ── Test 8: Blanket release quota enforcement ─────────────────────────────────

#[tokio::test]
#[serial]
async fn blanket_release_quota_enforced() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let customer_id = Uuid::new_v4();
    let item_id = Uuid::new_v4();

    let blanket = blanket_service::create_blanket(
        &pool,
        &tenant,
        "test-user",
        CreateBlanketRequest {
            customer_id: Some(customer_id),
            party_id: None,
            currency: "USD".to_string(),
            effective_date: None,
            expiry_date: None,
            notes: None,
        },
    )
    .await
    .expect("create blanket");

    let bl_line = blanket_service::add_blanket_line(
        &pool,
        &tenant,
        blanket.id,
        CreateBlanketLineRequest {
            item_id: Some(item_id),
            part_number: None,
            description: "Widget A".to_string(),
            uom: Some("EA".to_string()),
            committed_qty: 100.0,
            unit_price_cents: 500,
            notes: None,
        },
    )
    .await
    .expect("add blanket line");

    // Activate the blanket
    blanket_service::activate_blanket(
        &pool,
        &tenant,
        blanket.id,
        sales_orders_rs::domain::blankets::ActivateBlanketRequest { reason: None },
    )
    .await
    .expect("activate blanket");

    // Create releases totaling 100 → OK
    let release = blanket_service::create_release(
        &pool,
        &tenant,
        blanket.id,
        CreateReleaseRequest {
            blanket_line_id: bl_line.id,
            release_qty: 100.0,
            release_date: None,
            notes: None,
        },
    )
    .await
    .expect("release 100 units should succeed");
    assert_eq!(release.release_qty, 100.0);

    // Next release should be rejected (over quota)
    let result = blanket_service::create_release(
        &pool,
        &tenant,
        blanket.id,
        CreateReleaseRequest {
            blanket_line_id: bl_line.id,
            release_qty: 1.0,
            release_date: None,
            notes: None,
        },
    )
    .await;
    assert!(
        result.is_err(),
        "Release beyond committed_qty should be rejected, got Ok"
    );
}

// ── Test 9: Concurrent releases — no over-draw ───────────────────────────────

#[tokio::test]
#[serial]
async fn concurrent_releases_no_overdraw() {
    let pool = setup_db().await;
    let tenant = unique_tenant();

    let blanket = blanket_service::create_blanket(
        &pool,
        &tenant,
        "user",
        CreateBlanketRequest {
            customer_id: None,
            party_id: None,
            currency: "USD".to_string(),
            effective_date: None,
            expiry_date: None,
            notes: None,
        },
    )
    .await
    .expect("create blanket");

    let bl_line = blanket_service::add_blanket_line(
        &pool,
        &tenant,
        blanket.id,
        CreateBlanketLineRequest {
            item_id: None,
            part_number: None,
            description: "Concurrent test item".to_string(),
            uom: None,
            committed_qty: 10.0,
            unit_price_cents: 100,
            notes: None,
        },
    )
    .await
    .expect("add blanket line");

    blanket_service::activate_blanket(
        &pool,
        &tenant,
        blanket.id,
        sales_orders_rs::domain::blankets::ActivateBlanketRequest { reason: None },
    )
    .await
    .expect("activate blanket");

    // Fire 5 concurrent releases of 3 each — only 3 can fit (3+3+3+... would overdraw 10)
    let pool_clone = pool.clone();
    let tenant_clone = tenant.clone();
    let bl_id = blanket.id;
    let bll_id = bl_line.id;

    let handles: Vec<_> = (0..5)
        .map(|_| {
            let p = pool_clone.clone();
            let t = tenant_clone.clone();
            tokio::spawn(async move {
                blanket_service::create_release(
                    &p,
                    &t,
                    bl_id,
                    CreateReleaseRequest {
                        blanket_line_id: bll_id,
                        release_qty: 3.0,
                        release_date: None,
                        notes: None,
                    },
                )
                .await
            })
        })
        .collect();

    let mut success_count = 0usize;
    let mut fail_count = 0usize;
    for h in handles {
        match h.await.expect("task panicked") {
            Ok(_) => success_count += 1,
            Err(_) => fail_count += 1,
        }
    }

    // At most 3 releases of 3 can succeed (9 <= 10); the 4th would need 12 > 10
    assert!(
        success_count <= 3,
        "Expected at most 3 successful releases of 3 from committed_qty=10, got {}",
        success_count
    );
    assert!(
        success_count + fail_count == 5,
        "All 5 concurrent tasks must complete"
    );
    assert!(
        fail_count >= 2,
        "At least 2 must be rejected by quota guard"
    );
}

// ── Test 10: Book order with inventory (requires 7d-inventory on :8092) ───────

#[tokio::test]
#[serial]
async fn book_order_with_inventory_in_fulfillment() {
    let inv_url = match std::env::var("INVENTORY_BASE_URL")
        .ok()
        .filter(|s| !s.is_empty())
    {
        Some(u) => u,
        None => {
            eprintln!("SKIP: INVENTORY_BASE_URL not set; skipping inventory reservation test");
            return;
        }
    };

    let pool = setup_db().await;
    let tenant = unique_tenant();

    // Create item + stock in Inventory first via its API
    // (This requires a valid service JWT; the service_jwt helper in service.rs handles it)
    // For this test we create a line WITHOUT item_id (so no reservation needed)
    // but use inv_url to confirm the no-reservable-lines path still advances to in_fulfillment
    let order = service::create_order(&pool, &tenant, "test-user", base_order_req())
        .await
        .expect("create order");

    // Line without item_id/warehouse_id → not reservable but still counts as a line
    service::add_line(&pool, &tenant, order.id, line_req(1.0, 1000))
        .await
        .expect("add line");

    let booked = service::book_order(&pool, &tenant, order.id, corr(), Some(&inv_url))
        .await
        .expect("book order");

    // With inv_url set but no reservable lines → in_fulfillment
    assert_eq!(
        booked.status, "in_fulfillment",
        "Order with inv_url but no reservable lines should be in_fulfillment"
    );
}

// ── Test 11: Consumer: shipped_qty update and invoiced_at ────────────────────

#[tokio::test]
#[serial]
async fn repo_update_line_shipped_qty_and_invoiced() {
    use sales_orders_rs::domain::orders::repo;

    let pool = setup_db().await;
    let tenant = unique_tenant();

    let order = service::create_order(&pool, &tenant, "test-user", base_order_req())
        .await
        .expect("create order");
    let line = service::add_line(&pool, &tenant, order.id, line_req(5.0, 200))
        .await
        .expect("add line");

    // Simulate shipment_shipped consumer updating shipped_qty
    repo::update_line_shipped_qty(&pool, line.id, &tenant, 3.0)
        .await
        .expect("update shipped_qty");

    // Simulate shipment_shipped marking invoiced
    repo::mark_line_invoiced(&pool, line.id, &tenant)
        .await
        .expect("mark invoiced");

    let with_lines = service::get_order_with_lines(&pool, &tenant, order.id)
        .await
        .expect("get order");
    let updated_line = &with_lines.lines[0];
    assert_eq!(updated_line.shipped_qty, 3.0);
    assert!(
        updated_line.invoiced_at.is_some(),
        "invoiced_at should be set"
    );
}

// ── Test 12: Order closed when all lines invoiced ────────────────────────────

#[tokio::test]
#[serial]
async fn invoice_issued_closes_order_when_all_lines_invoiced() {
    use sales_orders_rs::domain::orders::{repo, SoStatus};

    let pool = setup_db().await;
    let tenant = unique_tenant();

    let order = service::create_order(&pool, &tenant, "test-user", base_order_req())
        .await
        .expect("create order");
    let line = service::add_line(&pool, &tenant, order.id, line_req(1.0, 100))
        .await
        .expect("add line");

    // Advance order to shipped state
    service::book_order(&pool, &tenant, order.id, corr(), None)
        .await
        .expect("book");
    service::transition_order(&pool, &tenant, order.id, SoStatus::InFulfillment)
        .await
        .expect("to in_fulfillment");
    service::transition_order(&pool, &tenant, order.id, SoStatus::Shipped)
        .await
        .expect("to shipped");

    // Mark the single line invoiced
    repo::mark_line_invoiced(&pool, line.id, &tenant)
        .await
        .expect("mark invoiced");

    // Simulate invoice_issued consumer: check all invoiced → close
    let lines = repo::fetch_lines_for_order(&pool, order.id, &tenant)
        .await
        .expect("fetch lines");
    let all_invoiced = !lines.is_empty() && lines.iter().all(|l| l.invoiced_at.is_some());
    assert!(all_invoiced);

    if all_invoiced {
        let current_order = repo::fetch_order_for_mutation(&pool, order.id, &tenant)
            .await
            .expect("fetch order")
            .expect("order exists");
        let current = SoStatus::from_str(&current_order.status).unwrap_or(SoStatus::Shipped);
        if current.can_transition_to(SoStatus::Closed) {
            repo::update_order_status(&pool, order.id, &tenant, SoStatus::Closed.as_str())
                .await
                .expect("close order");
        }
    }

    let final_order = service::get_order_with_lines(&pool, &tenant, order.id)
        .await
        .expect("get order");
    assert_eq!(final_order.order.status, "closed");
}
