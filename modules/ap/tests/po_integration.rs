//! Integration tests for AP purchase order create + approve (bd-3rvu).
//!
//! Covers:
//! 1. Create PO — happy path
//! 2. Approve PO: draft → approved
//! 3. Approve PO idempotent
//! 4. Create PO with nonexistent vendor → VendorNotFound
//! 5. Tenant isolation

use ap::domain::po::approve::approve_po;
use ap::domain::po::queries::get_po;
use ap::domain::po::service::create_po;
use ap::domain::po::{ApprovePoRequest, CreatePoLineRequest, CreatePoRequest, PoError};
use ap::domain::vendors::qualification::change_qualification;
use ap::domain::vendors::service::create_vendor;
use ap::domain::vendors::{ChangeQualificationRequest, CreateVendorRequest, QualificationStatus};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://ap_user:ap_pass@localhost:5443/ap_db".to_string());
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to AP test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run AP migrations");
    pool
}

fn unique_tenant() -> String {
    format!("ap-po-{}", Uuid::new_v4().simple())
}

fn corr() -> String {
    Uuid::new_v4().to_string()
}

async fn make_vendor(pool: &sqlx::PgPool, tid: &str) -> Uuid {
    let vendor_id = create_vendor(
        pool,
        tid,
        &CreateVendorRequest {
            name: format!("Vendor-{}", Uuid::new_v4().simple()),
            tax_id: None,
            currency: "USD".to_string(),
            payment_terms_days: 30,
            payment_method: Some("ach".to_string()),
            remittance_email: None,
            party_id: None,
        },
        Uuid::new_v4().to_string(),
    )
    .await
    .unwrap()
    .vendor_id;

    change_qualification(
        pool, tid, vendor_id,
        &ChangeQualificationRequest {
            status: QualificationStatus::Qualified,
            notes: None,
            changed_by: "test-setup".to_string(),
        },
        Uuid::new_v4().to_string(),
    )
    .await
    .unwrap();

    vendor_id
}

fn sample_po_line() -> CreatePoLineRequest {
    CreatePoLineRequest {
        item_id: None,
        description: Some("Office Supplies".to_string()),
        quantity: 10.0,
        unit_of_measure: "each".to_string(),
        unit_price_minor: 5_000,
        gl_account_code: "6100".to_string(),
    }
}

// ============================================================================
// 1. Create PO — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_po() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    let result = create_po(
        &pool,
        &tid,
        &CreatePoRequest {
            vendor_id,
            currency: "USD".to_string(),
            created_by: "buyer-1".to_string(),
            expected_delivery_date: None,
            lines: vec![sample_po_line()],
        },
        corr(),
    )
    .await
    .unwrap();

    assert_eq!(result.po.status, "draft");
    assert_eq!(result.po.vendor_id, vendor_id);
    assert_eq!(result.lines.len(), 1);
    assert_eq!(result.lines[0].unit_price_minor, 5_000);
    assert_eq!(result.po.total_minor, 50_000); // 10 × 5_000

    let fetched = get_po(&pool, &tid, result.po.po_id).await.unwrap().unwrap();
    assert_eq!(fetched.po.po_id, result.po.po_id);
}

// ============================================================================
// 2. Approve PO: draft → approved
// ============================================================================

#[tokio::test]
#[serial]
async fn test_approve_po() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    let result = create_po(
        &pool,
        &tid,
        &CreatePoRequest {
            vendor_id,
            currency: "USD".to_string(),
            created_by: "buyer-1".to_string(),
            expected_delivery_date: None,
            lines: vec![sample_po_line()],
        },
        corr(),
    )
    .await
    .unwrap();

    let approved = approve_po(
        &pool,
        &tid,
        result.po.po_id,
        &ApprovePoRequest {
            approved_by: "manager-1".to_string(),
        },
        corr(),
    )
    .await
    .unwrap();

    assert_eq!(approved.status, "approved");
}

// ============================================================================
// 3. Approve PO idempotent (call twice → same outcome, no error)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_approve_po_idempotent() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    let result = create_po(
        &pool,
        &tid,
        &CreatePoRequest {
            vendor_id,
            currency: "USD".to_string(),
            created_by: "buyer-1".to_string(),
            expected_delivery_date: None,
            lines: vec![sample_po_line()],
        },
        corr(),
    )
    .await
    .unwrap();

    let req = ApprovePoRequest {
        approved_by: "manager-1".to_string(),
    };
    let first = approve_po(&pool, &tid, result.po.po_id, &req, corr())
        .await
        .unwrap();
    let second = approve_po(&pool, &tid, result.po.po_id, &req, corr())
        .await
        .unwrap();

    assert_eq!(first.status, "approved");
    assert_eq!(second.status, "approved");
    assert_eq!(first.po_id, second.po_id);
}

// ============================================================================
// 4. Create PO with nonexistent vendor → VendorNotFound
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_po_vendor_not_found() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let err = create_po(
        &pool,
        &tid,
        &CreatePoRequest {
            vendor_id: Uuid::new_v4(), // nonexistent
            currency: "USD".to_string(),
            created_by: "buyer-1".to_string(),
            expected_delivery_date: None,
            lines: vec![sample_po_line()],
        },
        corr(),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, PoError::VendorNotFound(_)),
        "expected VendorNotFound, got: {:?}",
        err
    );
}

// ============================================================================
// 5. Tenant isolation — cross-tenant PO access fails
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_isolation_po() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();

    let vendor_id = make_vendor(&pool, &tid_a).await;

    let result = create_po(
        &pool,
        &tid_a,
        &CreatePoRequest {
            vendor_id,
            currency: "USD".to_string(),
            created_by: "buyer-1".to_string(),
            expected_delivery_date: None,
            lines: vec![sample_po_line()],
        },
        corr(),
    )
    .await
    .unwrap();

    // Tenant B cannot read tenant A's PO
    let fetched = get_po(&pool, &tid_b, result.po.po_id).await.unwrap();
    assert!(fetched.is_none());
}
