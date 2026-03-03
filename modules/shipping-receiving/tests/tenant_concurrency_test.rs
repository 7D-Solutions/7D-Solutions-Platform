//! Concurrent Tenant Isolation Tests (Phase 58 Gate A, bd-227n8)
//!
//! Proves that tenant isolation holds under concurrent operations:
//! - Multiple tenants inserting shipments simultaneously
//! - Concurrent reads never leak cross-tenant data
//! - Concurrent transitions on different tenants' shipments are independent
//! - Concurrent line operations stay within tenant boundaries

use serial_test::serial;
use shipping_receiving_rs::{
    db::repository::ShipmentRepository, domain::shipments::ShipmentService, InventoryIntegration,
};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
    PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
        .expect("Failed to connect to shipping-receiving test DB")
}

async fn insert_shipment(pool: &sqlx::PgPool, tenant_id: Uuid, direction: &str) -> Uuid {
    let row: (Uuid,) = sqlx::query_as(
        "INSERT INTO shipments (tenant_id, direction, status) VALUES ($1, $2, 'draft') RETURNING id",
    )
    .bind(tenant_id)
    .bind(direction)
    .fetch_one(pool)
    .await
    .expect("insert shipment");
    row.0
}

async fn insert_line(pool: &sqlx::PgPool, tenant_id: Uuid, shipment_id: Uuid) -> Uuid {
    let row: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO shipment_lines (tenant_id, shipment_id, sku, qty_expected)
        VALUES ($1, $2, 'CONC-SKU', 10)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(shipment_id)
    .fetch_one(pool)
    .await
    .expect("insert line");
    row.0
}

// ============================================================================
// Test 1: Concurrent inserts from multiple tenants stay isolated
// ============================================================================

#[tokio::test]
#[serial]
async fn concurrent_inserts_are_tenant_isolated() {
    let pool = setup_db().await;

    let tenants: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();

    // Spawn 5 concurrent inserts, one per tenant
    let mut handles = Vec::new();
    for tenant_id in &tenants {
        let pool = pool.clone();
        let tid = *tenant_id;
        handles.push(tokio::spawn(async move {
            let s1 = insert_shipment(&pool, tid, "inbound").await;
            let s2 = insert_shipment(&pool, tid, "outbound").await;
            (tid, s1, s2)
        }));
    }

    let results: Vec<(Uuid, Uuid, Uuid)> = futures::future::try_join_all(handles)
        .await
        .expect("all tasks complete");

    // Each tenant should see exactly 2 shipments
    for (tenant_id, s1, s2) in &results {
        let list = ShipmentRepository::list_shipments(&pool, *tenant_id, None, None, 100, 0)
            .await
            .expect("list");
        assert!(
            list.len() >= 2,
            "tenant {} must see at least 2 shipments, got {}",
            tenant_id,
            list.len()
        );
        let ids: Vec<Uuid> = list.iter().map(|s| s.id).collect();
        assert!(ids.contains(s1), "tenant must see own shipment s1");
        assert!(ids.contains(s2), "tenant must see own shipment s2");

        // Must not see other tenants' shipments
        for (other_tid, other_s1, other_s2) in &results {
            if other_tid == tenant_id {
                continue;
            }
            assert!(
                !ids.contains(other_s1),
                "tenant {} must not see tenant {}'s shipment",
                tenant_id,
                other_tid
            );
            assert!(
                !ids.contains(other_s2),
                "tenant {} must not see tenant {}'s shipment",
                tenant_id,
                other_tid
            );
        }
    }
}

// ============================================================================
// Test 2: Concurrent transitions on different tenants' shipments
// ============================================================================

#[tokio::test]
#[serial]
async fn concurrent_transitions_are_independent() {
    let pool = setup_db().await;

    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();

    let ship_a = insert_shipment(&pool, tenant_a, "inbound").await;
    let ship_b = insert_shipment(&pool, tenant_b, "inbound").await;

    let req = shipping_receiving_rs::domain::shipments::TransitionRequest {
        status: "confirmed".to_string(),
        arrived_at: None,
        shipped_at: None,
        delivered_at: None,
        closed_at: None,
    };

    // Transition both shipments concurrently
    let pool_a = pool.clone();
    let pool_b = pool.clone();
    let inv_a = InventoryIntegration::deterministic();
    let inv_b = InventoryIntegration::deterministic();

    let (result_a, result_b) = tokio::join!(
        ShipmentService::transition(&pool_a, ship_a, tenant_a, &req, &inv_a),
        ShipmentService::transition(&pool_b, ship_b, tenant_b, &req, &inv_b),
    );

    let sa = result_a.expect("tenant A transition must succeed");
    let sb = result_b.expect("tenant B transition must succeed");

    assert_eq!(sa.status, "confirmed");
    assert_eq!(sb.status, "confirmed");

    // Cross-tenant: tenant B still can't see tenant A's shipment
    let cross = ShipmentService::find_by_id(&pool, ship_a, tenant_b)
        .await
        .expect("query");
    assert!(
        cross.is_none(),
        "cross-tenant visibility must still be blocked after concurrent transitions"
    );
}

// ============================================================================
// Test 3: Concurrent line operations stay within tenant boundaries
// ============================================================================

#[tokio::test]
#[serial]
async fn concurrent_line_inserts_are_tenant_isolated() {
    let pool = setup_db().await;

    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();

    let ship_a = insert_shipment(&pool, tenant_a, "inbound").await;
    let ship_b = insert_shipment(&pool, tenant_b, "inbound").await;

    // Insert lines concurrently
    let pool_a = pool.clone();
    let pool_b = pool.clone();

    let (lines_a, lines_b) = tokio::join!(
        async {
            let mut ids = Vec::new();
            for _ in 0..3 {
                ids.push(insert_line(&pool_a, tenant_a, ship_a).await);
            }
            ids
        },
        async {
            let mut ids = Vec::new();
            for _ in 0..2 {
                ids.push(insert_line(&pool_b, tenant_b, ship_b).await);
            }
            ids
        },
    );

    // Tenant A sees only their 3 lines
    let a_lines = ShipmentRepository::get_lines_for_shipment(&pool, ship_a, tenant_a)
        .await
        .expect("get lines A");
    assert_eq!(a_lines.len(), 3, "tenant A must see 3 lines");
    for line in &a_lines {
        assert!(
            lines_a.contains(&line.id),
            "tenant A line must be one of the inserted lines"
        );
    }

    // Tenant B sees only their 2 lines
    let b_lines = ShipmentRepository::get_lines_for_shipment(&pool, ship_b, tenant_b)
        .await
        .expect("get lines B");
    assert_eq!(b_lines.len(), 2, "tenant B must see 2 lines");

    // Cross-tenant: tenant B can't see tenant A's lines
    let cross_lines = ShipmentRepository::get_lines_for_shipment(&pool, ship_a, tenant_b)
        .await
        .expect("cross-tenant lines");
    assert!(
        cross_lines.is_empty(),
        "tenant B must not see tenant A's lines"
    );
}

// ============================================================================
// Test 4: Concurrent reads under write pressure don't leak data
// ============================================================================

#[tokio::test]
#[serial]
async fn concurrent_reads_under_write_pressure() {
    let pool = setup_db().await;

    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();

    // Pre-populate: tenant A has 3 shipments, tenant B has 2
    for _ in 0..3 {
        insert_shipment(&pool, tenant_a, "inbound").await;
    }
    for _ in 0..2 {
        insert_shipment(&pool, tenant_b, "outbound").await;
    }

    // Concurrent: tenant A writes while tenant B reads, and vice versa
    let pool1 = pool.clone();
    let pool2 = pool.clone();
    let pool3 = pool.clone();
    let pool4 = pool.clone();

    let (_write_a, read_b, _write_b, read_a) = tokio::join!(
        // Tenant A inserts more
        async move {
            for _ in 0..3 {
                insert_shipment(&pool1, tenant_a, "outbound").await;
            }
        },
        // Tenant B reads simultaneously
        async move {
            ShipmentRepository::list_shipments(&pool2, tenant_b, None, None, 200, 0)
                .await
                .expect("tenant B read")
        },
        // Tenant B inserts
        async move {
            for _ in 0..2 {
                insert_shipment(&pool3, tenant_b, "inbound").await;
            }
        },
        // Tenant A reads simultaneously
        async move {
            ShipmentRepository::list_shipments(&pool4, tenant_a, None, None, 200, 0)
                .await
                .expect("tenant A read")
        },
    );

    // Tenant B's read must return only tenant B's shipments
    for s in &read_b {
        assert_eq!(
            s.tenant_id, tenant_b,
            "tenant B read must not contain tenant A data"
        );
    }

    // Tenant A's read must return only tenant A's shipments
    for s in &read_a {
        assert_eq!(
            s.tenant_id, tenant_a,
            "tenant A read must not contain tenant B data"
        );
    }
}
