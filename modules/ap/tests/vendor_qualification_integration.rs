//! Integration tests for AP vendor qualification gate (bd-vf7mt).
//!
//! Covers:
//! 1. New vendor defaults to 'unqualified' — PO creation blocked
//! 2. Qualify vendor → PO creation succeeds
//! 3. Disqualify vendor → PO creation blocked again with VENDOR_NOT_ELIGIBLE
//! 4. Restricted vendor → PO creation allowed
//! 5. Pending-review vendor → PO creation blocked
//! 6. mark_preferred / unmark_preferred round-trip
//! 7. get_qualification_history returns ordered audit trail
//! 8. Tenant isolation: vendor from tenant A not visible in tenant B qualification history

use ap::domain::po::service::create_po;
use ap::domain::po::{CreatePoLineRequest, CreatePoRequest, PoError};
use ap::domain::vendors::qualification::{
    change_qualification, get_qualification_history, mark_preferred, unmark_preferred,
};
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
    format!("ap-qual-{}", Uuid::new_v4().simple())
}

fn corr() -> String {
    Uuid::new_v4().to_string()
}

async fn make_vendor(pool: &sqlx::PgPool, tid: &str) -> Uuid {
    create_vendor(
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
        corr(),
    )
    .await
    .unwrap()
    .vendor_id
}

fn sample_po_line() -> CreatePoLineRequest {
    CreatePoLineRequest {
        item_id: None,
        description: Some("Test Part".to_string()),
        quantity: 1.0,
        unit_of_measure: "each".to_string(),
        unit_price_minor: 10_000,
        gl_account_code: "6100".to_string(),
    }
}

// ============================================================================
// 1. New vendor defaults to 'unqualified' — PO creation blocked
// ============================================================================

#[tokio::test]
#[serial]
async fn new_vendor_is_unqualified_and_blocks_po() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    // Verify default qualification status
    let vendor = ap::domain::vendors::repo::fetch_vendor(&pool, &tid, vendor_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(vendor.qualification_status, "unqualified");

    // Attempt to create a PO — should fail
    let req = CreatePoRequest {
        vendor_id,
        currency: "USD".to_string(),
        lines: vec![sample_po_line()],
        created_by: "buyer-1".to_string(),
        expected_delivery_date: None,
    };
    let result = create_po(&pool, &tid, &req, corr()).await;
    assert!(
        matches!(result, Err(PoError::VendorNotEligible(_, _))),
        "expected VendorNotEligible for unqualified vendor, got: {:?}",
        result
    );
}

// ============================================================================
// 2. Qualify vendor → PO creation succeeds
// ============================================================================

#[tokio::test]
#[serial]
async fn qualified_vendor_allows_po() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    change_qualification(
        &pool,
        &tid,
        vendor_id,
        &ChangeQualificationRequest {
            status: QualificationStatus::Qualified,
            notes: Some("AS9100 audit passed".to_string()),
            changed_by: "quality-manager".to_string(),
        },
        corr(),
    )
    .await
    .expect("qualify failed");

    let req = CreatePoRequest {
        vendor_id,
        currency: "USD".to_string(),
        lines: vec![sample_po_line()],
        created_by: "buyer-1".to_string(),
        expected_delivery_date: None,
    };
    let result = create_po(&pool, &tid, &req, corr()).await;
    assert!(
        result.is_ok(),
        "expected PO to succeed for qualified vendor, got: {:?}",
        result
    );
}

// ============================================================================
// 3. Disqualify vendor → PO creation blocked with correct error code
// ============================================================================

#[tokio::test]
#[serial]
async fn disqualified_vendor_blocks_po() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    // First qualify, then disqualify
    change_qualification(
        &pool,
        &tid,
        vendor_id,
        &ChangeQualificationRequest {
            status: QualificationStatus::Qualified,
            notes: None,
            changed_by: "qm".to_string(),
        },
        corr(),
    )
    .await
    .expect("qualify failed");

    change_qualification(
        &pool,
        &tid,
        vendor_id,
        &ChangeQualificationRequest {
            status: QualificationStatus::Disqualified,
            notes: Some("Failed re-audit".to_string()),
            changed_by: "qm".to_string(),
        },
        corr(),
    )
    .await
    .expect("disqualify failed");

    let req = CreatePoRequest {
        vendor_id,
        currency: "USD".to_string(),
        lines: vec![sample_po_line()],
        created_by: "buyer-1".to_string(),
        expected_delivery_date: None,
    };
    let result = create_po(&pool, &tid, &req, corr()).await;
    assert!(
        matches!(result, Err(PoError::VendorNotEligible(_, ref s)) if s == "disqualified"),
        "expected VendorNotEligible(disqualified), got: {:?}",
        result
    );
}

// ============================================================================
// 4. Restricted vendor → PO creation allowed
// ============================================================================

#[tokio::test]
#[serial]
async fn restricted_vendor_allows_po() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    change_qualification(
        &pool,
        &tid,
        vendor_id,
        &ChangeQualificationRequest {
            status: QualificationStatus::Restricted,
            notes: Some("Limited to sub-$5k orders".to_string()),
            changed_by: "qm".to_string(),
        },
        corr(),
    )
    .await
    .expect("restrict failed");

    let req = CreatePoRequest {
        vendor_id,
        currency: "USD".to_string(),
        lines: vec![sample_po_line()],
        created_by: "buyer-1".to_string(),
        expected_delivery_date: None,
    };
    let result = create_po(&pool, &tid, &req, corr()).await;
    assert!(
        result.is_ok(),
        "expected PO to succeed for restricted vendor, got: {:?}",
        result
    );
}

// ============================================================================
// 5. Pending-review vendor → PO creation blocked
// ============================================================================

#[tokio::test]
#[serial]
async fn pending_review_vendor_blocks_po() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    change_qualification(
        &pool,
        &tid,
        vendor_id,
        &ChangeQualificationRequest {
            status: QualificationStatus::PendingReview,
            notes: Some("Awaiting audit".to_string()),
            changed_by: "qm".to_string(),
        },
        corr(),
    )
    .await
    .expect("set pending_review failed");

    let req = CreatePoRequest {
        vendor_id,
        currency: "USD".to_string(),
        lines: vec![sample_po_line()],
        created_by: "buyer-1".to_string(),
        expected_delivery_date: None,
    };
    let result = create_po(&pool, &tid, &req, corr()).await;
    assert!(
        matches!(result, Err(PoError::VendorNotEligible(_, _))),
        "expected VendorNotEligible for pending_review vendor, got: {:?}",
        result
    );
}

// ============================================================================
// 6. mark_preferred / unmark_preferred round-trip
// ============================================================================

#[tokio::test]
#[serial]
async fn preferred_flag_round_trip() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    // Default: not preferred
    let before = ap::domain::vendors::repo::fetch_vendor(&pool, &tid, vendor_id)
        .await
        .unwrap()
        .unwrap();
    assert!(!before.preferred_vendor);

    // Mark preferred
    let marked = mark_preferred(&pool, &tid, vendor_id, "buyer-1")
        .await
        .expect("mark_preferred failed");
    assert!(marked.preferred_vendor);

    // Unmark preferred
    let unmarked = unmark_preferred(&pool, &tid, vendor_id, "buyer-1")
        .await
        .expect("unmark_preferred failed");
    assert!(!unmarked.preferred_vendor);
}

// ============================================================================
// 7. get_qualification_history returns ordered audit trail
// ============================================================================

#[tokio::test]
#[serial]
async fn qualification_history_returns_ordered_audit_trail() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    // Transition 1: unqualified → pending_review
    change_qualification(
        &pool,
        &tid,
        vendor_id,
        &ChangeQualificationRequest {
            status: QualificationStatus::PendingReview,
            notes: None,
            changed_by: "qm".to_string(),
        },
        corr(),
    )
    .await
    .unwrap();

    // Transition 2: pending_review → qualified
    change_qualification(
        &pool,
        &tid,
        vendor_id,
        &ChangeQualificationRequest {
            status: QualificationStatus::Qualified,
            notes: Some("Audit passed".to_string()),
            changed_by: "qm".to_string(),
        },
        corr(),
    )
    .await
    .unwrap();

    let history = get_qualification_history(&pool, &tid, vendor_id)
        .await
        .expect("get_qualification_history failed");

    assert_eq!(history.len(), 2, "expected 2 history entries");
    // Most recent first
    assert_eq!(history[0].to_status, "qualified");
    assert_eq!(history[1].to_status, "pending_review");
    assert_eq!(history[0].from_status.as_deref(), Some("pending_review"));
    assert_eq!(history[1].from_status.as_deref(), Some("unqualified"));
}

// ============================================================================
// 8. Tenant isolation: history from tenant A not visible in tenant B
// ============================================================================

#[tokio::test]
#[serial]
async fn qualification_history_is_tenant_isolated() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid_a).await;

    change_qualification(
        &pool,
        &tid_a,
        vendor_id,
        &ChangeQualificationRequest {
            status: QualificationStatus::Qualified,
            notes: None,
            changed_by: "qm".to_string(),
        },
        corr(),
    )
    .await
    .unwrap();

    // Tenant B should see empty history for a vendor that belongs to tenant A
    let history_b = get_qualification_history(&pool, &tid_b, vendor_id)
        .await
        .expect("history query failed");

    assert!(
        history_b.is_empty(),
        "tenant B should not see tenant A's qualification history"
    );
}
