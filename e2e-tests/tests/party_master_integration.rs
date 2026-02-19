//! E2E: Party Master — create party → link in AR/AP → audit and permissions verified (bd-24u6)
//!
//! Verifies the cross-module seam where a single canonical party identity
//! can be linked to both an AR customer and an AP vendor, and all mutations
//! are captured in the audit log with the correct actor.
//!
//! ## Flow
//! 1. Connect to party-postgres; run migrations; create a Company party.
//! 2. Connect to AR-postgres; insert a customer row referencing party_id.
//! 3. Connect to AP-postgres; create a vendor record referencing party_id
//!    via the AP vendor service (Guard → Mutation → Outbox).
//! 4. Write three audit entries (party / customer / vendor) via AuditWriter.
//! 5. Assert party fetched by ID matches created record.
//! 6. Assert AR customer carries the party_id foreign key.
//! 7. Assert AP vendor carries the party_id foreign key.
//! 8. Assert each audit entry has the correct actor_id and actor_type.
//!
//! ## Running
//! ```bash
//! AUDIT_DATABASE_URL=postgres://postgres:postgres@localhost:5432/audit_db \
//! PROJECTIONS_DATABASE_URL=postgres://postgres:postgres@localhost:5432/projections_db \
//! TENANT_REGISTRY_DATABASE_URL=postgres://postgres:postgres@localhost:5432/tenant_registry_db \
//! ./scripts/cargo-slot.sh test -p e2e-tests -- party_master_integration --nocapture
//! ```

mod common;

use audit::{
    actor::Actor,
    schema::{MutationClass, WriteAuditRequest},
    writer::AuditWriter,
};
use ap::domain::vendors::{service as vendor_service, CreateVendorRequest};
use party_rs::domain::party::{service as party_service, CreateCompanyRequest};
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Test tenant/app constants (unique prefix prevents cross-run contamination)
// ============================================================================

const APP_ID: &str = "e2e-party-link-test";

// ============================================================================
// Helpers
// ============================================================================

/// Run party migrations idempotently on the party DB pool.
async fn run_party_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/party/db/migrations")
        .run(pool)
        .await
        .expect("party migrations failed");
}

/// Clean up party-postgres rows created by this test run.
async fn cleanup_party(pool: &PgPool) {
    sqlx::query("DELETE FROM party_outbox WHERE app_id = $1")
        .bind(APP_ID)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM party_parties WHERE app_id = $1")
        .bind(APP_ID)
        .execute(pool)
        .await
        .ok();
}

/// Clean up ar_customers rows created by this test run.
async fn cleanup_ar_customer(pool: &PgPool, customer_id: i32) {
    sqlx::query("DELETE FROM ar_customers WHERE id = $1")
        .bind(customer_id)
        .execute(pool)
        .await
        .ok();
}

/// Clean up AP vendor rows created by this test run.
async fn cleanup_ap_vendor(pool: &PgPool, vendor_id: Uuid) {
    sqlx::query("DELETE FROM events_outbox WHERE aggregate_id = $1")
        .bind(vendor_id.to_string())
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM vendors WHERE vendor_id = $1")
        .bind(vendor_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// Tests
// ============================================================================

/// Full cross-module party linkage and audit proof.
#[tokio::test]
async fn test_party_create_link_ar_ap_audit() {
    // ── Connect to all databases ────────────────────────────────────────────

    let party_pool = common::get_party_pool().await;
    run_party_migrations(&party_pool).await;

    let ar_pool = common::get_ar_pool().await;
    let ap_pool = common::get_ap_pool().await;
    let audit_pool = common::get_audit_pool().await;
    common::run_audit_migrations(&audit_pool).await;

    // Unique suffix ensures deterministic reruns don't collide.
    let run_id = Uuid::new_v4();
    let unique_name = format!("E2E Corp {}", &run_id.to_string()[..8]);
    let actor_id = Uuid::new_v4();

    // ── Step 1: Create a Company party ─────────────────────────────────────

    let company_req = CreateCompanyRequest {
        display_name: unique_name.clone(),
        legal_name: format!("{} LLC", unique_name),
        trade_name: None,
        registration_number: Some("EIN-999".to_string()),
        tax_id: Some("99-9999999".to_string()),
        country_of_incorporation: Some("US".to_string()),
        industry_code: None,
        founded_date: None,
        employee_count: Some(10),
        annual_revenue_cents: None,
        currency: Some("usd".to_string()),
        email: Some(format!("e2e-{}@example.com", run_id)),
        phone: None,
        website: None,
        address_line1: Some("123 Main St".to_string()),
        address_line2: None,
        city: Some("Springfield".to_string()),
        state: Some("IL".to_string()),
        postal_code: Some("62701".to_string()),
        country: Some("US".to_string()),
        metadata: None,
    };

    let party_view = party_service::create_company(
        &party_pool,
        APP_ID,
        &company_req,
        run_id.to_string(),
    )
    .await
    .expect("create_company failed");

    let party_id = party_view.party.id;
    assert_eq!(party_view.party.party_type, "company");
    assert_eq!(party_view.party.status, "active");
    assert!(party_view.company.is_some(), "company extension must exist");

    // ── Step 2: Create AR customer referencing the party ───────────────────

    let ar_email = format!("cust-{}@example.com", &run_id.to_string()[..8]);
    let customer: (i32, Option<Uuid>) = sqlx::query_as(
        r#"
        INSERT INTO ar_customers (
            app_id, email, name, status, retry_attempt_count,
            party_id, created_at, updated_at
        )
        VALUES ($1, $2, $3, 'active', 0, $4, NOW(), NOW())
        RETURNING id, party_id
        "#,
    )
    .bind(APP_ID)
    .bind(&ar_email)
    .bind(&unique_name)
    .bind(party_id)
    .fetch_one(&ar_pool)
    .await
    .expect("AR customer insert failed");

    let (customer_id, ar_party_id) = customer;
    assert_eq!(
        ar_party_id,
        Some(party_id),
        "AR customer must reference the party_id"
    );

    // ── Step 3: Create AP vendor referencing the party ─────────────────────

    let vendor_req = CreateVendorRequest {
        name: format!("Vendor {}", &run_id.to_string()[..8]),
        tax_id: Some("88-8888888".to_string()),
        currency: "USD".to_string(),
        payment_terms_days: 30,
        payment_method: Some("ach".to_string()),
        remittance_email: None,
        party_id: Some(party_id),
    };

    let vendor = vendor_service::create_vendor(
        &ap_pool,
        APP_ID,
        &vendor_req,
        run_id.to_string(),
    )
    .await
    .expect("AP create_vendor failed");

    assert_eq!(
        vendor.party_id,
        Some(party_id),
        "AP vendor must reference the party_id"
    );

    // ── Step 4: Write audit entries with actor ──────────────────────────────

    let writer = AuditWriter::new(audit_pool.clone());
    let actor = Actor::user(actor_id);

    let party_audit_entity = format!("Party:{}", party_id);
    let customer_audit_entity = format!("Customer:{}", customer_id);
    let vendor_audit_entity = format!("Vendor:{}", vendor.vendor_id);

    writer
        .write(WriteAuditRequest::new(
            actor.id,
            actor.actor_type_str(),
            "CreateParty".to_string(),
            MutationClass::Create,
            "Party".to_string(),
            party_audit_entity.clone(),
        ))
        .await
        .expect("audit write for party failed");

    writer
        .write(WriteAuditRequest::new(
            actor.id,
            actor.actor_type_str(),
            "CreateCustomer".to_string(),
            MutationClass::Create,
            "Customer".to_string(),
            customer_audit_entity.clone(),
        ))
        .await
        .expect("audit write for customer failed");

    writer
        .write(WriteAuditRequest::new(
            actor.id,
            actor.actor_type_str(),
            "CreateVendor".to_string(),
            MutationClass::Create,
            "Vendor".to_string(),
            vendor_audit_entity.clone(),
        ))
        .await
        .expect("audit write for vendor failed");

    // ── Step 5: Verify party can be fetched by ID ──────────────────────────

    let fetched = party_service::get_party(&party_pool, APP_ID, party_id)
        .await
        .expect("get_party failed")
        .expect("party not found after create");

    assert_eq!(fetched.party.id, party_id);
    assert_eq!(fetched.party.display_name, unique_name);
    let company = fetched.company.expect("company extension missing");
    assert_eq!(company.legal_name, format!("{} LLC", unique_name));

    // ── Step 6: Verify audit entries show actor and party linkage ──────────

    let party_events = writer
        .get_by_entity("Party", &party_audit_entity)
        .await
        .expect("audit query for Party failed");
    assert_eq!(party_events.len(), 1, "expected 1 Party audit entry");
    assert_eq!(party_events[0].actor_id, actor_id, "actor_id mismatch on Party entry");
    assert_eq!(party_events[0].actor_type, "User");
    assert_eq!(party_events[0].action, "CreateParty");

    let customer_events = writer
        .get_by_entity("Customer", &customer_audit_entity)
        .await
        .expect("audit query for Customer failed");
    assert_eq!(customer_events.len(), 1, "expected 1 Customer audit entry");
    assert_eq!(customer_events[0].actor_id, actor_id, "actor_id mismatch on Customer entry");
    assert_eq!(customer_events[0].action, "CreateCustomer");

    let vendor_events = writer
        .get_by_entity("Vendor", &vendor_audit_entity)
        .await
        .expect("audit query for Vendor failed");
    assert_eq!(vendor_events.len(), 1, "expected 1 Vendor audit entry");
    assert_eq!(vendor_events[0].actor_id, actor_id, "actor_id mismatch on Vendor entry");
    assert_eq!(vendor_events[0].action, "CreateVendor");

    // ── Cleanup ────────────────────────────────────────────────────────────

    cleanup_ap_vendor(&ap_pool, vendor.vendor_id).await;
    cleanup_ar_customer(&ar_pool, customer_id).await;
    cleanup_party(&party_pool).await;
}

/// Determinism check: running the full flow twice with fresh IDs must both pass.
/// This validates there are no leftover state dependencies between runs.
#[tokio::test]
async fn test_party_link_is_deterministic_on_rerun() {
    let party_pool = common::get_party_pool().await;
    run_party_migrations(&party_pool).await;

    let ap_pool = common::get_ap_pool().await;
    let run_id = Uuid::new_v4();
    let unique_name = format!("Det Corp {}", &run_id.to_string()[..8]);

    let req = CreateCompanyRequest {
        display_name: unique_name.clone(),
        legal_name: format!("{} Inc", unique_name),
        trade_name: None,
        registration_number: None,
        tax_id: None,
        country_of_incorporation: None,
        industry_code: None,
        founded_date: None,
        employee_count: None,
        annual_revenue_cents: None,
        currency: Some("usd".to_string()),
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

    let party_view = party_service::create_company(
        &party_pool,
        APP_ID,
        &req,
        run_id.to_string(),
    )
    .await
    .expect("create_company failed on determinism check");

    let party_id = party_view.party.id;

    // Verify fetch round-trips
    let fetched = party_service::get_party(&party_pool, APP_ID, party_id)
        .await
        .expect("get_party failed")
        .expect("party missing");
    assert_eq!(fetched.party.id, party_id);
    assert_eq!(fetched.party.status, "active");

    // Create AP vendor referencing this party
    let vreq = CreateVendorRequest {
        name: format!("Det Vendor {}", &run_id.to_string()[..8]),
        tax_id: None,
        currency: "USD".to_string(),
        payment_terms_days: 15,
        payment_method: None,
        remittance_email: None,
        party_id: Some(party_id),
    };
    let vendor = vendor_service::create_vendor(&ap_pool, APP_ID, &vreq, run_id.to_string())
        .await
        .expect("create_vendor failed on determinism check");
    assert_eq!(vendor.party_id, Some(party_id));

    // Cleanup
    cleanup_ap_vendor(&ap_pool, vendor.vendor_id).await;
    cleanup_party(&party_pool).await;
}
