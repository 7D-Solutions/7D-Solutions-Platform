//! Integration tests for lot expiry computation/assignment and alert scanning.

use chrono::{Duration, NaiveDate, Utc};
use inventory_rs::domain::{
    expiry::{
        run_expiry_alert_scan, set_lot_expiry, ExpiryError, RunExpiryAlertScanRequest,
        SetLotExpiryRequest,
    },
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    receipt_service::{process_receipt, ReceiptRequest},
    revisions::{
        activate_revision, create_revision, ActivateRevisionRequest, CreateRevisionRequest,
    },
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

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

fn make_lot_item(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: "Lot Item".to_string(),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::Lot,
        make_buy: None,
    }
}

async fn cleanup(pool: &sqlx::PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM inv_lot_expiry_alert_state WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_layers WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_lots WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_ledger WHERE tenant_id = $1")
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

#[tokio::test]
#[serial]
async fn receipt_computes_expiry_from_active_revision_policy() {
    let pool = setup_db().await;
    let tenant = format!("test-expiry-{}", Uuid::new_v4());
    let item = ItemRepo::create(&pool, &make_lot_item(&tenant, "SKU-EXP-REC"))
        .await
        .expect("create item");

    let (rev, _) = create_revision(
        &pool,
        &CreateRevisionRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            name: "Rev 1".to_string(),
            description: None,
            uom: "ea".to_string(),
            inventory_account_ref: "1200".to_string(),
            cogs_account_ref: "5000".to_string(),
            variance_account_ref: "5010".to_string(),
            traceability_level: "lot".to_string(),
            inspection_required: false,
            shelf_life_days: Some(30),
            shelf_life_enforced: true,
            change_reason: "Policy".to_string(),
            idempotency_key: format!("idem-rev-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
            actor_id: None,
        },
    )
    .await
    .expect("create revision");
    activate_revision(
        &pool,
        item.id,
        rev.id,
        &ActivateRevisionRequest {
            tenant_id: tenant.clone(),
            effective_from: Utc::now() - Duration::hours(1),
            effective_to: None,
            idempotency_key: format!("idem-act-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
            actor_id: None,
        },
    )
    .await
    .expect("activate revision");

    let (receipt, _) = process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            warehouse_id: Uuid::new_v4(),
            location_id: None,
            quantity: 10,
            unit_cost_minor: 1000,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: format!("idem-rc-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
            lot_code: Some("LOT-EXP-001".to_string()),
            serial_codes: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("receipt");

    let lot_id = receipt.lot_id.expect("lot id");
    let expires_on: Option<NaiveDate> = sqlx::query_scalar(
        "SELECT expires_on FROM inventory_lots WHERE id = $1 AND tenant_id = $2",
    )
    .bind(lot_id)
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("query lot expiry");
    assert_eq!(
        expires_on,
        Some(receipt.received_at.date_naive() + Duration::days(30)),
        "expiry should be computed from active revision shelf_life_days"
    );

    let expiry_set_events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inv_outbox WHERE tenant_id = $1 AND event_type = 'inventory.expiry_set.v1'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("outbox count");
    assert_eq!(expiry_set_events, 1, "receipt should emit expiry_set event");

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn manual_expiry_set_is_idempotent() {
    let pool = setup_db().await;
    let tenant = format!("test-expiry-manual-{}", Uuid::new_v4());
    let item = ItemRepo::create(&pool, &make_lot_item(&tenant, "SKU-EXP-MAN"))
        .await
        .expect("create item");

    let (receipt, _) = process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            warehouse_id: Uuid::new_v4(),
            location_id: None,
            quantity: 5,
            unit_cost_minor: 1200,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: format!("idem-rc-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
            lot_code: Some("LOT-MAN-001".to_string()),
            serial_codes: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("receipt");

    let lot_id = receipt.lot_id.expect("lot id");
    let idem = format!("idem-exp-set-{}", Uuid::new_v4());
    let req = SetLotExpiryRequest {
        tenant_id: tenant.clone(),
        lot_id,
        expires_on: Some(NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()),
        compute_from_policy: false,
        reference_at: None,
        idempotency_key: idem.clone(),
        correlation_id: None,
        causation_id: None,
    };

    let (first, first_replay) = set_lot_expiry(&pool, &req).await.expect("set expiry");
    assert!(!first_replay);
    let (second, second_replay) = set_lot_expiry(&pool, &req).await.expect("replay");
    assert!(second_replay);
    assert_eq!(first.lot_id, second.lot_id);
    assert_eq!(first.expires_on, second.expires_on);

    let mut conflict_req = req;
    conflict_req.expires_on = Some(NaiveDate::from_ymd_opt(2026, 6, 2).unwrap());
    let err = set_lot_expiry(&pool, &conflict_req)
        .await
        .expect_err("conflicting request should fail");
    assert!(
        matches!(err, ExpiryError::ConflictingIdempotencyKey),
        "expected idempotency conflict, got: {:?}",
        err
    );

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn alert_scan_dedupes_and_is_tenant_scoped() {
    let pool = setup_db().await;
    let tenant_a = format!("test-expiry-a-{}", Uuid::new_v4());
    let tenant_b = format!("test-expiry-b-{}", Uuid::new_v4());

    let item_a = ItemRepo::create(&pool, &make_lot_item(&tenant_a, "SKU-EXP-A"))
        .await
        .expect("create item A");
    let item_b = ItemRepo::create(&pool, &make_lot_item(&tenant_b, "SKU-EXP-B"))
        .await
        .expect("create item B");

    let (receipt_a, _) = process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: tenant_a.clone(),
            item_id: item_a.id,
            warehouse_id: Uuid::new_v4(),
            location_id: None,
            quantity: 3,
            unit_cost_minor: 800,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: format!("idem-rc-a-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
            lot_code: Some("LOT-A".to_string()),
            serial_codes: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("receipt A");

    let (receipt_b, _) = process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: tenant_b.clone(),
            item_id: item_b.id,
            warehouse_id: Uuid::new_v4(),
            location_id: None,
            quantity: 3,
            unit_cost_minor: 800,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: format!("idem-rc-b-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
            lot_code: Some("LOT-B".to_string()),
            serial_codes: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("receipt B");

    let as_of = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
    set_lot_expiry(
        &pool,
        &SetLotExpiryRequest {
            tenant_id: tenant_a.clone(),
            lot_id: receipt_a.lot_id.expect("lot a"),
            expires_on: Some(as_of + Duration::days(1)),
            compute_from_policy: false,
            reference_at: None,
            idempotency_key: format!("idem-set-a-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect("set lot A expiry");
    set_lot_expiry(
        &pool,
        &SetLotExpiryRequest {
            tenant_id: tenant_b.clone(),
            lot_id: receipt_b.lot_id.expect("lot b"),
            expires_on: Some(as_of + Duration::days(1)),
            compute_from_policy: false,
            reference_at: None,
            idempotency_key: format!("idem-set-b-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect("set lot B expiry");

    let (first, first_replay) = run_expiry_alert_scan(
        &pool,
        &RunExpiryAlertScanRequest {
            tenant_id: tenant_a.clone(),
            as_of_date: Some(as_of),
            expiring_within_days: 2,
            idempotency_key: format!("idem-scan-1-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect("first scan");
    assert!(!first_replay);
    assert_eq!(first.expiring_soon_emitted, 1);
    assert_eq!(first.expired_emitted, 0);

    let (second, second_replay) = run_expiry_alert_scan(
        &pool,
        &RunExpiryAlertScanRequest {
            tenant_id: tenant_a.clone(),
            as_of_date: Some(as_of),
            expiring_within_days: 2,
            idempotency_key: format!("idem-scan-2-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect("second scan");
    assert!(!second_replay);
    assert_eq!(
        second.expiring_soon_emitted, 0,
        "dedupe should suppress duplicate alert"
    );
    assert_eq!(second.expired_emitted, 0);

    let a_alerts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inv_outbox WHERE tenant_id = $1 AND event_type = 'inventory.expiry_alert.v1'",
    )
    .bind(&tenant_a)
    .fetch_one(&pool)
    .await
    .expect("count A alerts");
    let b_alerts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inv_outbox WHERE tenant_id = $1 AND event_type = 'inventory.expiry_alert.v1'",
    )
    .bind(&tenant_b)
    .fetch_one(&pool)
    .await
    .expect("count B alerts");
    assert_eq!(a_alerts, 1, "tenant A should emit exactly one alert");
    assert_eq!(
        b_alerts, 0,
        "tenant B should not be scanned by tenant A job"
    );

    cleanup(&pool, &tenant_a).await;
    cleanup(&pool, &tenant_b).await;
}
