//! Integration tests for AP vendor CRUD (bd-3rvu).
//!
//! Covers:
//! 1. Create vendor — happy path
//! 2. Duplicate name rejection
//! 3. Get vendor not found
//! 4. List vendors (active filter)
//! 5. Update vendor
//! 6. Deactivate vendor
//! 7. Tenant isolation

use ap::domain::vendors::service::{
    create_vendor, deactivate_vendor, get_vendor, list_vendors, update_vendor,
};
use ap::domain::vendors::{CreateVendorRequest, UpdateVendorRequest, VendorError};
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
    format!("ap-vendor-{}", Uuid::new_v4().simple())
}

fn corr() -> String {
    Uuid::new_v4().to_string()
}

fn base_vendor_req(name: &str) -> CreateVendorRequest {
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

// ============================================================================
// 1. Create vendor — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_vendor() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let vendor = create_vendor(&pool, &tid, &base_vendor_req("Acme Corp"), corr())
        .await
        .unwrap();

    assert_eq!(vendor.name, "Acme Corp");
    assert_eq!(vendor.currency, "USD");
    assert_eq!(vendor.payment_terms_days, 30);
    assert!(vendor.is_active);

    let fetched = get_vendor(&pool, &tid, vendor.vendor_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.vendor_id, vendor.vendor_id);
}

// ============================================================================
// 2. Duplicate vendor name → DuplicateName error
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_vendor_duplicate_name_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    create_vendor(&pool, &tid, &base_vendor_req("Duplicate Supplies"), corr())
        .await
        .unwrap();

    let err = create_vendor(&pool, &tid, &base_vendor_req("Duplicate Supplies"), corr())
        .await
        .unwrap_err();

    assert!(
        matches!(err, VendorError::DuplicateName(_)),
        "expected DuplicateName, got: {:?}",
        err
    );
}

// ============================================================================
// 3. Get vendor not found → None
// ============================================================================

#[tokio::test]
#[serial]
async fn test_get_vendor_not_found() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let result = get_vendor(&pool, &tid, Uuid::new_v4()).await.unwrap();
    assert!(result.is_none());
}

// ============================================================================
// 4. List vendors (active filter)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_vendors() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    create_vendor(&pool, &tid, &base_vendor_req("List Vendor A"), corr())
        .await
        .unwrap();
    create_vendor(&pool, &tid, &base_vendor_req("List Vendor B"), corr())
        .await
        .unwrap();

    let active = list_vendors(&pool, &tid, false).await.unwrap();
    assert_eq!(active.len(), 2);

    let all = list_vendors(&pool, &tid, true).await.unwrap();
    assert_eq!(all.len(), 2);
}

// ============================================================================
// 5. Update vendor
// ============================================================================

#[tokio::test]
#[serial]
async fn test_update_vendor() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let vendor = create_vendor(&pool, &tid, &base_vendor_req("Old Name Corp"), corr())
        .await
        .unwrap();

    let updated = update_vendor(
        &pool,
        &tid,
        vendor.vendor_id,
        &UpdateVendorRequest {
            name: Some("New Name Corp".to_string()),
            tax_id: None,
            currency: None,
            payment_terms_days: Some(60),
            payment_method: None,
            remittance_email: None,
            updated_by: Some("admin".to_string()),
            party_id: None,
        },
        corr(),
    )
    .await
    .unwrap();

    assert_eq!(updated.name, "New Name Corp");
    assert_eq!(updated.payment_terms_days, 60);
}

// ============================================================================
// 6. Deactivate vendor
// ============================================================================

#[tokio::test]
#[serial]
async fn test_deactivate_vendor() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let vendor = create_vendor(&pool, &tid, &base_vendor_req("Deactivate Me Corp"), corr())
        .await
        .unwrap();

    deactivate_vendor(&pool, &tid, vendor.vendor_id, "admin", corr())
        .await
        .unwrap();

    // Not in active list
    let active = list_vendors(&pool, &tid, false).await.unwrap();
    assert!(active.is_empty());

    // Appears in include-inactive list
    let all = list_vendors(&pool, &tid, true).await.unwrap();
    assert_eq!(all.len(), 1);
    assert!(!all[0].is_active);
}

// ============================================================================
// 7. Tenant isolation — cross-tenant vendor access fails
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_isolation_vendors() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();

    let vendor = create_vendor(&pool, &tid_a, &base_vendor_req("Tenant A Vendor"), corr())
        .await
        .unwrap();

    // Tenant B cannot read tenant A's vendor
    let result = get_vendor(&pool, &tid_b, vendor.vendor_id).await.unwrap();
    assert!(result.is_none());

    let list = list_vendors(&pool, &tid_b, false).await.unwrap();
    assert!(list.is_empty());
}
