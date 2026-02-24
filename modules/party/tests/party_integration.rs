//! Integration tests for party CRUD — companies, individuals, list, search,
//! get, update, deactivate, and tenant isolation.
//!
//! Connects to postgresql://party_user:party_pass@localhost:5448/party_db
//! (overridable via DATABASE_URL).

use party_rs::domain::party::service::{
    create_company, create_individual, deactivate_party, get_party, list_parties, search_parties,
    update_party,
};
use party_rs::domain::party::{
    CreateCompanyRequest, CreateIndividualRequest, PartyError, SearchQuery, UpdatePartyRequest,
};
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
    format!("party-test-{}", Uuid::new_v4().simple())
}

fn corr() -> String {
    Uuid::new_v4().to_string()
}

fn base_company_req(name: &str) -> CreateCompanyRequest {
    CreateCompanyRequest {
        display_name: name.to_string(),
        legal_name: format!("{} LLC", name),
        trade_name: None,
        registration_number: None,
        tax_id: None,
        country_of_incorporation: Some("US".to_string()),
        industry_code: None,
        founded_date: None,
        employee_count: None,
        annual_revenue_cents: None,
        currency: None,
        email: Some(format!("{}@example.com", name.to_lowercase().replace(' ', "."))),
        phone: None,
        website: None,
        address_line1: None,
        address_line2: None,
        city: None,
        state: None,
        postal_code: None,
        country: Some("US".to_string()),
        metadata: None,
    }
}

fn base_individual_req(first: &str, last: &str) -> CreateIndividualRequest {
    CreateIndividualRequest {
        display_name: format!("{} {}", first, last),
        first_name: first.to_string(),
        last_name: last.to_string(),
        middle_name: None,
        date_of_birth: None,
        tax_id: None,
        nationality: Some("US".to_string()),
        job_title: Some("Engineer".to_string()),
        department: None,
        email: Some(format!("{}.{}@example.com", first.to_lowercase(), last.to_lowercase())),
        phone: None,
        address_line1: None,
        address_line2: None,
        city: None,
        state: None,
        postal_code: None,
        country: None,
        metadata: None,
    }
}

// ============================================================================
// Company happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_company_happy_path() {
    let pool = setup_db().await;
    let app = unique_app();

    let view = create_company(&pool, &app, &base_company_req("Acme Corp"), corr())
        .await
        .expect("create_company failed");

    assert_eq!(view.party.display_name, "Acme Corp");
    assert_eq!(view.party.party_type, "company");
    assert_eq!(view.party.status, "active");
    assert_eq!(view.party.app_id, app);
    let co = view.company.expect("company extension missing");
    assert_eq!(co.legal_name, "Acme Corp LLC");
}

// ============================================================================
// Company validation error — empty display_name
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_company_empty_display_name() {
    let pool = setup_db().await;
    let app = unique_app();

    let mut req = base_company_req("Ignored");
    req.display_name = "  ".to_string();

    let err = create_company(&pool, &app, &req, corr()).await.unwrap_err();
    assert!(
        matches!(err, PartyError::Validation(_)),
        "expected Validation, got: {:?}",
        err
    );
}

// ============================================================================
// Individual happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_individual_happy_path() {
    let pool = setup_db().await;
    let app = unique_app();

    let view = create_individual(&pool, &app, &base_individual_req("Alice", "Smith"), corr())
        .await
        .expect("create_individual failed");

    assert_eq!(view.party.party_type, "individual");
    assert_eq!(view.party.status, "active");
    let ind = view.individual.expect("individual extension missing");
    assert_eq!(ind.first_name, "Alice");
    assert_eq!(ind.last_name, "Smith");
}

// ============================================================================
// Individual validation error — empty first_name
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_individual_empty_first_name() {
    let pool = setup_db().await;
    let app = unique_app();

    let mut req = base_individual_req("Bob", "Jones");
    req.first_name = "".to_string();

    let err = create_individual(&pool, &app, &req, corr()).await.unwrap_err();
    assert!(
        matches!(err, PartyError::Validation(_)),
        "expected Validation, got: {:?}",
        err
    );
}

// ============================================================================
// Get party by ID
// ============================================================================

#[tokio::test]
#[serial]
async fn test_get_party_by_id() {
    let pool = setup_db().await;
    let app = unique_app();

    let created = create_company(&pool, &app, &base_company_req("Beta Corp"), corr())
        .await
        .unwrap();

    let fetched = get_party(&pool, &app, created.party.id)
        .await
        .unwrap()
        .expect("party not found");

    assert_eq!(fetched.party.id, created.party.id);
    assert_eq!(fetched.party.display_name, "Beta Corp");
}

// ============================================================================
// Get party not found — returns None
// ============================================================================

#[tokio::test]
#[serial]
async fn test_get_party_not_found() {
    let pool = setup_db().await;
    let app = unique_app();

    let result = get_party(&pool, &app, Uuid::new_v4()).await.unwrap();
    assert!(result.is_none());
}

// ============================================================================
// List parties — active only
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_parties_active_only() {
    let pool = setup_db().await;
    let app = unique_app();

    let a = create_company(&pool, &app, &base_company_req("Active One"), corr()).await.unwrap();
    create_company(&pool, &app, &base_company_req("Active Two"), corr()).await.unwrap();
    // Deactivate first
    deactivate_party(&pool, &app, a.party.id, "test", corr()).await.unwrap();

    let active = list_parties(&pool, &app, false).await.unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].display_name, "Active Two");
}

// ============================================================================
// List parties — include inactive
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_parties_include_inactive() {
    let pool = setup_db().await;
    let app = unique_app();

    let a = create_company(&pool, &app, &base_company_req("Firm A"), corr()).await.unwrap();
    create_company(&pool, &app, &base_company_req("Firm B"), corr()).await.unwrap();
    deactivate_party(&pool, &app, a.party.id, "test", corr()).await.unwrap();

    let all = list_parties(&pool, &app, true).await.unwrap();
    assert_eq!(all.len(), 2);

    let inactive = all.iter().find(|p| p.id == a.party.id).unwrap();
    assert_eq!(inactive.status, "inactive");
}

// ============================================================================
// Update party
// ============================================================================

#[tokio::test]
#[serial]
async fn test_update_party() {
    let pool = setup_db().await;
    let app = unique_app();

    let created = create_company(&pool, &app, &base_company_req("Old Name"), corr()).await.unwrap();

    let updated = update_party(
        &pool,
        &app,
        created.party.id,
        &UpdatePartyRequest {
            display_name: Some("New Name".to_string()),
            email: Some("new@example.com".to_string()),
            phone: None,
            website: None,
            address_line1: None,
            address_line2: None,
            city: None,
            state: None,
            postal_code: None,
            country: None,
            metadata: None,
            updated_by: Some("test-agent".to_string()),
        },
        corr(),
    )
    .await
    .unwrap();

    assert_eq!(updated.party.display_name, "New Name");
    assert_eq!(updated.party.email.as_deref(), Some("new@example.com"));
}

// ============================================================================
// Update party not found — returns NotFound error
// ============================================================================

#[tokio::test]
#[serial]
async fn test_update_party_not_found() {
    let pool = setup_db().await;
    let app = unique_app();

    let err = update_party(
        &pool,
        &app,
        Uuid::new_v4(),
        &UpdatePartyRequest {
            display_name: Some("Ghost".to_string()),
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
            updated_by: None,
        },
        corr(),
    )
    .await
    .unwrap_err();

    assert!(matches!(err, PartyError::NotFound(_)), "expected NotFound, got: {:?}", err);
}

// ============================================================================
// Deactivate party
// ============================================================================

#[tokio::test]
#[serial]
async fn test_deactivate_party() {
    let pool = setup_db().await;
    let app = unique_app();

    let created = create_company(&pool, &app, &base_company_req("Gamma Inc"), corr()).await.unwrap();
    deactivate_party(&pool, &app, created.party.id, "admin", corr()).await.unwrap();

    let view = get_party(&pool, &app, created.party.id).await.unwrap().unwrap();
    assert_eq!(view.party.status, "inactive");
}

// ============================================================================
// Deactivate party not found
// ============================================================================

#[tokio::test]
#[serial]
async fn test_deactivate_party_not_found() {
    let pool = setup_db().await;
    let app = unique_app();

    let err = deactivate_party(&pool, &app, Uuid::new_v4(), "admin", corr())
        .await
        .unwrap_err();

    assert!(matches!(err, PartyError::NotFound(_)), "expected NotFound, got: {:?}", err);
}

// ============================================================================
// Search parties by name
// ============================================================================

#[tokio::test]
#[serial]
async fn test_search_parties_by_name() {
    let pool = setup_db().await;
    let app = unique_app();

    create_company(&pool, &app, &base_company_req("Delta Supplies"), corr()).await.unwrap();
    create_company(&pool, &app, &base_company_req("Delta Analytics"), corr()).await.unwrap();
    create_company(&pool, &app, &base_company_req("Epsilon LLC"), corr()).await.unwrap();

    let results = search_parties(
        &pool,
        &app,
        &SearchQuery {
            name: Some("Delta".to_string()),
            party_type: None,
            status: None,
            external_system: None,
            external_id: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(results.len(), 2, "expected 2 Delta parties, got {}", results.len());
    assert!(results.iter().all(|p| p.display_name.contains("Delta")));
}

// ============================================================================
// Search parties by party_type
// ============================================================================

#[tokio::test]
#[serial]
async fn test_search_parties_by_type() {
    let pool = setup_db().await;
    let app = unique_app();

    create_company(&pool, &app, &base_company_req("Theta Corp"), corr()).await.unwrap();
    create_individual(&pool, &app, &base_individual_req("Iota", "Person"), corr()).await.unwrap();

    let companies = search_parties(
        &pool,
        &app,
        &SearchQuery {
            name: None,
            party_type: Some("company".to_string()),
            status: None,
            external_system: None,
            external_id: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(companies.len(), 1);
    assert_eq!(companies[0].party_type, "company");

    let individuals = search_parties(
        &pool,
        &app,
        &SearchQuery {
            name: None,
            party_type: Some("individual".to_string()),
            status: None,
            external_system: None,
            external_id: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(individuals.len(), 1);
    assert_eq!(individuals[0].party_type, "individual");
}

// ============================================================================
// Tenant isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_isolation() {
    let pool = setup_db().await;
    let app_a = unique_app();
    let app_b = unique_app();

    let party = create_company(&pool, &app_a, &base_company_req("Tenant A Corp"), corr())
        .await
        .unwrap();

    // App B cannot see App A's party by ID
    let result = get_party(&pool, &app_b, party.party.id).await.unwrap();
    assert!(result.is_none(), "app_b must not see app_a's party");

    // App B list is empty
    let list = list_parties(&pool, &app_b, false).await.unwrap();
    assert!(list.is_empty(), "app_b list must be empty");

    // App B search returns nothing
    let search = search_parties(
        &pool,
        &app_b,
        &SearchQuery {
            name: Some("Tenant A".to_string()),
            party_type: None,
            status: None,
            external_system: None,
            external_id: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert!(search.is_empty(), "app_b search must be empty");

    // App B cannot deactivate App A's party
    let err = deactivate_party(&pool, &app_b, party.party.id, "attacker", corr())
        .await
        .unwrap_err();
    assert!(matches!(err, PartyError::NotFound(_)));
}
