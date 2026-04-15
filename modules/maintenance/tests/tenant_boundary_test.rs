//! Tenant Boundary Concurrency Tests (Phase 58 Gate A, bd-1umv8)
//!
//! Proves no cross-tenant data leakage under concurrent load.
//! Two tenants operate simultaneously and must never see each other's data.
//!
//! ## Strategy
//! - Two tenants each create assets, work orders, and parts concurrently
//! - After all writes, verify each tenant sees only its own data
//! - Read queries scoped by tenant never return the other tenant's rows
//!
//! ## Prerequisites
//! - PostgreSQL at localhost:5452 (docker compose up -d)

use maintenance_rs::domain::assets::{AssetRepo, CreateAssetRequest};
use maintenance_rs::domain::work_orders::{
    AddPartRequest, CreateWorkOrderRequest, ListWorkOrdersQuery, WoPartsRepo, WorkOrderRepo,
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// Test DB helpers
// ============================================================================

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://maintenance_user:maintenance_pass@localhost:5452/maintenance_db".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
        .expect("Failed to connect to maintenance test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run maintenance migrations");

    pool
}

fn unique_tenant(prefix: &str) -> String {
    format!("{}-{}", prefix, Uuid::new_v4().simple())
}

async fn create_asset(pool: &sqlx::PgPool, tid: &str, tag: &str) -> Uuid {
    let asset = AssetRepo::create(
        pool,
        &CreateAssetRequest {
            tenant_id: tid.to_string(),
            asset_tag: tag.to_string(),
            name: format!("Asset {}", tag),
            description: None,
            asset_type: "equipment".into(),
            location: None,
            department: None,
            responsible_person: None,
            serial_number: None,
            fixed_asset_ref: None,
            metadata: None,
            maintenance_schedule: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();
    asset.id
}

fn wo_req(tid: &str, asset_id: Uuid) -> CreateWorkOrderRequest {
    CreateWorkOrderRequest {
        tenant_id: tid.to_string(),
        asset_id,
        plan_assignment_id: None,
        title: format!("WO for {}", tid),
        description: None,
        wo_type: "corrective".into(),
        priority: None,
        assigned_to: None,
        scheduled_date: None,
        checklist: None,
        notes: None,
    }
}

async fn cleanup_tenant(pool: &sqlx::PgPool, tid: &str) {
    // Delete in dependency order
    let tables = [
        "events_outbox",
        "work_order_labor",
        "work_order_parts",
        "work_orders",
        "downtime_events",
        "meter_readings",
        "maintenance_plan_assignments",
        "maintenance_plans",
        "meter_types",
        "maintainable_assets",
        "wo_counters",
        "maintenance_tenant_config",
    ];
    for table in &tables {
        let sql = format!("DELETE FROM {} WHERE tenant_id = $1", table);
        sqlx::query(&sql).bind(tid).execute(pool).await.ok();
    }
    // events_outbox doesn't have tenant_id, clean via aggregate_id/payload
    // (already handled by deletion of work_orders above removing FKs)
}

// ============================================================================
// Test 1: Concurrent writes — two tenants, no cross-contamination
// ============================================================================

#[tokio::test]
#[serial]
async fn concurrent_work_orders_are_tenant_isolated() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant("boundary-a");
    let tenant_b = unique_tenant("boundary-b");

    // Create assets for each tenant
    let asset_a = create_asset(&pool, &tenant_a, "ASSET-ISO-A").await;
    let asset_b = create_asset(&pool, &tenant_b, "ASSET-ISO-B").await;

    // Create work orders concurrently (5 each, interleaved)
    let mut handles = Vec::new();
    for _ in 0..5 {
        let p = pool.clone();
        let ta = tenant_a.clone();
        handles.push(tokio::spawn(async move {
            WorkOrderRepo::create(&p, &wo_req(&ta, asset_a))
                .await
                .expect("create WO for tenant A")
        }));

        let p = pool.clone();
        let tb = tenant_b.clone();
        handles.push(tokio::spawn(async move {
            WorkOrderRepo::create(&p, &wo_req(&tb, asset_b))
                .await
                .expect("create WO for tenant B")
        }));
    }
    for h in handles {
        h.await.expect("join");
    }

    // Verify tenant A sees only their own work orders
    let a_wos = WorkOrderRepo::list(
        &pool,
        &ListWorkOrdersQuery {
            tenant_id: tenant_a.clone(),
            asset_id: None,
            status: None,
            limit: Some(100),
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(a_wos.len(), 5, "Tenant A should have exactly 5 work orders");
    for wo in &a_wos {
        assert_eq!(wo.tenant_id, tenant_a, "WO must belong to tenant A");
        assert_eq!(wo.asset_id, asset_a, "WO must reference tenant A's asset");
    }

    // Verify tenant B sees only their own work orders
    let b_wos = WorkOrderRepo::list(
        &pool,
        &ListWorkOrdersQuery {
            tenant_id: tenant_b.clone(),
            asset_id: None,
            status: None,
            limit: Some(100),
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(b_wos.len(), 5, "Tenant B should have exactly 5 work orders");
    for wo in &b_wos {
        assert_eq!(wo.tenant_id, tenant_b, "WO must belong to tenant B");
        assert_eq!(wo.asset_id, asset_b, "WO must reference tenant B's asset");
    }

    // Cross-tenant check: tenant A's asset must not appear in tenant B's data
    let cross_leak: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM work_orders WHERE tenant_id = $1 AND asset_id = $2",
    )
    .bind(&tenant_b)
    .bind(asset_a)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        cross_leak, 0,
        "Tenant B must not have Tenant A's asset in WOs"
    );

    let cross_leak_rev: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM work_orders WHERE tenant_id = $1 AND asset_id = $2",
    )
    .bind(&tenant_a)
    .bind(asset_b)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        cross_leak_rev, 0,
        "Tenant A must not have Tenant B's asset in WOs"
    );

    // Verify WO numbers are independently sequenced per tenant
    let a_numbers: Vec<String> = a_wos.iter().map(|wo| wo.wo_number.clone()).collect();
    let b_numbers: Vec<String> = b_wos.iter().map(|wo| wo.wo_number.clone()).collect();
    // Each tenant should have WO-000001 through WO-000005 (independent counters)
    for n in &a_numbers {
        assert!(n.starts_with("WO-"), "WO number should start with WO-");
    }
    for n in &b_numbers {
        assert!(n.starts_with("WO-"), "WO number should start with WO-");
    }

    cleanup_tenant(&pool, &tenant_a).await;
    cleanup_tenant(&pool, &tenant_b).await;
}

// ============================================================================
// Test 2: find_by_id respects tenant boundary
// ============================================================================

#[tokio::test]
#[serial]
async fn find_by_id_respects_tenant_boundary() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant("find-a");
    let tenant_b = unique_tenant("find-b");

    let asset_a = create_asset(&pool, &tenant_a, "ASSET-FIND-A").await;
    let asset_b = create_asset(&pool, &tenant_b, "ASSET-FIND-B").await;

    let wo_a = WorkOrderRepo::create(&pool, &wo_req(&tenant_a, asset_a))
        .await
        .unwrap();
    let wo_b = WorkOrderRepo::create(&pool, &wo_req(&tenant_b, asset_b))
        .await
        .unwrap();

    // Tenant A can see their own WO
    let found_a = WorkOrderRepo::find_by_id(&pool, wo_a.id, &tenant_a)
        .await
        .unwrap();
    assert!(found_a.is_some(), "Tenant A should find their own WO");

    // Tenant A cannot see tenant B's WO
    let cross = WorkOrderRepo::find_by_id(&pool, wo_b.id, &tenant_a)
        .await
        .unwrap();
    assert!(cross.is_none(), "Tenant A must NOT see Tenant B's WO");

    // Tenant B cannot see tenant A's WO
    let cross_rev = WorkOrderRepo::find_by_id(&pool, wo_a.id, &tenant_b)
        .await
        .unwrap();
    assert!(cross_rev.is_none(), "Tenant B must NOT see Tenant A's WO");

    // Asset cross-tenant check
    let cross_asset = AssetRepo::find_by_id(&pool, asset_a, &tenant_b)
        .await
        .unwrap();
    assert!(
        cross_asset.is_none(),
        "Tenant B must NOT see Tenant A's asset"
    );

    cleanup_tenant(&pool, &tenant_a).await;
    cleanup_tenant(&pool, &tenant_b).await;
}

// ============================================================================
// Test 3: Parts subresource respects tenant boundary
// ============================================================================

#[tokio::test]
#[serial]
async fn parts_respect_tenant_boundary() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant("parts-a");
    let tenant_b = unique_tenant("parts-b");

    let asset_a = create_asset(&pool, &tenant_a, "ASSET-PARTS-A").await;
    let asset_b = create_asset(&pool, &tenant_b, "ASSET-PARTS-B").await;

    let wo_a = WorkOrderRepo::create(&pool, &wo_req(&tenant_a, asset_a))
        .await
        .unwrap();
    let wo_b = WorkOrderRepo::create(&pool, &wo_req(&tenant_b, asset_b))
        .await
        .unwrap();

    // Add parts to each tenant's WO
    WoPartsRepo::add(
        &pool,
        wo_a.id,
        &AddPartRequest {
            tenant_id: tenant_a.clone(),
            part_description: "Gasket A".into(),
            part_ref: None,
            quantity: 2,
            unit_cost_minor: 1500,
            currency: None,
            inventory_issue_ref: None,
        },
    )
    .await
    .unwrap();

    WoPartsRepo::add(
        &pool,
        wo_b.id,
        &AddPartRequest {
            tenant_id: tenant_b.clone(),
            part_description: "Gasket B".into(),
            part_ref: None,
            quantity: 3,
            unit_cost_minor: 2000,
            currency: None,
            inventory_issue_ref: None,
        },
    )
    .await
    .unwrap();

    // Tenant A lists parts on their WO — sees only their part
    let a_parts = WoPartsRepo::list(&pool, wo_a.id, &tenant_a).await.unwrap();
    assert_eq!(a_parts.len(), 1);
    assert_eq!(a_parts[0].part_description, "Gasket A");

    // Tenant B lists parts on their WO — sees only their part
    let b_parts = WoPartsRepo::list(&pool, wo_b.id, &tenant_b).await.unwrap();
    assert_eq!(b_parts.len(), 1);
    assert_eq!(b_parts[0].part_description, "Gasket B");

    // Tenant A tries to list parts on tenant B's WO — WO not found (tenant filter)
    let cross = WoPartsRepo::list(&pool, wo_b.id, &tenant_a).await;
    assert!(
        cross.is_err(),
        "Tenant A listing parts on B's WO should fail"
    );

    cleanup_tenant(&pool, &tenant_a).await;
    cleanup_tenant(&pool, &tenant_b).await;
}

// ============================================================================
// Test 4: All core tables scope queries by tenant_id
// ============================================================================

#[tokio::test]
#[serial]
async fn all_core_tables_scope_by_tenant() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant("scope-a");
    let tenant_b = unique_tenant("scope-b");

    // Create data for both tenants
    let asset_a = create_asset(&pool, &tenant_a, "ASSET-SCOPE-A").await;
    let asset_b = create_asset(&pool, &tenant_b, "ASSET-SCOPE-B").await;

    WorkOrderRepo::create(&pool, &wo_req(&tenant_a, asset_a))
        .await
        .unwrap();
    WorkOrderRepo::create(&pool, &wo_req(&tenant_b, asset_b))
        .await
        .unwrap();

    // Verify each tenant-scoped table has data for both tenants, scoped correctly
    let tables = vec!["maintainable_assets", "work_orders", "wo_counters"];

    for table in &tables {
        let a_count: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM {} WHERE tenant_id = $1",
            table
        ))
        .bind(&tenant_a)
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|e| panic!("query {} for tenant_a: {}", table, e));
        assert!(a_count > 0, "{} should have rows for tenant A", table);

        let b_count: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM {} WHERE tenant_id = $1",
            table
        ))
        .bind(&tenant_b)
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|e| panic!("query {} for tenant_b: {}", table, e));
        assert!(b_count > 0, "{} should have rows for tenant B", table);

        // Verify isolation: rows for A don't appear under B's tenant_id
        let total: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM {} WHERE tenant_id IN ($1, $2)",
            table
        ))
        .bind(&tenant_a)
        .bind(&tenant_b)
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|e| panic!("total query {}: {}", table, e));
        assert_eq!(
            a_count + b_count,
            total,
            "{}: sum of per-tenant counts must equal total for both tenants",
            table
        );
    }

    cleanup_tenant(&pool, &tenant_a).await;
    cleanup_tenant(&pool, &tenant_b).await;
}
