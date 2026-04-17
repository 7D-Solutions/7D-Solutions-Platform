use bom_rs::domain::bom_service;
use bom_rs::domain::inventory_client::InventoryClient;
use bom_rs::domain::kit_readiness_engine;
use bom_rs::domain::models::*;
use chrono::{Duration, Utc};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://bom_user:bom_pass@localhost:5450/bom_db".to_string());

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to BOM test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run BOM migrations");

    pool
}

fn unique_tenant() -> String {
    format!("test-tenant-{}", Uuid::new_v4())
}

/// Create a single-level BOM with the given component → bom.id
async fn create_single_level_bom(
    pool: &sqlx::PgPool,
    tenant: &str,
    corr: &str,
    components: &[(Uuid, f64)], // (item_id, quantity)
) -> BomHeader {
    let part_id = Uuid::new_v4();
    let bom = bom_service::create_bom(
        pool,
        tenant,
        &CreateBomRequest { part_id, description: None },
        corr,
        None,
    )
    .await
    .unwrap();

    let rev = bom_service::create_revision(
        pool,
        tenant,
        bom.id,
        &CreateRevisionRequest { revision_label: "Rev-1".to_string() },
        corr,
        None,
    )
    .await
    .unwrap();

    for (comp_id, qty) in components {
        bom_service::add_line(
            pool,
            tenant,
            rev.id,
            &AddLineRequest {
                component_item_id: *comp_id,
                quantity: *qty,
                uom: Some("EA".to_string()),
                scrap_factor: Some(0.0),
                find_number: None,
            },
            corr,
            None,
        )
        .await
        .unwrap();
    }

    bom_service::set_effectivity(
        pool,
        tenant,
        rev.id,
        &SetEffectivityRequest {
            effective_from: Utc::now() - Duration::hours(1),
            effective_to: None,
        },
        corr,
        None,
    )
    .await
    .unwrap();

    bom
}

/// Seed on-hand availability for an item into the test scaffold table.
async fn seed_on_hand(
    pool: &sqlx::PgPool,
    tenant: &str,
    item_id: Uuid,
    on_hand: i64,
    expired: i64,
    quarantine: i64,
) {
    sqlx::query(
        r#"
        INSERT INTO item_on_hand (tenant_id, item_id, on_hand_qty, expired_qty, quarantine_qty)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (tenant_id, item_id)
        DO UPDATE SET on_hand_qty = EXCLUDED.on_hand_qty,
                      expired_qty = EXCLUDED.expired_qty,
                      quarantine_qty = EXCLUDED.quarantine_qty
        "#,
    )
    .bind(tenant)
    .bind(item_id)
    .bind(on_hand)
    .bind(expired)
    .bind(quarantine)
    .execute(pool)
    .await
    .expect("seed_on_hand");
}

// ============================================================================
// Test 1: All 3 components fully available → overall_status = "ready"
// ============================================================================

#[tokio::test]
#[serial]
async fn kit_readiness_all_components_available_returns_ready() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let comp_a = Uuid::new_v4();
    let comp_b = Uuid::new_v4();
    let comp_c = Uuid::new_v4();

    let bom = create_single_level_bom(
        &pool,
        &tenant,
        &corr,
        &[(comp_a, 2.0), (comp_b, 5.0), (comp_c, 1.0)],
    )
    .await;

    // Each component has more than enough: need 2, 5, 1 units (required_quantity=1)
    seed_on_hand(&pool, &tenant, comp_a, 100, 0, 0).await;
    seed_on_hand(&pool, &tenant, comp_b, 100, 0, 0).await;
    seed_on_hand(&pool, &tenant, comp_c, 100, 0, 0).await;

    let inventory = InventoryClient::direct(pool.clone());
    let result = kit_readiness_engine::check(
        &pool,
        &tenant,
        &KitReadinessCheckRequest {
            bom_id: bom.id,
            required_quantity: 1.0,
            check_date: Utc::now(),
            created_by: "test".to_string(),
        },
        &inventory,
        None,
        &corr,
        None,
    )
    .await
    .expect("kit readiness check");

    assert_eq!(result.snapshot.overall_status, "ready");
    assert_eq!(result.lines.len(), 3);
    assert!(result.lines.iter().all(|l| l.status == "ready"));
}

// ============================================================================
// Test 2: One component short → overall_status = "partial"
// ============================================================================

#[tokio::test]
#[serial]
async fn kit_readiness_one_short_returns_partial() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let comp_a = Uuid::new_v4();
    let comp_b = Uuid::new_v4();

    let bom = create_single_level_bom(
        &pool,
        &tenant,
        &corr,
        &[(comp_a, 10.0), (comp_b, 5.0)],
    )
    .await;

    // comp_a: available=20 (enough for 10 required), comp_b: available=3 (short of 5)
    seed_on_hand(&pool, &tenant, comp_a, 20, 0, 0).await;
    seed_on_hand(&pool, &tenant, comp_b, 3, 0, 0).await;

    let inventory = InventoryClient::direct(pool.clone());
    let result = kit_readiness_engine::check(
        &pool,
        &tenant,
        &KitReadinessCheckRequest {
            bom_id: bom.id,
            required_quantity: 1.0,
            check_date: Utc::now(),
            created_by: "test".to_string(),
        },
        &inventory,
        None,
        &corr,
        None,
    )
    .await
    .expect("kit readiness check");

    assert_eq!(result.snapshot.overall_status, "partial");

    let ready: Vec<_> = result.lines.iter().filter(|l| l.status == "ready").collect();
    let short: Vec<_> = result.lines.iter().filter(|l| l.status == "short").collect();
    assert_eq!(ready.len(), 1, "one ready");
    assert_eq!(short.len(), 1, "one short");
    assert_eq!(short[0].component_item_id, comp_b);
}

// ============================================================================
// Test 3: No components available → overall_status = "not_ready"
// ============================================================================

#[tokio::test]
#[serial]
async fn kit_readiness_none_available_returns_not_ready() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let comp_a = Uuid::new_v4();

    let bom = create_single_level_bom(&pool, &tenant, &corr, &[(comp_a, 10.0)]).await;

    // Zero on-hand
    seed_on_hand(&pool, &tenant, comp_a, 0, 0, 0).await;

    let inventory = InventoryClient::direct(pool.clone());
    let result = kit_readiness_engine::check(
        &pool,
        &tenant,
        &KitReadinessCheckRequest {
            bom_id: bom.id,
            required_quantity: 1.0,
            check_date: Utc::now(),
            created_by: "test".to_string(),
        },
        &inventory,
        None,
        &corr,
        None,
    )
    .await
    .expect("kit readiness check");

    assert_eq!(result.snapshot.overall_status, "not_ready");
    assert!(result.lines.iter().all(|l| l.status == "short"));
}

// ============================================================================
// Test 4: Snapshot is persisted and retrievable via get_snapshot
// ============================================================================

#[tokio::test]
#[serial]
async fn kit_readiness_snapshot_is_persisted_and_retrievable() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let comp_a = Uuid::new_v4();

    let bom = create_single_level_bom(&pool, &tenant, &corr, &[(comp_a, 2.0)]).await;
    seed_on_hand(&pool, &tenant, comp_a, 50, 0, 0).await;

    let inventory = InventoryClient::direct(pool.clone());
    let created = kit_readiness_engine::check(
        &pool,
        &tenant,
        &KitReadinessCheckRequest {
            bom_id: bom.id,
            required_quantity: 1.0,
            check_date: Utc::now(),
            created_by: "auditor".to_string(),
        },
        &inventory,
        None,
        &corr,
        None,
    )
    .await
    .expect("kit readiness check");

    let fetched = kit_readiness_engine::get_snapshot(&pool, &tenant, created.snapshot.id)
        .await
        .expect("get_snapshot");

    assert_eq!(fetched.snapshot.id, created.snapshot.id);
    assert_eq!(fetched.snapshot.overall_status, "ready");
    assert_eq!(fetched.snapshot.created_by, "auditor");
    assert_eq!(fetched.lines.len(), 1);
    assert_eq!(fetched.lines[0].component_item_id, comp_a);
    assert_eq!(fetched.lines[0].status, "ready");
}

// ============================================================================
// Test 5: Outbox event emitted with correct event_type and overall_status
// ============================================================================

#[tokio::test]
#[serial]
async fn kit_readiness_event_emitted_to_outbox() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let comp_a = Uuid::new_v4();

    let bom = create_single_level_bom(&pool, &tenant, &corr, &[(comp_a, 1.0)]).await;
    seed_on_hand(&pool, &tenant, comp_a, 100, 0, 0).await;

    let inventory = InventoryClient::direct(pool.clone());
    let result = kit_readiness_engine::check(
        &pool,
        &tenant,
        &KitReadinessCheckRequest {
            bom_id: bom.id,
            required_quantity: 1.0,
            check_date: Utc::now(),
            created_by: "test".to_string(),
        },
        &inventory,
        None,
        &corr,
        None,
    )
    .await
    .expect("kit readiness check");

    let row: (String, serde_json::Value) = sqlx::query_as(
        "SELECT event_type, payload FROM bom_outbox WHERE aggregate_id = $1 AND tenant_id = $2",
    )
    .bind(result.snapshot.id.to_string())
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("outbox row");

    assert_eq!(row.0, "bom.kit_readiness_checked");
    let payload = &row.1;
    assert_eq!(
        payload["payload"]["overall_status"].as_str().unwrap(),
        "ready"
    );
    assert_eq!(
        payload["payload"]["snapshot_id"].as_str().unwrap(),
        result.snapshot.id.to_string()
    );
}

// ============================================================================
// Test 6: Read-only — item_on_hand row count unchanged after check
// ============================================================================

#[tokio::test]
#[serial]
async fn kit_readiness_does_not_modify_on_hand_records() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let comp_a = Uuid::new_v4();

    let bom = create_single_level_bom(&pool, &tenant, &corr, &[(comp_a, 3.0)]).await;
    seed_on_hand(&pool, &tenant, comp_a, 10, 0, 0).await;

    // Capture on_hand_qty before
    let before: i64 = sqlx::query_scalar(
        "SELECT on_hand_qty FROM item_on_hand WHERE item_id = $1 AND tenant_id = $2",
    )
    .bind(comp_a)
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("on_hand before");

    let inventory = InventoryClient::direct(pool.clone());
    kit_readiness_engine::check(
        &pool,
        &tenant,
        &KitReadinessCheckRequest {
            bom_id: bom.id,
            required_quantity: 1.0,
            check_date: Utc::now(),
            created_by: "test".to_string(),
        },
        &inventory,
        None,
        &corr,
        None,
    )
    .await
    .expect("kit readiness check");

    let after: i64 = sqlx::query_scalar(
        "SELECT on_hand_qty FROM item_on_hand WHERE item_id = $1 AND tenant_id = $2",
    )
    .bind(comp_a)
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("on_hand after");

    assert_eq!(before, after, "on_hand_qty must not change after a readiness check");
}
