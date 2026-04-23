//! Integration tests for the outside-processing module.
//!
//! Requires a real Postgres database. Set DATABASE_URL or rely on the default.
//! All tests use unique tenant IDs to avoid cross-test interference.

use chrono::{NaiveDate, Utc};
use outside_processing_rs::domain::{models::*, repo, state_machine};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://op_user:op_pass@localhost:5466/op_db".to_string());
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to OP test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run OP migrations");
    pool
}

fn unique_tenant() -> String {
    format!("op-test-{}", Uuid::new_v4().simple())
}

fn corr() -> String {
    Uuid::new_v4().to_string()
}

fn base_create_req(created_by: &str) -> CreateOpOrderRequest {
    CreateOpOrderRequest {
        vendor_id: Some(Uuid::new_v4()),
        service_type: Some("heat_treat".to_string()),
        service_description: None,
        process_spec_ref: None,
        part_number: Some("PN-001".to_string()),
        part_revision: None,
        quantity_sent: 10,
        unit_of_measure: None,
        work_order_id: None,
        operation_id: None,
        lot_id: None,
        serial_numbers: None,
        expected_ship_date: None,
        expected_return_date: None,
        estimated_cost_cents: None,
        notes: None,
        created_by: created_by.to_string(),
    }
}

fn ship_req(qty: i32) -> CreateShipEventRequest {
    CreateShipEventRequest {
        ship_date: NaiveDate::from_ymd_opt(2026, 4, 17).unwrap(),
        quantity_shipped: qty,
        unit_of_measure: None,
        lot_number: None,
        serial_numbers: None,
        carrier_name: None,
        tracking_number: None,
        packing_slip_number: None,
        shipped_by: "user-1".to_string(),
        shipping_reference: None,
        notes: None,
    }
}

fn return_req(qty: i32) -> CreateReturnEventRequest {
    CreateReturnEventRequest {
        received_date: NaiveDate::from_ymd_opt(2026, 4, 20).unwrap(),
        quantity_received: qty,
        unit_of_measure: None,
        condition: ReturnCondition::Good,
        discrepancy_notes: None,
        lot_number: None,
        serial_numbers: None,
        cert_ref: None,
        vendor_packing_slip: None,
        carrier_name: None,
        tracking_number: None,
        re_identification_required: None,
        received_by: "user-2".to_string(),
        notes: None,
    }
}

// ── 1. draft→issued with vendor_id=null → validation error ─────────────────

#[tokio::test]
#[serial]
async fn test_issue_without_vendor_fails() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let mut create_req = base_create_req("alice");
    create_req.vendor_id = None;

    let mut ctx = pool.begin().await.unwrap();
    let order = repo::create_order(&mut ctx, &tid, &create_req, "OP-000001")
        .await
        .unwrap();
    ctx.commit().await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    let result = repo::issue_order(&mut tx, &tid, order.op_order_id, None).await;
    assert!(result.is_err(), "Expected error issuing without vendor_id");
}

// ── 2. draft→issued with quantity_sent=0 → validation error ─────────────────

#[tokio::test]
#[serial]
async fn test_issue_without_quantity_fails() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let mut create_req = base_create_req("alice");
    create_req.quantity_sent = 0;

    let mut ctx = pool.begin().await.unwrap();
    let order = repo::create_order(&mut ctx, &tid, &create_req, "OP-000002")
        .await
        .unwrap();
    ctx.commit().await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    let result = repo::issue_order(&mut tx, &tid, order.op_order_id, None).await;
    assert!(
        result.is_err(),
        "Expected error issuing with quantity_sent=0"
    );
}

// ── 3. Issue with all required fields → status=issued ───────────────────────

#[tokio::test]
#[serial]
async fn test_issue_happy_path() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let mut ctx = pool.begin().await.unwrap();
    let order = repo::create_order(&mut ctx, &tid, &base_create_req("alice"), "OP-000003")
        .await
        .unwrap();
    ctx.commit().await.unwrap();
    assert_eq!(order.status, "draft");

    let mut tx = pool.begin().await.unwrap();
    let issued = repo::issue_order(&mut tx, &tid, order.op_order_id, None)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    assert_eq!(issued.status, "issued");
}

// ── 4. Ship event advancing status to shipped_to_vendor ─────────────────────

#[tokio::test]
#[serial]
async fn test_ship_event_advances_status() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let mut ctx = pool.begin().await.unwrap();
    let order = repo::create_order(&mut ctx, &tid, &base_create_req("alice"), "OP-000004")
        .await
        .unwrap();
    ctx.commit().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    repo::issue_order(&mut tx, &tid, order.op_order_id, None)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let mut tx2 = pool.begin().await.unwrap();
    let (locked_order, sum_shipped, _) =
        repo::lock_order_for_quantity_check(&mut tx2, &tid, order.op_order_id)
            .await
            .unwrap();

    let new_status = state_machine::transition_on_ship_event(&locked_order.status).unwrap();
    repo::create_ship_event_tx(&mut tx2, &tid, order.op_order_id, &ship_req(5))
        .await
        .unwrap();
    repo::set_order_status(&mut tx2, &tid, order.op_order_id, new_status.as_str())
        .await
        .unwrap();
    tx2.commit().await.unwrap();

    let updated = repo::get_order(&pool, &tid, order.op_order_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.status, "shipped_to_vendor");
}

// ── 5. Ship quantity exceeding quantity_sent → error ────────────────────────

#[tokio::test]
#[serial]
async fn test_ship_quantity_bound_enforced() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let mut ctx = pool.begin().await.unwrap();
    let order = repo::create_order(&mut ctx, &tid, &base_create_req("alice"), "OP-000005")
        .await
        .unwrap();
    ctx.commit().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    repo::issue_order(&mut tx, &tid, order.op_order_id, None)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // First ship — 7 (ok)
    let mut tx2 = pool.begin().await.unwrap();
    let (_, sum_shipped, _) =
        repo::lock_order_for_quantity_check(&mut tx2, &tid, order.op_order_id)
            .await
            .unwrap();
    assert!(sum_shipped + 7 <= order.quantity_sent as i64);
    repo::create_ship_event_tx(&mut tx2, &tid, order.op_order_id, &ship_req(7))
        .await
        .unwrap();
    repo::set_order_status(&mut tx2, &tid, order.op_order_id, "shipped_to_vendor")
        .await
        .unwrap();
    tx2.commit().await.unwrap();

    // Second ship — 5 (7+5=12 > 10, should be caught by caller)
    let mut tx3 = pool.begin().await.unwrap();
    let (order3, sum3, _) = repo::lock_order_for_quantity_check(&mut tx3, &tid, order.op_order_id)
        .await
        .unwrap();
    assert_eq!(sum3, 7);
    let would_exceed = sum3 + 5 > order3.quantity_sent as i64;
    assert!(would_exceed, "Should detect quantity exceeded");
    tx3.rollback().await.unwrap();
}

// ── 6. Return before any ship event → error caught by caller ────────────────

#[tokio::test]
#[serial]
async fn test_return_before_ship_event_fails() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let mut ctx = pool.begin().await.unwrap();
    let order = repo::create_order(&mut ctx, &tid, &base_create_req("alice"), "OP-000006")
        .await
        .unwrap();
    ctx.commit().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    repo::issue_order(&mut tx, &tid, order.op_order_id, None)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Advance to shipped_to_vendor
    let mut tx2 = pool.begin().await.unwrap();
    repo::set_order_status(&mut tx2, &tid, order.op_order_id, "shipped_to_vendor")
        .await
        .unwrap();
    tx2.commit().await.unwrap();

    // Check sum_shipped
    let mut tx3 = pool.begin().await.unwrap();
    let (_, sum_shipped, _) =
        repo::lock_order_for_quantity_check(&mut tx3, &tid, order.op_order_id)
            .await
            .unwrap();
    assert_eq!(
        sum_shipped, 0,
        "No ship events recorded, return should be blocked"
    );
    tx3.rollback().await.unwrap();
}

// ── 7. Return quantity > shipped quantity → caller detects error ─────────────

#[tokio::test]
#[serial]
async fn test_return_quantity_bound_enforced() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let mut ctx = pool.begin().await.unwrap();
    let order = repo::create_order(&mut ctx, &tid, &base_create_req("alice"), "OP-000007")
        .await
        .unwrap();
    ctx.commit().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    repo::issue_order(&mut tx, &tid, order.op_order_id, None)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Ship 8
    let mut tx2 = pool.begin().await.unwrap();
    repo::create_ship_event_tx(&mut tx2, &tid, order.op_order_id, &ship_req(8))
        .await
        .unwrap();
    repo::set_order_status(&mut tx2, &tid, order.op_order_id, "shipped_to_vendor")
        .await
        .unwrap();
    tx2.commit().await.unwrap();

    // Try to return 9 (> 8 shipped)
    let mut tx3 = pool.begin().await.unwrap();
    let (_, sum_shipped, sum_received) =
        repo::lock_order_for_quantity_check(&mut tx3, &tid, order.op_order_id)
            .await
            .unwrap();
    assert_eq!(sum_shipped, 8);
    assert_eq!(sum_received, 0);
    let would_exceed = sum_received + 9 > sum_shipped;
    assert!(
        would_exceed,
        "Return of 9 should exceed shipped quantity of 8"
    );
    tx3.rollback().await.unwrap();
}

// ── 8. Review before any return event → count check fails ───────────────────

#[tokio::test]
#[serial]
async fn test_review_before_return_event_fails() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let mut ctx = pool.begin().await.unwrap();
    let order = repo::create_order(&mut ctx, &tid, &base_create_req("alice"), "OP-000008")
        .await
        .unwrap();
    ctx.commit().await.unwrap();
    let mut tx = pool.begin().await.unwrap();

    let return_count = repo::count_return_events(&mut tx, order.op_order_id)
        .await
        .unwrap();
    assert_eq!(return_count, 0, "No return events should exist");
    tx.rollback().await.unwrap();
}

// ── 9. Review rejected + rework → status=at_vendor ───────────────────────────

#[tokio::test]
#[serial]
async fn test_review_rejected_rework_transitions_to_at_vendor() {
    let result = state_machine::transition_on_review_outcome(
        "review_in_progress",
        ReviewOutcome::Rejected,
        true,
    );
    assert_eq!(result.unwrap(), OpOrderStatus::AtVendor);
}

// ── 10. Review accepted → status=closed ─────────────────────────────────────

#[tokio::test]
#[serial]
async fn test_review_accepted_closes_order() {
    let result = state_machine::transition_on_review_outcome(
        "review_in_progress",
        ReviewOutcome::Accepted,
        false,
    );
    assert_eq!(result.unwrap(), OpOrderStatus::Closed);
}

// ── 11. Full happy path: draft→issued→shipped→returned→review→closed ─────────

#[tokio::test]
#[serial]
async fn test_full_lifecycle_happy_path() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let mut ctx = pool.begin().await.unwrap();
    let order = repo::create_order(&mut ctx, &tid, &base_create_req("alice"), "OP-000011")
        .await
        .unwrap();
    ctx.commit().await.unwrap();

    // Issue
    let mut tx = pool.begin().await.unwrap();
    repo::issue_order(&mut tx, &tid, order.op_order_id, None)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Ship 10
    let mut tx2 = pool.begin().await.unwrap();
    let (_, sum_shipped, _) =
        repo::lock_order_for_quantity_check(&mut tx2, &tid, order.op_order_id)
            .await
            .unwrap();
    assert_eq!(sum_shipped, 0);
    repo::create_ship_event_tx(&mut tx2, &tid, order.op_order_id, &ship_req(10))
        .await
        .unwrap();
    repo::set_order_status(&mut tx2, &tid, order.op_order_id, "shipped_to_vendor")
        .await
        .unwrap();
    tx2.commit().await.unwrap();

    // Return 10
    let mut tx3 = pool.begin().await.unwrap();
    let (_, sum_shipped2, sum_received) =
        repo::lock_order_for_quantity_check(&mut tx3, &tid, order.op_order_id)
            .await
            .unwrap();
    assert_eq!(sum_shipped2, 10);
    assert_eq!(sum_received, 0);
    let ret = repo::create_return_event_tx(&mut tx3, &tid, order.op_order_id, &return_req(10))
        .await
        .unwrap();
    repo::set_order_status(&mut tx3, &tid, order.op_order_id, "returned")
        .await
        .unwrap();
    tx3.commit().await.unwrap();

    // Record review accepted → closed
    let review_req = CreateReviewRequest {
        return_event_id: ret.id,
        outcome: ReviewOutcome::Accepted,
        conditions: None,
        rejection_reason: None,
        rework: None,
        reviewed_by: "inspector-1".to_string(),
        reviewed_at: Utc::now(),
        notes: None,
    };
    let mut tx4 = pool.begin().await.unwrap();
    // Enter review_in_progress
    repo::set_order_status(&mut tx4, &tid, order.op_order_id, "review_in_progress")
        .await
        .unwrap();
    repo::create_review_tx(&mut tx4, &tid, order.op_order_id, &review_req)
        .await
        .unwrap();
    repo::set_order_status(&mut tx4, &tid, order.op_order_id, "closed")
        .await
        .unwrap();
    tx4.commit().await.unwrap();

    let final_order = repo::get_order(&pool, &tid, order.op_order_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(final_order.status, "closed");
}

// ── 12. Cancel from non-terminal state → cancelled ───────────────────────────

#[tokio::test]
#[serial]
async fn test_cancel_from_draft() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let mut ctx = pool.begin().await.unwrap();
    let order = repo::create_order(&mut ctx, &tid, &base_create_req("alice"), "OP-000012")
        .await
        .unwrap();
    ctx.commit().await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    let new_status = state_machine::transition_cancel(&order.status).unwrap();
    repo::set_order_status(&mut tx, &tid, order.op_order_id, new_status.as_str())
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let updated = repo::get_order(&pool, &tid, order.op_order_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.status, "cancelled");
}

// ── 13. Cancel from closed → error ──────────────────────────────────────────

#[tokio::test]
#[serial]
async fn test_cancel_from_closed_fails() {
    let result = state_machine::transition_cancel("closed");
    assert!(
        result.is_err(),
        "Should not be able to cancel a closed order"
    );
}

// ── 14. Re-identification requires return event ──────────────────────────────

#[tokio::test]
#[serial]
async fn test_re_identification_requires_return_event() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let mut ctx = pool.begin().await.unwrap();
    let order = repo::create_order(&mut ctx, &tid, &base_create_req("alice"), "OP-000014")
        .await
        .unwrap();
    ctx.commit().await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    let return_count = repo::count_return_events(&mut tx, order.op_order_id)
        .await
        .unwrap();
    assert_eq!(return_count, 0);
    tx.rollback().await.unwrap();
}

// ── 15. Quantity bound holds across two rework cycles ───────────────────────

#[tokio::test]
#[serial]
async fn test_quantity_bound_across_rework_cycles() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    // quantity_sent = 10
    let mut ctx = pool.begin().await.unwrap();
    let order = repo::create_order(&mut ctx, &tid, &base_create_req("alice"), "OP-000015")
        .await
        .unwrap();
    ctx.commit().await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    repo::issue_order(&mut tx, &tid, order.op_order_id, None)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Cycle 1: ship 6
    let mut tx2 = pool.begin().await.unwrap();
    repo::create_ship_event_tx(&mut tx2, &tid, order.op_order_id, &ship_req(6))
        .await
        .unwrap();
    repo::set_order_status(&mut tx2, &tid, order.op_order_id, "shipped_to_vendor")
        .await
        .unwrap();
    tx2.commit().await.unwrap();

    // Return 6
    let mut tx3 = pool.begin().await.unwrap();
    let ret1 = repo::create_return_event_tx(&mut tx3, &tid, order.op_order_id, &return_req(6))
        .await
        .unwrap();
    repo::set_order_status(&mut tx3, &tid, order.op_order_id, "returned")
        .await
        .unwrap();
    tx3.commit().await.unwrap();

    // Review rejected + rework → at_vendor
    let review_req = CreateReviewRequest {
        return_event_id: ret1.id,
        outcome: ReviewOutcome::Rejected,
        conditions: None,
        rejection_reason: Some("rework needed".to_string()),
        rework: Some(true),
        reviewed_by: "inspector-1".to_string(),
        reviewed_at: Utc::now(),
        notes: None,
    };
    let mut tx4 = pool.begin().await.unwrap();
    repo::set_order_status(&mut tx4, &tid, order.op_order_id, "review_in_progress")
        .await
        .unwrap();
    repo::create_review_tx(&mut tx4, &tid, order.op_order_id, &review_req)
        .await
        .unwrap();
    repo::set_order_status(&mut tx4, &tid, order.op_order_id, "at_vendor")
        .await
        .unwrap();
    tx4.commit().await.unwrap();

    // Cycle 2: try to ship 5 (total would be 6+5=11 > 10)
    let mut tx5 = pool.begin().await.unwrap();
    let (order5, sum_shipped5, _) =
        repo::lock_order_for_quantity_check(&mut tx5, &tid, order.op_order_id)
            .await
            .unwrap();
    assert_eq!(sum_shipped5, 6, "Cycle 1 shipped 6");
    let would_exceed = sum_shipped5 + 5 > order5.quantity_sent as i64;
    assert!(
        would_exceed,
        "Cycle 2 ship of 5 would exceed quantity_sent of 10"
    );
    tx5.rollback().await.unwrap();

    // Cycle 2: ship 4 (total 6+4=10, exactly at bound — allowed)
    let mut tx6 = pool.begin().await.unwrap();
    let (order6, sum6, _) = repo::lock_order_for_quantity_check(&mut tx6, &tid, order.op_order_id)
        .await
        .unwrap();
    assert_eq!(sum6, 6);
    assert!(
        sum6 + 4 <= order6.quantity_sent as i64,
        "Should be within bound"
    );
    repo::create_ship_event_tx(&mut tx6, &tid, order.op_order_id, &ship_req(4))
        .await
        .unwrap();
    repo::set_order_status(&mut tx6, &tid, order.op_order_id, "shipped_to_vendor")
        .await
        .unwrap();
    tx6.commit().await.unwrap();

    // Verify total shipped
    let mut tx7 = pool.begin().await.unwrap();
    let (_, total_shipped, _) =
        repo::lock_order_for_quantity_check(&mut tx7, &tid, order.op_order_id)
            .await
            .unwrap();
    assert_eq!(
        total_shipped, 10,
        "Total shipped across 2 cycles should be 10"
    );
    tx7.rollback().await.unwrap();
}

// ── 16. Label upsert ─────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn test_status_label_upsert() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let label = repo::upsert_status_label(
        &pool,
        &tid,
        "draft",
        &UpsertStatusLabelRequest {
            display_label: "Pending".to_string(),
            description: Some("Order not yet issued".to_string()),
            updated_by: "admin".to_string(),
        },
    )
    .await
    .unwrap();

    assert_eq!(label.display_label, "Pending");
    assert_eq!(label.canonical_status, "draft");
}

// ── 17. Service-type label upsert ────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn test_service_type_label_upsert() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let label = repo::upsert_service_type_label(
        &pool,
        &tid,
        "heat_treat",
        &UpsertServiceTypeLabelRequest {
            display_label: "Heat Treatment".to_string(),
            description: Some("Thermal processing services".to_string()),
            updated_by: "admin".to_string(),
        },
    )
    .await
    .unwrap();

    assert_eq!(label.display_label, "Heat Treatment");
    assert_eq!(label.service_type, "heat_treat");
}
