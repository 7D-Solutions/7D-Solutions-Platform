//! Integration tests for party contact CRUD.
//!
//! Covers: create, list, get, update, delete contacts; validation errors;
//! party-not-found guard; and tenant isolation.

use party_rs::domain::contact::{CreateContactRequest, UpdateContactRequest};
use party_rs::domain::contact_service::{
    create_contact, delete_contact, get_contact, list_contacts, update_contact,
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
    format!("contact-test-{}", Uuid::new_v4().simple())
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

fn base_contact_req(first: &str, last: &str) -> CreateContactRequest {
    CreateContactRequest {
        first_name: first.to_string(),
        last_name: last.to_string(),
        email: Some(format!("{}.{}@example.com", first.to_lowercase(), last.to_lowercase())),
        phone: Some("+1-555-0100".to_string()),
        role: Some("Manager".to_string()),
        is_primary: Some(false),
        metadata: None,
    }
}

// ============================================================================
// Create contact — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_contact_happy_path() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Contact Happy Corp").await;

    let contact = create_contact(&pool, &app, party_id, &base_contact_req("Alice", "Smith"))
        .await
        .expect("create_contact failed");

    assert_eq!(contact.first_name, "Alice");
    assert_eq!(contact.last_name, "Smith");
    assert_eq!(contact.party_id, party_id);
    assert_eq!(contact.app_id, app);
    assert!(!contact.is_primary);
}

// ============================================================================
// Create contact — empty first_name → validation error
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_contact_empty_first_name() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Validation Corp").await;

    let mut req = base_contact_req("Bob", "Jones");
    req.first_name = "".to_string();

    let err = create_contact(&pool, &app, party_id, &req).await.unwrap_err();
    assert!(
        matches!(err, PartyError::Validation(_)),
        "expected Validation, got: {:?}",
        err
    );
}

// ============================================================================
// Create contact — invalid email → validation error
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_contact_invalid_email() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Email Validation Corp").await;

    let mut req = base_contact_req("Carol", "White");
    req.email = Some("not-an-email".to_string());

    let err = create_contact(&pool, &app, party_id, &req).await.unwrap_err();
    assert!(
        matches!(err, PartyError::Validation(_)),
        "expected Validation, got: {:?}",
        err
    );
}

// ============================================================================
// Create contact — party not found
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_contact_party_not_found() {
    let pool = setup_db().await;
    let app = unique_app();

    let err = create_contact(&pool, &app, Uuid::new_v4(), &base_contact_req("Dan", "Brown"))
        .await
        .unwrap_err();
    assert!(
        matches!(err, PartyError::NotFound(_)),
        "expected NotFound, got: {:?}",
        err
    );
}

// ============================================================================
// List contacts
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_contacts() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "List Contacts Corp").await;

    create_contact(&pool, &app, party_id, &base_contact_req("Eve", "Adams")).await.unwrap();
    create_contact(&pool, &app, party_id, &base_contact_req("Frank", "Baker")).await.unwrap();

    let contacts = list_contacts(&pool, &app, party_id).await.unwrap();
    assert_eq!(contacts.len(), 2);
}

// ============================================================================
// Get contact
// ============================================================================

#[tokio::test]
#[serial]
async fn test_get_contact() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Get Contact Corp").await;

    let created = create_contact(&pool, &app, party_id, &base_contact_req("Grace", "Hall"))
        .await
        .unwrap();

    let fetched = get_contact(&pool, &app, created.id)
        .await
        .unwrap()
        .expect("contact not found");

    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.first_name, "Grace");
}

// ============================================================================
// Get contact — wrong app_id returns None
// ============================================================================

#[tokio::test]
#[serial]
async fn test_get_contact_wrong_app() {
    let pool = setup_db().await;
    let app = unique_app();
    let other_app = unique_app();
    let party_id = make_party(&pool, &app, "Wrong App Corp").await;

    let created = create_contact(&pool, &app, party_id, &base_contact_req("Hank", "Irwin"))
        .await
        .unwrap();

    let result = get_contact(&pool, &other_app, created.id).await.unwrap();
    assert!(result.is_none(), "other app must not see this contact");
}

// ============================================================================
// Update contact
// ============================================================================

#[tokio::test]
#[serial]
async fn test_update_contact() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Update Contact Corp").await;

    let created = create_contact(&pool, &app, party_id, &base_contact_req("Ivan", "Jones"))
        .await
        .unwrap();

    let updated = update_contact(
        &pool,
        &app,
        created.id,
        &UpdateContactRequest {
            first_name: Some("Ivan-Updated".to_string()),
            last_name: None,
            email: Some("ivan.updated@example.com".to_string()),
            phone: None,
            role: None,
            is_primary: None,
            metadata: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.first_name, "Ivan-Updated");
    assert_eq!(updated.email.as_deref(), Some("ivan.updated@example.com"));
}

// ============================================================================
// Update contact — not found
// ============================================================================

#[tokio::test]
#[serial]
async fn test_update_contact_not_found() {
    let pool = setup_db().await;
    let app = unique_app();

    let err = update_contact(
        &pool,
        &app,
        Uuid::new_v4(),
        &UpdateContactRequest {
            first_name: Some("Ghost".to_string()),
            last_name: None,
            email: None,
            phone: None,
            role: None,
            is_primary: None,
            metadata: None,
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, PartyError::NotFound(_)), "expected NotFound, got: {:?}", err);
}

// ============================================================================
// Delete contact
// ============================================================================

#[tokio::test]
#[serial]
async fn test_delete_contact() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Delete Contact Corp").await;

    let contact = create_contact(&pool, &app, party_id, &base_contact_req("Jack", "King"))
        .await
        .unwrap();

    delete_contact(&pool, &app, contact.id).await.unwrap();

    let result = get_contact(&pool, &app, contact.id).await.unwrap();
    assert!(result.is_none(), "contact should be gone after delete");
}

// ============================================================================
// Delete contact — not found
// ============================================================================

#[tokio::test]
#[serial]
async fn test_delete_contact_not_found() {
    let pool = setup_db().await;
    let app = unique_app();

    let err = delete_contact(&pool, &app, Uuid::new_v4()).await.unwrap_err();
    assert!(matches!(err, PartyError::NotFound(_)), "expected NotFound, got: {:?}", err);
}

// ============================================================================
// Primary contact — only one primary at a time
// ============================================================================

#[tokio::test]
#[serial]
async fn test_only_one_primary_contact() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Primary Contact Corp").await;

    let mut req1 = base_contact_req("Lara", "Moon");
    req1.is_primary = Some(true);
    let first = create_contact(&pool, &app, party_id, &req1).await.unwrap();
    assert!(first.is_primary);

    let mut req2 = base_contact_req("Mike", "Nash");
    req2.is_primary = Some(true);
    let second = create_contact(&pool, &app, party_id, &req2).await.unwrap();
    assert!(second.is_primary);

    // First contact should now be non-primary
    let first_refreshed = get_contact(&pool, &app, first.id).await.unwrap().unwrap();
    assert!(!first_refreshed.is_primary, "original primary should be cleared");
}

// ============================================================================
// Tenant isolation — contact from app_a not visible to app_b
// ============================================================================

#[tokio::test]
#[serial]
async fn test_contact_tenant_isolation() {
    let pool = setup_db().await;
    let app_a = unique_app();
    let app_b = unique_app();

    let party_id = make_party(&pool, &app_a, "Isolation Corp").await;
    let contact =
        create_contact(&pool, &app_a, party_id, &base_contact_req("Nora", "Owen")).await.unwrap();

    // App B cannot read App A's contact
    let result = get_contact(&pool, &app_b, contact.id).await.unwrap();
    assert!(result.is_none(), "app_b must not see app_a's contact");

    // App B delete attempt fails
    let err = delete_contact(&pool, &app_b, contact.id).await.unwrap_err();
    assert!(matches!(err, PartyError::NotFound(_)));
}
