//! Integration tests for vendor qualification, credit terms, contact roles,
//! and scorecards.
//!
//! Required test categories (all 7):
//! 1. Vendor qualification E2E
//! 2. Credit terms E2E
//! 3. Contact roles E2E
//! 4. Scorecard E2E
//! 5. Tenant isolation across all 4 entity types
//! 6. Idempotency (duplicate idempotency_key)
//! 7. Outbox events for each operation

use chrono::{NaiveDate, Utc};
use party_rs::domain::contact::{CreateContactRequest};
use party_rs::domain::contact_role::{CreateContactRoleRequest};
use party_rs::domain::contact_role_service::{
    create_contact_role, get_contact_role, list_contact_roles,
};
use party_rs::domain::contact_service::create_contact;
use party_rs::domain::credit_terms::{CreateCreditTermsRequest, UpdateCreditTermsRequest};
use party_rs::domain::credit_terms_service::{
    create_credit_terms, list_credit_terms, update_credit_terms,
};
use party_rs::domain::party::service::create_company;
use party_rs::domain::party::CreateCompanyRequest;
use party_rs::domain::scorecard::{CreateScorecardRequest};
use party_rs::domain::scorecard_service::{create_scorecard, list_scorecards};
use party_rs::domain::vendor_qualification::{
    CreateVendorQualificationRequest, UpdateVendorQualificationRequest,
};
use party_rs::domain::vendor_qualification_service::{
    create_vendor_qualification, get_vendor_qualification, list_vendor_qualifications,
    update_vendor_qualification,
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
    format!("vendor-test-{}", Uuid::new_v4().simple())
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

async fn make_contact(pool: &sqlx::PgPool, app: &str, party_id: Uuid) -> Uuid {
    let req = CreateContactRequest {
        first_name: "Test".to_string(),
        last_name: "Contact".to_string(),
        email: Some("test.contact@example.com".to_string()),
        phone: None,
        role: Some("Engineer".to_string()),
        is_primary: Some(false),
        metadata: None,
    };
    let contact = create_contact(pool, app, party_id, &req, corr())
        .await
        .expect("make_contact failed");
    contact.id
}

// ============================================================================
// 1. Vendor Qualification E2E
// ============================================================================

#[tokio::test]
#[serial]
async fn test_vendor_qualification_e2e() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Qual Vendor Inc").await;

    let expires = Utc::now() + chrono::Duration::days(365);

    // Create qualification
    let qual = create_vendor_qualification(
        &pool,
        &app,
        party_id,
        &CreateVendorQualificationRequest {
            qualification_status: "approved".to_string(),
            certification_ref: Some("AS9100-REV-D".to_string()),
            issued_at: Some(Utc::now()),
            expires_at: Some(expires),
            notes: Some("Annual audit passed".to_string()),
            idempotency_key: None,
            metadata: None,
        },
        corr(),
    )
    .await
    .expect("create_vendor_qualification failed");

    assert_eq!(qual.qualification_status, "approved");
    assert_eq!(qual.certification_ref.as_deref(), Some("AS9100-REV-D"));
    assert!(qual.expires_at.is_some());
    assert_eq!(qual.party_id, party_id);

    // Query back
    let fetched = get_vendor_qualification(&pool, &app, qual.id)
        .await
        .expect("get failed")
        .expect("not found");
    assert_eq!(fetched.id, qual.id);
    assert_eq!(fetched.certification_ref.as_deref(), Some("AS9100-REV-D"));

    // List
    let list = list_vendor_qualifications(&pool, &app, party_id)
        .await
        .expect("list failed");
    assert_eq!(list.len(), 1);

    // Update
    let updated = update_vendor_qualification(
        &pool,
        &app,
        qual.id,
        &UpdateVendorQualificationRequest {
            qualification_status: Some("expired".to_string()),
            certification_ref: None,
            issued_at: None,
            expires_at: None,
            notes: None,
            metadata: None,
        },
        corr(),
    )
    .await
    .expect("update failed");
    assert_eq!(updated.qualification_status, "expired");
    // certification_ref preserved from original
    assert_eq!(updated.certification_ref.as_deref(), Some("AS9100-REV-D"));
}

// ============================================================================
// 2. Credit Terms E2E
// ============================================================================

#[tokio::test]
#[serial]
async fn test_credit_terms_e2e() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Credit Terms Corp").await;

    // Create initial terms
    let terms = create_credit_terms(
        &pool,
        &app,
        party_id,
        &CreateCreditTermsRequest {
            payment_terms: "Net 30".to_string(),
            credit_limit_cents: Some(500_000_00),
            currency: Some("USD".to_string()),
            effective_from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            effective_to: Some(NaiveDate::from_ymd_opt(2026, 12, 31).unwrap()),
            notes: None,
            idempotency_key: None,
            metadata: None,
        },
        corr(),
    )
    .await
    .expect("create_credit_terms failed");

    assert_eq!(terms.payment_terms, "Net 30");
    assert_eq!(terms.credit_limit_cents, Some(500_000_00));

    // Update terms
    let updated = update_credit_terms(
        &pool,
        &app,
        terms.id,
        &UpdateCreditTermsRequest {
            payment_terms: Some("Net 60".to_string()),
            credit_limit_cents: Some(1_000_000_00),
            currency: None,
            effective_from: None,
            effective_to: None,
            notes: Some("Increased after annual review".to_string()),
            metadata: None,
        },
        corr(),
    )
    .await
    .expect("update_credit_terms failed");

    assert_eq!(updated.payment_terms, "Net 60");
    assert_eq!(updated.credit_limit_cents, Some(1_000_000_00));

    // Verify history: add a second terms record
    let terms2 = create_credit_terms(
        &pool,
        &app,
        party_id,
        &CreateCreditTermsRequest {
            payment_terms: "Net 90".to_string(),
            credit_limit_cents: Some(2_000_000_00),
            currency: Some("USD".to_string()),
            effective_from: NaiveDate::from_ymd_opt(2027, 1, 1).unwrap(),
            effective_to: None,
            notes: None,
            idempotency_key: None,
            metadata: None,
        },
        corr(),
    )
    .await
    .expect("create second credit_terms failed");

    let history = list_credit_terms(&pool, &app, party_id)
        .await
        .expect("list failed");
    assert_eq!(history.len(), 2, "expected 2 credit terms records");

    // Most recent first (2027 before 2026)
    assert_eq!(history[0].id, terms2.id);
}

// ============================================================================
// 3. Contact Roles E2E
// ============================================================================

#[tokio::test]
#[serial]
async fn test_contact_roles_e2e() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Contact Roles Corp").await;
    let contact_id = make_contact(&pool, &app, party_id).await;

    // Create billing role as primary
    let role1 = create_contact_role(
        &pool,
        &app,
        party_id,
        &CreateContactRoleRequest {
            contact_id,
            role_type: "billing".to_string(),
            is_primary: Some(true),
            effective_from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            effective_to: None,
            idempotency_key: None,
            metadata: None,
        },
        corr(),
    )
    .await
    .expect("create billing role failed");

    assert_eq!(role1.role_type, "billing");
    assert!(role1.is_primary);

    // Create a second contact and assign same role type as primary
    let contact2_req = CreateContactRequest {
        first_name: "Other".to_string(),
        last_name: "Person".to_string(),
        email: Some("other@example.com".to_string()),
        phone: None,
        role: None,
        is_primary: Some(false),
        metadata: None,
    };
    let contact2 = create_contact(&pool, &app, party_id, &contact2_req, corr())
        .await
        .expect("create contact2 failed");

    let role2 = create_contact_role(
        &pool,
        &app,
        party_id,
        &CreateContactRoleRequest {
            contact_id: contact2.id,
            role_type: "billing".to_string(),
            is_primary: Some(true),
            effective_from: NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
            effective_to: None,
            idempotency_key: None,
            metadata: None,
        },
        corr(),
    )
    .await
    .expect("create second billing role failed");

    assert!(role2.is_primary);

    // Verify primary flag uniqueness per role type — first should be demoted
    let refreshed = get_contact_role(&pool, &app, role1.id)
        .await
        .expect("get failed")
        .expect("not found");
    assert!(
        !refreshed.is_primary,
        "original primary should be cleared when new primary assigned"
    );

    // List all roles
    let roles = list_contact_roles(&pool, &app, party_id)
        .await
        .expect("list failed");
    assert_eq!(roles.len(), 2);
}

// ============================================================================
// 4. Scorecard E2E
// ============================================================================

#[tokio::test]
#[serial]
async fn test_scorecard_e2e() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Scorecard Vendor Inc").await;

    // Create scorecard entries
    let sc = create_scorecard(
        &pool,
        &app,
        party_id,
        &CreateScorecardRequest {
            metric_name: "Quality Rating".to_string(),
            score: 92.5,
            max_score: Some(100.0),
            review_date: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            reviewer: Some("QA Manager".to_string()),
            notes: Some("Excellent quality metrics".to_string()),
            idempotency_key: None,
            metadata: None,
        },
        corr(),
    )
    .await
    .expect("create_scorecard failed");

    assert_eq!(sc.metric_name, "Quality Rating");
    assert_eq!(sc.reviewer.as_deref(), Some("QA Manager"));

    // Create a second metric
    create_scorecard(
        &pool,
        &app,
        party_id,
        &CreateScorecardRequest {
            metric_name: "Delivery Timeliness".to_string(),
            score: 88.0,
            max_score: Some(100.0),
            review_date: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            reviewer: Some("Logistics Lead".to_string()),
            notes: None,
            idempotency_key: None,
            metadata: None,
        },
        corr(),
    )
    .await
    .expect("create second scorecard failed");

    // List and verify
    let cards = list_scorecards(&pool, &app, party_id)
        .await
        .expect("list failed");
    assert_eq!(cards.len(), 2);

    // Verify persistence via get
    let fetched = party_rs::domain::scorecard_service::get_scorecard(&pool, &app, sc.id)
        .await
        .expect("get failed")
        .expect("not found");
    assert_eq!(fetched.metric_name, "Quality Rating");
}

// ============================================================================
// 5. Tenant Isolation — all 4 entity types
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_isolation_all_entities() {
    let pool = setup_db().await;
    let app_a = unique_app();
    let app_b = unique_app();

    // Create data under tenant A
    let party_a = make_party(&pool, &app_a, "Tenant A Vendor").await;
    let contact_a = make_contact(&pool, &app_a, party_a).await;

    // Vendor qualification
    let qual = create_vendor_qualification(
        &pool,
        &app_a,
        party_a,
        &CreateVendorQualificationRequest {
            qualification_status: "approved".to_string(),
            certification_ref: Some("CERT-A".to_string()),
            issued_at: None,
            expires_at: None,
            notes: None,
            idempotency_key: None,
            metadata: None,
        },
        corr(),
    )
    .await
    .unwrap();

    // Credit terms
    let ct = create_credit_terms(
        &pool,
        &app_a,
        party_a,
        &CreateCreditTermsRequest {
            payment_terms: "Net 30".to_string(),
            credit_limit_cents: Some(100_000),
            currency: None,
            effective_from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            effective_to: None,
            notes: None,
            idempotency_key: None,
            metadata: None,
        },
        corr(),
    )
    .await
    .unwrap();

    // Contact role
    let role = create_contact_role(
        &pool,
        &app_a,
        party_a,
        &CreateContactRoleRequest {
            contact_id: contact_a,
            role_type: "billing".to_string(),
            is_primary: Some(true),
            effective_from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            effective_to: None,
            idempotency_key: None,
            metadata: None,
        },
        corr(),
    )
    .await
    .unwrap();

    // Scorecard
    let sc = create_scorecard(
        &pool,
        &app_a,
        party_a,
        &CreateScorecardRequest {
            metric_name: "Quality".to_string(),
            score: 95.0,
            max_score: None,
            review_date: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            reviewer: None,
            notes: None,
            idempotency_key: None,
            metadata: None,
        },
        corr(),
    )
    .await
    .unwrap();

    // Tenant B cannot see any of these by ID
    let qual_b = get_vendor_qualification(&pool, &app_b, qual.id)
        .await
        .unwrap();
    assert!(
        qual_b.is_none(),
        "tenant B must not see tenant A's qualification"
    );

    let ct_b = party_rs::domain::credit_terms_service::get_credit_terms(&pool, &app_b, ct.id)
        .await
        .unwrap();
    assert!(
        ct_b.is_none(),
        "tenant B must not see tenant A's credit terms"
    );

    let role_b = get_contact_role(&pool, &app_b, role.id).await.unwrap();
    assert!(
        role_b.is_none(),
        "tenant B must not see tenant A's contact role"
    );

    let sc_b = party_rs::domain::scorecard_service::get_scorecard(&pool, &app_b, sc.id)
        .await
        .unwrap();
    assert!(
        sc_b.is_none(),
        "tenant B must not see tenant A's scorecard"
    );

    // Tenant B lists are empty (using a different party which doesn't exist for B)
    let quals_b = list_vendor_qualifications(&pool, &app_b, party_a)
        .await
        .unwrap();
    assert!(quals_b.is_empty(), "tenant B qualification list must be empty");

    let cts_b = list_credit_terms(&pool, &app_b, party_a).await.unwrap();
    assert!(cts_b.is_empty(), "tenant B credit terms list must be empty");

    let roles_b = list_contact_roles(&pool, &app_b, party_a).await.unwrap();
    assert!(roles_b.is_empty(), "tenant B contact roles list must be empty");

    let scs_b = list_scorecards(&pool, &app_b, party_a).await.unwrap();
    assert!(scs_b.is_empty(), "tenant B scorecards list must be empty");
}

// ============================================================================
// 6. Idempotency — duplicate idempotency_key
// ============================================================================

#[tokio::test]
#[serial]
async fn test_idempotency_no_duplicate() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Idempotent Vendor").await;

    let idem_key = format!("idem-{}", Uuid::new_v4());

    // First submission
    let qual1 = create_vendor_qualification(
        &pool,
        &app,
        party_id,
        &CreateVendorQualificationRequest {
            qualification_status: "pending".to_string(),
            certification_ref: Some("CERT-IDEM".to_string()),
            issued_at: None,
            expires_at: None,
            notes: None,
            idempotency_key: Some(idem_key.clone()),
            metadata: None,
        },
        corr(),
    )
    .await
    .expect("first submission failed");

    // Second submission with same idempotency_key
    let qual2 = create_vendor_qualification(
        &pool,
        &app,
        party_id,
        &CreateVendorQualificationRequest {
            qualification_status: "pending".to_string(),
            certification_ref: Some("CERT-IDEM".to_string()),
            issued_at: None,
            expires_at: None,
            notes: None,
            idempotency_key: Some(idem_key.clone()),
            metadata: None,
        },
        corr(),
    )
    .await
    .expect("second submission failed");

    // Same record returned — no duplicate created
    assert_eq!(qual1.id, qual2.id, "idempotency_key must return same record");

    // Verify only one record exists
    let list = list_vendor_qualifications(&pool, &app, party_id)
        .await
        .expect("list failed");
    assert_eq!(
        list.len(),
        1,
        "expected exactly 1 qualification, got {}",
        list.len()
    );
}

// ============================================================================
// 7. Outbox Events — verify outbox event after each operation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_outbox_events() {
    let pool = setup_db().await;
    let app = unique_app();
    let party_id = make_party(&pool, &app, "Outbox Vendor Inc").await;
    let contact_id = make_contact(&pool, &app, party_id).await;

    // Vendor qualification → outbox event
    let qual = create_vendor_qualification(
        &pool,
        &app,
        party_id,
        &CreateVendorQualificationRequest {
            qualification_status: "approved".to_string(),
            certification_ref: None,
            issued_at: None,
            expires_at: None,
            notes: None,
            idempotency_key: None,
            metadata: None,
        },
        corr(),
    )
    .await
    .unwrap();

    let qual_events: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM party_outbox WHERE aggregate_type = 'vendor_qualification' AND aggregate_id = $1 AND app_id = $2",
    )
    .bind(qual.id.to_string())
    .bind(&app)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        qual_events.0 >= 1,
        "expected outbox event for vendor_qualification"
    );

    // Credit terms → outbox event
    let ct = create_credit_terms(
        &pool,
        &app,
        party_id,
        &CreateCreditTermsRequest {
            payment_terms: "Net 30".to_string(),
            credit_limit_cents: None,
            currency: None,
            effective_from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            effective_to: None,
            notes: None,
            idempotency_key: None,
            metadata: None,
        },
        corr(),
    )
    .await
    .unwrap();

    let ct_events: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM party_outbox WHERE aggregate_type = 'credit_terms' AND aggregate_id = $1 AND app_id = $2",
    )
    .bind(ct.id.to_string())
    .bind(&app)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        ct_events.0 >= 1,
        "expected outbox event for credit_terms"
    );

    // Contact role → outbox event
    let role = create_contact_role(
        &pool,
        &app,
        party_id,
        &CreateContactRoleRequest {
            contact_id,
            role_type: "technical".to_string(),
            is_primary: Some(false),
            effective_from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            effective_to: None,
            idempotency_key: None,
            metadata: None,
        },
        corr(),
    )
    .await
    .unwrap();

    let role_events: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM party_outbox WHERE aggregate_type = 'contact_role' AND aggregate_id = $1 AND app_id = $2",
    )
    .bind(role.id.to_string())
    .bind(&app)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        role_events.0 >= 1,
        "expected outbox event for contact_role"
    );

    // Scorecard → outbox event
    let sc = create_scorecard(
        &pool,
        &app,
        party_id,
        &CreateScorecardRequest {
            metric_name: "OTD".to_string(),
            score: 90.0,
            max_score: None,
            review_date: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            reviewer: None,
            notes: None,
            idempotency_key: None,
            metadata: None,
        },
        corr(),
    )
    .await
    .unwrap();

    let sc_events: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM party_outbox WHERE aggregate_type = 'scorecard' AND aggregate_id = $1 AND app_id = $2",
    )
    .bind(sc.id.to_string())
    .bind(&app)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        sc_events.0 >= 1,
        "expected outbox event for scorecard"
    );

    // Verify correct event_type in outbox
    let qual_type: (String,) = sqlx::query_as(
        "SELECT event_type FROM party_outbox WHERE aggregate_type = 'vendor_qualification' AND aggregate_id = $1 AND app_id = $2 LIMIT 1",
    )
    .bind(qual.id.to_string())
    .bind(&app)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(qual_type.0, "party.vendor_qualification.created");

    let ct_type: (String,) = sqlx::query_as(
        "SELECT event_type FROM party_outbox WHERE aggregate_type = 'credit_terms' AND aggregate_id = $1 AND app_id = $2 LIMIT 1",
    )
    .bind(ct.id.to_string())
    .bind(&app)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ct_type.0, "party.credit_terms.created");

    let role_type_row: (String,) = sqlx::query_as(
        "SELECT event_type FROM party_outbox WHERE aggregate_type = 'contact_role' AND aggregate_id = $1 AND app_id = $2 LIMIT 1",
    )
    .bind(role.id.to_string())
    .bind(&app)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(role_type_row.0, "party.contact_role.created");

    let sc_type: (String,) = sqlx::query_as(
        "SELECT event_type FROM party_outbox WHERE aggregate_type = 'scorecard' AND aggregate_id = $1 AND app_id = $2 LIMIT 1",
    )
    .bind(sc.id.to_string())
    .bind(&app)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(sc_type.0, "party.scorecard.created");
}
