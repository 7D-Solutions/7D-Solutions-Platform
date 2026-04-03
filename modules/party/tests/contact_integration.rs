//! Integration tests for party contact CRUD.
//!
//! Covers: create, list, get, update, deactivate (soft-delete); validation;
//! party-not-found guard; tenant isolation; set-primary per role;
//! primary-contacts query; and outbox events.

use party_rs::domain::contact::{CreateContactRequest, UpdateContactRequest};
use party_rs::domain::contact_service::{
    create_contact, deactivate_contact, get_contact, get_primary_contacts, list_contacts,
    set_primary_for_role, update_contact,
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
    let view = create_company(pool, app, &req, corr())
        .await
        .expect("make_party failed");
    view.party.id
}

fn base_contact_req(first: &str, last: &str) -> CreateContactRequest {
    CreateContactRequest {
        first_name: first.to_string(),
        last_name: Some(last.to_string()),
        email: Some(format!(
            "{}.{}@example.com",
            first.to_lowercase(),
            last.to_lowercase()
        )),
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

    let contact = create_contact(
        &pool,
        &app,
        party_id,
        &base_contact_req("Alice", "Smith"),
        corr(),
    )
    .await
    .expect("create_contact failed");

    assert_eq!(contact.first_name, "Alice");
    assert_eq!(contact.last_name, Some("Smith".to_string()));
    assert_eq!(contact.party_id, party_id);
    assert_eq!(contact.app_id, app);
    assert!(!contact.is_primary);
    assert!(contact.deactivated_at.is_none());
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

    let err = create_contact(&pool, &app, party_id, &req, corr())
        .await
        .unwrap_err();
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

    let err = create_contact(&pool, &app, party_id, &req, corr())
        .await
        .unwrap_err();
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

    let err = create_contact(
        &pool,
        &app,
        Uuid::new_v4(),
        &base_contact_req("Dan", "Brown"),
        corr(),
    )
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

    create_contact(
        &pool,
        &app,
        party_id,
        &base_contact_req("Eve", "Adams"),
        corr(),
    )
    .await
    .unwrap();
    create_contact(
        &pool,
        &app,
        party_id,
        &base_contact_req("Frank", "Baker"),
        corr(),
    )
    .await
    .unwrap();

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

    let created = create_contact(
        &pool,
        &app,
        party_id,
        &base_contact_req("Grace", "Hall"),
        corr(),
    )
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
// Get contact — wrong app_id returns None (tenant isolation)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_get_contact_wrong_app() {
    let pool = setup_db().await;
    let app = unique_app();
    let other_app = unique_app();
    let party_id = make_party(&pool, &app, "Wrong App Corp").await;

    let created = create_contact(
        &pool,
        &app,
        party_id,
        &base_contact_req("Hank", "Irwin"),
        corr(),
    )
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

    let created = create_contact(
        &pool,
        &app,
        party_id,
        &base_contact_req("Ivan", "Jones"),
        corr(),
    )
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
        corr(),
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
        corr(),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, PartyError::NotFound(_)),
        "expected NotFound, got: {:?}",
        err
    );
}

// ============================================================================
// Deactivate contact (soft-delete)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_deactivate_contact() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Deactivate Contact Corp").await;

    let contact = create_contact(
        &pool,
        &app,
        party_id,
        &base_contact_req("Jack", "King"),
        corr(),
    )
    .await
    .unwrap();

    deactivate_contact(&pool, &app, contact.id, corr())
        .await
        .unwrap();

    // Active queries should not find it
    let result = get_contact(&pool, &app, contact.id).await.unwrap();
    assert!(
        result.is_none(),
        "contact should not be visible after deactivation"
    );

    // But list should also exclude it
    let contacts = list_contacts(&pool, &app, party_id).await.unwrap();
    assert!(
        contacts.iter().all(|c| c.id != contact.id),
        "deactivated contact should not appear in list"
    );
}

// ============================================================================
// Deactivate contact — not found
// ============================================================================

#[tokio::test]
#[serial]
async fn test_deactivate_contact_not_found() {
    let pool = setup_db().await;
    let app = unique_app();

    let err = deactivate_contact(&pool, &app, Uuid::new_v4(), corr())
        .await
        .unwrap_err();
    assert!(
        matches!(err, PartyError::NotFound(_)),
        "expected NotFound, got: {:?}",
        err
    );
}

// ============================================================================
// Primary contact — set-primary per role enforces uniqueness
// ============================================================================

#[tokio::test]
#[serial]
async fn test_set_primary_per_role() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Primary Role Corp").await;

    let c1 = create_contact(
        &pool,
        &app,
        party_id,
        &base_contact_req("Lara", "Moon"),
        corr(),
    )
    .await
    .unwrap();

    let c2 = create_contact(
        &pool,
        &app,
        party_id,
        &base_contact_req("Mike", "Nash"),
        corr(),
    )
    .await
    .unwrap();

    // Set c1 as primary for "sales"
    let updated_c1 = set_primary_for_role(&pool, &app, party_id, c1.id, "sales", corr())
        .await
        .unwrap();
    assert!(updated_c1.is_primary);
    assert_eq!(updated_c1.role.as_deref(), Some("sales"));

    // Set c2 as primary for "sales" — should clear c1
    let updated_c2 = set_primary_for_role(&pool, &app, party_id, c2.id, "sales", corr())
        .await
        .unwrap();
    assert!(updated_c2.is_primary);

    let c1_refreshed = get_contact(&pool, &app, c1.id).await.unwrap().unwrap();
    assert!(
        !c1_refreshed.is_primary,
        "c1 should no longer be primary for sales"
    );

    // Set c1 as primary for "ap" — different role, should not affect c2
    set_primary_for_role(&pool, &app, party_id, c1.id, "ap", corr())
        .await
        .unwrap();

    let c2_still = get_contact(&pool, &app, c2.id).await.unwrap().unwrap();
    assert!(c2_still.is_primary, "c2 should still be primary for sales");
}

// ============================================================================
// Primary contacts query — role→contact map
// ============================================================================

#[tokio::test]
#[serial]
async fn test_get_primary_contacts() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Primary Contacts Map Corp").await;

    let c1 = create_contact(
        &pool,
        &app,
        party_id,
        &base_contact_req("Nora", "Owen"),
        corr(),
    )
    .await
    .unwrap();

    let c2 = create_contact(
        &pool,
        &app,
        party_id,
        &base_contact_req("Pete", "Quinn"),
        corr(),
    )
    .await
    .unwrap();

    set_primary_for_role(&pool, &app, party_id, c1.id, "sales", corr())
        .await
        .unwrap();
    set_primary_for_role(&pool, &app, party_id, c2.id, "ap", corr())
        .await
        .unwrap();

    let entries = get_primary_contacts(&pool, &app, party_id)
        .await
        .unwrap();

    assert_eq!(entries.len(), 2, "expected 2 primary contacts");

    let sales = entries.iter().find(|e| e.role == "sales");
    assert!(sales.is_some(), "should have a sales primary");
    assert_eq!(sales.unwrap().contact.id, c1.id);

    let ap = entries.iter().find(|e| e.role == "ap");
    assert!(ap.is_some(), "should have an ap primary");
    assert_eq!(ap.unwrap().contact.id, c2.id);
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
    let contact = create_contact(
        &pool,
        &app_a,
        party_id,
        &base_contact_req("Nora", "Owen"),
        corr(),
    )
    .await
    .unwrap();

    // App B cannot read App A's contact
    let result = get_contact(&pool, &app_b, contact.id).await.unwrap();
    assert!(result.is_none(), "app_b must not see app_a's contact");

    // App B deactivate attempt fails
    let err = deactivate_contact(&pool, &app_b, contact.id, corr())
        .await
        .unwrap_err();
    assert!(matches!(err, PartyError::NotFound(_)));
}

// ============================================================================
// Outbox events for contact operations
// ============================================================================

#[tokio::test]
#[serial]
async fn test_contact_events_in_outbox() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Events Corp").await;

    let contact = create_contact(
        &pool,
        &app,
        party_id,
        &base_contact_req("Sam", "Taylor"),
        corr(),
    )
    .await
    .unwrap();

    // Check create event
    let create_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM party_outbox WHERE aggregate_type = 'contact' AND aggregate_id = $1 AND event_type = 'party.events.contact.created'",
    )
    .bind(contact.id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(create_count.0, 1, "expected 1 contact.created event");

    // Update and check
    update_contact(
        &pool,
        &app,
        contact.id,
        &UpdateContactRequest {
            first_name: Some("Samuel".to_string()),
            last_name: None,
            email: None,
            phone: None,
            role: None,
            is_primary: None,
            metadata: None,
        },
        corr(),
    )
    .await
    .unwrap();

    let update_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM party_outbox WHERE aggregate_type = 'contact' AND aggregate_id = $1 AND event_type = 'party.events.contact.updated'",
    )
    .bind(contact.id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(update_count.0, 1, "expected 1 contact.updated event");

    // Deactivate and check
    deactivate_contact(&pool, &app, contact.id, corr())
        .await
        .unwrap();

    let deactivate_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM party_outbox WHERE aggregate_type = 'contact' AND aggregate_id = $1 AND event_type = 'party.events.contact.deactivated'",
    )
    .bind(contact.id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        deactivate_count.0, 1,
        "expected 1 contact.deactivated event"
    );
}

// ============================================================================
// Set-primary emits outbox event
// ============================================================================

#[tokio::test]
#[serial]
async fn test_set_primary_emits_event() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Primary Event Corp").await;

    let contact = create_contact(
        &pool,
        &app,
        party_id,
        &base_contact_req("Uma", "Vance"),
        corr(),
    )
    .await
    .unwrap();

    set_primary_for_role(&pool, &app, party_id, contact.id, "quality", corr())
        .await
        .unwrap();

    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM party_outbox WHERE aggregate_type = 'contact' AND aggregate_id = $1 AND event_type = 'party.events.contact.primary_set'",
    )
    .bind(contact.id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count.0, 1, "expected 1 contact.primary_set event");
}
