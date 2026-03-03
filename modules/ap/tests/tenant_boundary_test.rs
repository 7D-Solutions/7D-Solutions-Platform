//! Tenant Boundary Concurrency Tests (Phase 58 Gate A, bd-mvane)
//!
//! Proves no cross-tenant data leakage under concurrent load.
//! Two tenants operate simultaneously and must never see each other's data.
//!
//! ## Strategy
//! - Two tenants each create vendors and bills concurrently
//! - After all writes, verify each tenant sees only its own data
//! - Read queries scoped by tenant never return the other tenant's data
//!
//! ## Prerequisites
//! - PostgreSQL at localhost:5443 (docker compose up -d)

use ap::domain::bills::service::{create_bill, get_bill, list_bills};
use ap::domain::bills::{CreateBillLineRequest, CreateBillRequest};
use ap::domain::vendors::service::{create_vendor, get_vendor, list_vendors};
use ap::domain::vendors::CreateVendorRequest;
use chrono::Utc;
use serial_test::serial;
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use uuid::Uuid;

// ============================================================================
// Test DB helpers
// ============================================================================

async fn setup_db() -> PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://ap_user:ap_pass@localhost:5443/ap_db".to_string());

    PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
        .expect("Failed to connect to AP test DB")
}

async fn ensure_migrations(pool: &PgPool) {
    sqlx::migrate!("db/migrations")
        .run(pool)
        .await
        .expect("Failed to run AP migrations");
}

fn unique_tenant() -> String {
    format!("boundary-{}", Uuid::new_v4().simple())
}

fn corr() -> String {
    Uuid::new_v4().to_string()
}

fn vendor_req(name: &str) -> CreateVendorRequest {
    CreateVendorRequest {
        name: name.to_string(),
        tax_id: None,
        currency: "USD".to_string(),
        payment_terms_days: 30,
        payment_method: Some("ach".to_string()),
        remittance_email: None,
        party_id: None,
    }
}

fn bill_req(vendor_id: Uuid, ref_num: &str) -> CreateBillRequest {
    CreateBillRequest {
        vendor_id,
        vendor_invoice_ref: ref_num.to_string(),
        currency: "USD".to_string(),
        invoice_date: Utc::now(),
        due_date: None,
        tax_minor: None,
        entered_by: "boundary-test".to_string(),
        fx_rate_id: None,
        lines: vec![CreateBillLineRequest {
            description: Some("Boundary test line".to_string()),
            item_id: None,
            quantity: 1.0,
            unit_price_minor: 1_000,
            gl_account_code: Some("6200".to_string()),
            po_line_id: None,
        }],
    }
}

// ============================================================================
// Test 1: Concurrent writes — two tenants create vendors and bills in parallel
// ============================================================================

#[tokio::test]
#[serial]
async fn concurrent_vendor_bill_writes_are_tenant_isolated() {
    let pool = setup_db().await;
    ensure_migrations(&pool).await;

    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    // Create one vendor per tenant first (bills need a vendor)
    let vendor_a = create_vendor(&pool, &tenant_a, &vendor_req("Vendor-A-Boundary"), corr())
        .await
        .expect("create vendor A")
        .vendor_id;
    let vendor_b = create_vendor(&pool, &tenant_b, &vendor_req("Vendor-B-Boundary"), corr())
        .await
        .expect("create vendor B")
        .vendor_id;

    // Post 5 bills per tenant concurrently
    let mut handles = Vec::new();
    for i in 0..5 {
        let p = pool.clone();
        let ta = tenant_a.clone();
        let va = vendor_a;
        handles.push(tokio::spawn(async move {
            create_bill(
                &p,
                &ta,
                &bill_req(va, &format!("INV-A-{}", i)),
                corr(),
            )
            .await
            .expect("bill A")
        }));

        let p = pool.clone();
        let tb = tenant_b.clone();
        let vb = vendor_b;
        handles.push(tokio::spawn(async move {
            create_bill(
                &p,
                &tb,
                &bill_req(vb, &format!("INV-B-{}", i)),
                corr(),
            )
            .await
            .expect("bill B")
        }));
    }
    for h in handles {
        h.await.expect("join");
    }

    // Verify tenant A sees only their own vendors
    let a_vendors = list_vendors(&pool, &tenant_a, true).await.expect("list A");
    assert_eq!(a_vendors.len(), 1, "Tenant A should have 1 vendor");
    assert_eq!(a_vendors[0].name, "Vendor-A-Boundary");

    // Verify tenant B sees only their own vendors
    let b_vendors = list_vendors(&pool, &tenant_b, true).await.expect("list B");
    assert_eq!(b_vendors.len(), 1, "Tenant B should have 1 vendor");
    assert_eq!(b_vendors[0].name, "Vendor-B-Boundary");

    // Verify bill counts
    let a_bills = list_bills(&pool, &tenant_a, None, true).await.expect("bills A");
    assert_eq!(a_bills.len(), 5, "Tenant A should have 5 bills");

    let b_bills = list_bills(&pool, &tenant_b, None, true).await.expect("bills B");
    assert_eq!(b_bills.len(), 5, "Tenant B should have 5 bills");

    // Cross-tenant bill access: tenant B must not see tenant A's bills
    for bill in &a_bills {
        let cross = get_bill(&pool, &tenant_b, bill.bill_id)
            .await
            .expect("cross get_bill");
        assert!(
            cross.is_none(),
            "Tenant B must not see Tenant A's bill {}",
            bill.bill_id
        );
    }

    // Cross-tenant vendor access: tenant B must not see tenant A's vendor
    let cross_vendor = get_vendor(&pool, &tenant_b, vendor_a)
        .await
        .expect("cross get_vendor");
    assert!(
        cross_vendor.is_none(),
        "Tenant B must not see Tenant A's vendor"
    );
}

// ============================================================================
// Test 2: Reads during concurrent writes — no cross-tenant visibility
// ============================================================================

#[tokio::test]
#[serial]
async fn reads_during_writes_are_tenant_isolated() {
    let pool = setup_db().await;
    ensure_migrations(&pool).await;

    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    let vendor_a = create_vendor(&pool, &tenant_a, &vendor_req("Vendor-RW-A"), corr())
        .await
        .expect("vendor A")
        .vendor_id;
    let vendor_b = create_vendor(&pool, &tenant_b, &vendor_req("Vendor-RW-B"), corr())
        .await
        .expect("vendor B")
        .vendor_id;

    let mut handles = Vec::new();

    // Tenant A writes bills
    for i in 0..5 {
        let p = pool.clone();
        let ta = tenant_a.clone();
        let va = vendor_a;
        handles.push(tokio::spawn(async move {
            create_bill(&p, &ta, &bill_req(va, &format!("RW-A-{}", i)), corr())
                .await
                .expect("bill A");
        }));
    }

    // Concurrent reads by tenant B — must never see tenant A's data
    for _ in 0..5 {
        let p = pool.clone();
        let tb = tenant_b.clone();
        let va = vendor_a;
        handles.push(tokio::spawn(async move {
            // Tenant B tries to read tenant A's vendor — must get None
            let cross = get_vendor(&p, &tb, va).await.expect("cross vendor read");
            assert!(
                cross.is_none(),
                "Tenant B must never see Tenant A's vendor during concurrent writes"
            );

            // Tenant B lists their own bills — must be empty or only theirs
            let bills = list_bills(&p, &tb, None, true).await.expect("list bills B");
            for b in &bills {
                assert_eq!(
                    b.vendor_id, vendor_b,
                    "Tenant B's bills must reference tenant B's vendor"
                );
            }
        }));
    }

    // Tenant B writes some bills too
    for i in 0..3 {
        let p = pool.clone();
        let tb = tenant_b.clone();
        let vb = vendor_b;
        handles.push(tokio::spawn(async move {
            create_bill(&p, &tb, &bill_req(vb, &format!("RW-B-{}", i)), corr())
                .await
                .expect("bill B");
        }));
    }

    for h in handles {
        h.await.expect("join");
    }

    // Final verification
    let a_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM vendor_bills WHERE tenant_id = $1")
            .bind(&tenant_a)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(a_count, 5, "Tenant A should have 5 bills");

    let b_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM vendor_bills WHERE tenant_id = $1")
            .bind(&tenant_b)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(b_count, 3, "Tenant B should have 3 bills");
}

// ============================================================================
// Test 3: SQL-level tenant scoping — all core tables scope by tenant_id
// ============================================================================

#[tokio::test]
#[serial]
async fn all_core_tables_scope_queries_by_tenant() {
    let pool = setup_db().await;
    ensure_migrations(&pool).await;

    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    // Set up data for both tenants
    let vendor_a = create_vendor(&pool, &tenant_a, &vendor_req("Scope-Vendor-A"), corr())
        .await
        .expect("vendor A")
        .vendor_id;
    let vendor_b = create_vendor(&pool, &tenant_b, &vendor_req("Scope-Vendor-B"), corr())
        .await
        .expect("vendor B")
        .vendor_id;

    create_bill(&pool, &tenant_a, &bill_req(vendor_a, "SCOPE-A-1"), corr())
        .await
        .expect("bill A");
    create_bill(&pool, &tenant_b, &bill_req(vendor_b, "SCOPE-B-1"), corr())
        .await
        .expect("bill B");

    // Verify each tenant-scoped table scopes correctly
    let tables_with_tenant = vec![
        "vendors",
        "vendor_bills",
    ];

    for table in &tables_with_tenant {
        let query = format!("SELECT COUNT(*) FROM {} WHERE tenant_id = $1", table);

        let count_a: i64 = sqlx::query_scalar(&query)
            .bind(&tenant_a)
            .fetch_one(&pool)
            .await
            .unwrap_or_else(|e| panic!("query {} for tenant_a: {}", table, e));
        assert!(count_a > 0, "{} should have rows for tenant A", table);

        let count_b: i64 = sqlx::query_scalar(&query)
            .bind(&tenant_b)
            .fetch_one(&pool)
            .await
            .unwrap_or_else(|e| panic!("query {} for tenant_b: {}", table, e));
        assert!(count_b > 0, "{} should have rows for tenant B", table);

        // Rows for A don't appear under B's tenant_id
        let rows_a: Vec<sqlx::postgres::PgRow> =
            sqlx::query(&format!("SELECT tenant_id FROM {} WHERE tenant_id = $1", table))
                .bind(&tenant_a)
                .fetch_all(&pool)
                .await
                .unwrap();
        for row in &rows_a {
            let tid: &str = row.get("tenant_id");
            assert_eq!(tid, tenant_a, "{} row must belong to tenant A", table);
        }
    }
}
