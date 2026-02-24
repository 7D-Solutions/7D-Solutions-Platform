//! Integration tests for party address CRUD.
//!
//! Covers: create, list, get, update, delete addresses; validation errors;
//! party-not-found guard; primary-address enforcement; and tenant isolation.

use party_rs::domain::address::{CreateAddressRequest, UpdateAddressRequest};
use party_rs::domain::address_service::{
    create_address, delete_address, get_address, list_addresses, update_address,
};
use party_rs::domain::party::service::create_company;
use party_rs::domain::party::{CreateCompanyRequest, PartyError};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://party_user:party_pass@localhost:5448/party_db".to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to party test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run party migrations");
    pool
}

fn unique_app() -> String {
    format!("address-test-{}", Uuid::new_v4().simple())
}

fn corr() -> String {
    Uuid::new_v4().to_string()
}

async fn make_party(pool: &sqlx::PgPool, app: &str, name: &str) -> Uuid {
    let req = CreateCompanyRequest {
        display_name: name.to_string(),
        legal_name: format!("{} LLC", name),
        trade_name: None,
        registration_number: None,
        tax_id: None,
        country_of_incorporation: None,
        industry_code: None,
        founded_date: None,
        employee_count: None,
        annual_revenue_cents: None,
        currency: None,
        email: None,
        phone: None,
        website: None,
        address_line1: None,
        address_line2: None,
        city: None,
        state: None,
        postal_code: None,
        country: None,
        metadata: None,
    };
    let view = create_company(pool, app, &req, corr()).await.expect("make_party failed");
    view.party.id
}

fn billing_address_req() -> CreateAddressRequest {
    CreateAddressRequest {
        address_type: Some("billing".to_string()),
        label: Some("HQ Billing".to_string()),
        line1: "123 Main St".to_string(),
        line2: Some("Suite 400".to_string()),
        city: "Springfield".to_string(),
        state: Some("IL".to_string()),
        postal_code: Some("62701".to_string()),
        country: Some("US".to_string()),
        is_primary: Some(false),
        metadata: None,
    }
}

fn shipping_address_req() -> CreateAddressRequest {
    CreateAddressRequest {
        address_type: Some("shipping".to_string()),
        label: None,
        line1: "456 Oak Ave".to_string(),
        line2: None,
        city: "Shelbyville".to_string(),
        state: Some("IL".to_string()),
        postal_code: Some("62565".to_string()),
        country: Some("US".to_string()),
        is_primary: Some(false),
        metadata: None,
    }
}

// ============================================================================
// Create address — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_address_happy_path() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Address Happy Corp").await;

    let addr = create_address(&pool, &app, party_id, &billing_address_req())
        .await
        .expect("create_address failed");

    assert_eq!(addr.line1, "123 Main St");
    assert_eq!(addr.city, "Springfield");
    assert_eq!(addr.address_type, "billing");
    assert_eq!(addr.party_id, party_id);
    assert_eq!(addr.app_id, app);
    assert!(!addr.is_primary);
}

// ============================================================================
// Create address — empty line1 → validation error
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_address_empty_line1() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Line1 Validation Corp").await;

    let mut req = billing_address_req();
    req.line1 = "  ".to_string();

    let err = create_address(&pool, &app, party_id, &req).await.unwrap_err();
    assert!(
        matches!(err, PartyError::Validation(_)),
        "expected Validation, got: {:?}",
        err
    );
}

// ============================================================================
// Create address — invalid address_type → validation error
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_address_invalid_type() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Type Validation Corp").await;

    let mut req = billing_address_req();
    req.address_type = Some("warehouse".to_string()); // not in enum

    let err = create_address(&pool, &app, party_id, &req).await.unwrap_err();
    assert!(
        matches!(err, PartyError::Validation(_)),
        "expected Validation, got: {:?}",
        err
    );
}

// ============================================================================
// Create address — party not found
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_address_party_not_found() {
    let pool = setup_db().await;
    let app = unique_app();

    let err = create_address(&pool, &app, Uuid::new_v4(), &billing_address_req())
        .await
        .unwrap_err();
    assert!(
        matches!(err, PartyError::NotFound(_)),
        "expected NotFound, got: {:?}",
        err
    );
}

// ============================================================================
// List addresses
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_addresses() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "List Addresses Corp").await;

    create_address(&pool, &app, party_id, &billing_address_req()).await.unwrap();
    create_address(&pool, &app, party_id, &shipping_address_req()).await.unwrap();

    let addrs = list_addresses(&pool, &app, party_id).await.unwrap();
    assert_eq!(addrs.len(), 2);
}

// ============================================================================
// Get address by ID
// ============================================================================

#[tokio::test]
#[serial]
async fn test_get_address() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Get Address Corp").await;

    let created = create_address(&pool, &app, party_id, &billing_address_req())
        .await
        .unwrap();

    let fetched = get_address(&pool, &app, created.id).await.unwrap().expect("address not found");

    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.line1, "123 Main St");
}

// ============================================================================
// Get address — wrong app_id returns None
// ============================================================================

#[tokio::test]
#[serial]
async fn test_get_address_wrong_app() {
    let pool = setup_db().await;
    let app = unique_app();
    let other_app = unique_app();
    let party_id = make_party(&pool, &app, "Wrong App Address Corp").await;

    let created = create_address(&pool, &app, party_id, &billing_address_req())
        .await
        .unwrap();

    let result = get_address(&pool, &other_app, created.id).await.unwrap();
    assert!(result.is_none(), "other app must not see this address");
}

// ============================================================================
// Update address
// ============================================================================

#[tokio::test]
#[serial]
async fn test_update_address() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Update Address Corp").await;

    let created = create_address(&pool, &app, party_id, &billing_address_req()).await.unwrap();

    let updated = update_address(
        &pool,
        &app,
        created.id,
        &UpdateAddressRequest {
            address_type: None,
            label: None,
            line1: None,
            line2: None,
            city: Some("Chicago".to_string()),
            state: None,
            postal_code: Some("60601".to_string()),
            country: None,
            is_primary: None,
            metadata: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.city, "Chicago");
    assert_eq!(updated.postal_code.as_deref(), Some("60601"));
    assert_eq!(updated.line1, "123 Main St"); // unchanged
}

// ============================================================================
// Update address — not found
// ============================================================================

#[tokio::test]
#[serial]
async fn test_update_address_not_found() {
    let pool = setup_db().await;
    let app = unique_app();

    let err = update_address(
        &pool,
        &app,
        Uuid::new_v4(),
        &UpdateAddressRequest {
            address_type: None,
            label: None,
            line1: Some("Ghost St".to_string()),
            line2: None,
            city: None,
            state: None,
            postal_code: None,
            country: None,
            is_primary: None,
            metadata: None,
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, PartyError::NotFound(_)), "expected NotFound, got: {:?}", err);
}

// ============================================================================
// Delete address
// ============================================================================

#[tokio::test]
#[serial]
async fn test_delete_address() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Delete Address Corp").await;

    let addr = create_address(&pool, &app, party_id, &billing_address_req()).await.unwrap();

    delete_address(&pool, &app, addr.id).await.unwrap();

    let result = get_address(&pool, &app, addr.id).await.unwrap();
    assert!(result.is_none(), "address should be gone after delete");
}

// ============================================================================
// Delete address — not found
// ============================================================================

#[tokio::test]
#[serial]
async fn test_delete_address_not_found() {
    let pool = setup_db().await;
    let app = unique_app();

    let err = delete_address(&pool, &app, Uuid::new_v4()).await.unwrap_err();
    assert!(matches!(err, PartyError::NotFound(_)), "expected NotFound, got: {:?}", err);
}

// ============================================================================
// Primary address — only one primary at a time
// ============================================================================

#[tokio::test]
#[serial]
async fn test_only_one_primary_address() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Primary Address Corp").await;

    let mut req1 = billing_address_req();
    req1.is_primary = Some(true);
    let first = create_address(&pool, &app, party_id, &req1).await.unwrap();
    assert!(first.is_primary);

    let mut req2 = shipping_address_req();
    req2.is_primary = Some(true);
    let second = create_address(&pool, &app, party_id, &req2).await.unwrap();
    assert!(second.is_primary);

    // First address should now be non-primary
    let first_refreshed = get_address(&pool, &app, first.id).await.unwrap().unwrap();
    assert!(!first_refreshed.is_primary, "original primary should be cleared");
}

// ============================================================================
// Tenant isolation — address from app_a not visible to app_b
// ============================================================================

#[tokio::test]
#[serial]
async fn test_address_tenant_isolation() {
    let pool = setup_db().await;
    let app_a = unique_app();
    let app_b = unique_app();

    let party_id = make_party(&pool, &app_a, "Isolation Address Corp").await;
    let addr = create_address(&pool, &app_a, party_id, &billing_address_req()).await.unwrap();

    // App B cannot read App A's address
    let result = get_address(&pool, &app_b, addr.id).await.unwrap();
    assert!(result.is_none(), "app_b must not see app_a's address");

    // App B delete attempt fails
    let err = delete_address(&pool, &app_b, addr.id).await.unwrap_err();
    assert!(matches!(err, PartyError::NotFound(_)));
}
