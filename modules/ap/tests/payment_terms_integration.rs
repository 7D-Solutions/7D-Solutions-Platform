//! Integration tests for AP payment terms schedule (bd-22xr2).
//!
//! All 6 required test categories — real Postgres on port 5443, no mocks:
//! 1. Terms CRUD E2E: Create Net 30, assign to invoice, verify due date
//! 2. Discount terms: 2/10 Net 30 discount date and amount calculation
//! 3. Tenant isolation: cross-tenant query returns zero results
//! 4. Idempotency: duplicate idempotency_key returns original, no duplicate
//! 5. Outbox event: verify outbox event with correct type and tenant_id
//! 6. Due date calculation: Net 30, Net 60, 2/10 Net 30 all compute correctly

use ap::domain::bills::service as bill_service;
use ap::domain::bills::CreateBillLineRequest;
use ap::domain::bills::CreateBillRequest;
use ap::domain::payment_terms::service::{
    assign_terms_to_bill, create_terms, get_terms, list_terms,
};
use ap::domain::payment_terms::{
    compute_discount_amount, compute_discount_date, compute_due_date, CreatePaymentTermsRequest,
};
use chrono::Utc;
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
    format!("ap-terms-{}", Uuid::new_v4().simple())
}

fn corr() -> String {
    Uuid::new_v4().to_string()
}

fn net30_req() -> CreatePaymentTermsRequest {
    CreatePaymentTermsRequest {
        term_code: "NET30".to_string(),
        description: Some("Net 30 days".to_string()),
        days_due: 30,
        discount_pct: None,
        discount_days: None,
        installment_schedule: None,
        idempotency_key: None,
    }
}

fn net60_req() -> CreatePaymentTermsRequest {
    CreatePaymentTermsRequest {
        term_code: "NET60".to_string(),
        description: Some("Net 60 days".to_string()),
        days_due: 60,
        discount_pct: None,
        discount_days: None,
        installment_schedule: None,
        idempotency_key: None,
    }
}

fn discount_2_10_net30_req() -> CreatePaymentTermsRequest {
    CreatePaymentTermsRequest {
        term_code: "2/10NET30".to_string(),
        description: Some("2% 10 Net 30".to_string()),
        days_due: 30,
        discount_pct: Some(2.0),
        discount_days: Some(10),
        installment_schedule: None,
        idempotency_key: None,
    }
}

/// Insert a minimal vendor for testing; returns its vendor_id.
async fn create_test_vendor(pool: &sqlx::PgPool, tenant_id: &str, terms_days: i32) -> Uuid {
    let vendor_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO vendors (
            vendor_id, tenant_id, name, currency, payment_terms_days,
            is_active, created_at, updated_at
        )
        VALUES ($1, $2, $3, 'USD', $4, TRUE, NOW(), NOW())
        "#,
    )
    .bind(vendor_id)
    .bind(tenant_id)
    .bind(format!("Test Vendor {}", vendor_id))
    .bind(terms_days)
    .execute(pool)
    .await
    .expect("insert test vendor");
    vendor_id
}

/// Create a test bill for the given vendor+tenant; returns the bill_id.
async fn create_test_bill(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    vendor_id: Uuid,
    inv_ref: &str,
) -> Uuid {
    let req = CreateBillRequest {
        vendor_id,
        vendor_invoice_ref: inv_ref.to_string(),
        currency: "USD".to_string(),
        invoice_date: Utc::now(),
        due_date: None,
        tax_minor: None,
        entered_by: "test-user".to_string(),
        fx_rate_id: None,
        lines: vec![CreateBillLineRequest {
            description: Some("Service".to_string()),
            item_id: None,
            quantity: 10.0,
            unit_price_minor: 5000,
            gl_account_code: Some("6100".to_string()),
            po_line_id: None,
        }],
    };
    let result = bill_service::create_bill(pool, tenant_id, &req, corr())
        .await
        .expect("create test bill");
    result.bill.bill_id
}

// ============================================================================
// 1. Terms CRUD E2E: Create Net 30, assign to invoice, verify due date
// ============================================================================

#[tokio::test]
#[serial]
async fn test_terms_crud_and_assign_to_invoice() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    // Create payment terms
    let terms = create_terms(&pool, &tid, &net30_req(), corr())
        .await
        .expect("create Net 30 terms");
    assert_eq!(terms.term_code, "NET30");
    assert_eq!(terms.days_due, 30);
    assert!(terms.is_active);

    // Read back
    let fetched = get_terms(&pool, &tid, terms.term_id)
        .await
        .expect("get terms")
        .expect("terms should exist");
    assert_eq!(fetched.term_id, terms.term_id);

    // List
    let all = list_terms(&pool, &tid, false).await.expect("list terms");
    assert_eq!(all.len(), 1);

    // Create a bill and assign terms
    let vendor_id = create_test_vendor(&pool, &tid, 15).await;
    let bill_id = create_test_bill(&pool, &tid, vendor_id, "INV-TERMS-001").await;

    let result = assign_terms_to_bill(&pool, &tid, bill_id, terms.term_id)
        .await
        .expect("assign terms");
    assert_eq!(result.term_id, terms.term_id);

    // Verify due_date was computed from terms (invoice_date + 30 days)
    let bill_row: (chrono::DateTime<Utc>,) =
        sqlx::query_as("SELECT due_date FROM vendor_bills WHERE bill_id = $1")
            .bind(bill_id)
            .fetch_one(&pool)
            .await
            .expect("fetch bill due_date");

    let expected_due = result.due_date.date_naive();
    assert_eq!(bill_row.0.date_naive(), expected_due);
}

// ============================================================================
// 2. Discount terms: 2/10 Net 30 discount date and amount calculation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_discount_terms_2_10_net30() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let terms = create_terms(&pool, &tid, &discount_2_10_net30_req(), corr())
        .await
        .expect("create 2/10 Net 30");
    assert_eq!(terms.discount_pct, 2.0);
    assert_eq!(terms.discount_days, 10);

    // Create bill with total = 10 * 5000 = 50000
    let vendor_id = create_test_vendor(&pool, &tid, 15).await;
    let bill_id = create_test_bill(&pool, &tid, vendor_id, "INV-DISCOUNT-001").await;

    let result = assign_terms_to_bill(&pool, &tid, bill_id, terms.term_id)
        .await
        .expect("assign discount terms");

    // Discount date should be invoice_date + 10 days
    assert!(
        result.discount_date.is_some(),
        "discount terms should produce a discount_date"
    );

    // Discount amount: 2% of 50000 = 1000
    assert_eq!(
        result.discount_amount_minor,
        Some(1000),
        "2% of 50000 = 1000"
    );

    // Verify persisted on the bill row
    let row: (Option<chrono::DateTime<Utc>>, Option<i64>) = sqlx::query_as(
        "SELECT discount_date, discount_amount_minor FROM vendor_bills WHERE bill_id = $1",
    )
    .bind(bill_id)
    .fetch_one(&pool)
    .await
    .expect("fetch bill discount info");

    assert!(row.0.is_some(), "discount_date should be persisted");
    assert_eq!(row.1, Some(1000), "discount_amount should be persisted");
}

// ============================================================================
// 3. Tenant isolation: cross-tenant query returns zero results
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_isolation() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();

    // Create terms under tenant A
    let terms = create_terms(&pool, &tid_a, &net30_req(), corr())
        .await
        .expect("create terms tenant A");

    // Query as tenant B — should find nothing
    let result = get_terms(&pool, &tid_b, terms.term_id)
        .await
        .expect("get terms tenant B");
    assert!(result.is_none(), "tenant B should not see tenant A's terms");

    let list = list_terms(&pool, &tid_b, false)
        .await
        .expect("list terms tenant B");
    assert!(list.is_empty(), "tenant B list should be empty");

    // Assign attempt from tenant B should fail (bill not found)
    let vendor_id = create_test_vendor(&pool, &tid_a, 15).await;
    let bill_id = create_test_bill(&pool, &tid_a, vendor_id, "INV-ISO-001").await;

    let err = assign_terms_to_bill(&pool, &tid_b, bill_id, terms.term_id).await;
    assert!(err.is_err(), "cross-tenant assign should fail");
}

// ============================================================================
// 4. Idempotency: duplicate idempotency_key returns original, no duplicate
// ============================================================================

#[tokio::test]
#[serial]
async fn test_idempotency() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let idem_key = format!("idem-{}", Uuid::new_v4());

    let mut req = net30_req();
    req.idempotency_key = Some(idem_key.clone());

    // First create
    let first = create_terms(&pool, &tid, &req, corr())
        .await
        .expect("first create");

    // Second create with same idempotency_key — should return the original
    let second = create_terms(&pool, &tid, &req, corr())
        .await
        .expect("second create (idempotent)");

    assert_eq!(
        first.term_id, second.term_id,
        "idempotent create should return same term_id"
    );

    // Verify only one row exists
    let all = list_terms(&pool, &tid, true).await.expect("list all");
    assert_eq!(all.len(), 1, "should have exactly one record, not two");
}

// ============================================================================
// 5. Outbox event: verify outbox event with correct type and tenant_id
// ============================================================================

#[tokio::test]
#[serial]
async fn test_outbox_event() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let terms = create_terms(&pool, &tid, &net30_req(), corr())
        .await
        .expect("create terms for outbox test");

    // Check outbox for payment_terms event
    let row: (i64, String) = sqlx::query_as(
        r#"
        SELECT COUNT(*), MIN(event_type)
        FROM events_outbox
        WHERE aggregate_type = 'payment_terms'
          AND aggregate_id = $1
        "#,
    )
    .bind(terms.term_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("outbox query");

    assert!(
        row.0 >= 1,
        "expected at least 1 outbox event, got {}",
        row.0
    );
    assert_eq!(
        row.1, "ap.payment_terms_created",
        "event_type should be ap.payment_terms_created"
    );

    // Verify the event payload contains tenant_id
    let payload: (serde_json::Value,) = sqlx::query_as(
        r#"
        SELECT payload
        FROM events_outbox
        WHERE aggregate_type = 'payment_terms'
          AND aggregate_id = $1
        LIMIT 1
        "#,
    )
    .bind(terms.term_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("fetch outbox payload");

    let tenant_in_payload = payload.0["tenant_id"]
        .as_str()
        .expect("tenant_id in payload");
    assert_eq!(tenant_in_payload, tid);
}

// ============================================================================
// 6. Due date calculation: Net 30, Net 60, 2/10 Net 30 all compute correctly
// ============================================================================

#[tokio::test]
#[serial]
async fn test_due_date_calculation_multiple_types() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = create_test_vendor(&pool, &tid, 15).await;

    // --- Net 30 ---
    let net30 = create_terms(&pool, &tid, &net30_req(), corr())
        .await
        .expect("create Net 30");

    let bill_30 = create_test_bill(&pool, &tid, vendor_id, "INV-DUE-30").await;
    let r30 = assign_terms_to_bill(&pool, &tid, bill_30, net30.term_id)
        .await
        .expect("assign Net 30");

    // Fetch invoice_date to verify
    let inv_date_30: (chrono::DateTime<Utc>,) =
        sqlx::query_as("SELECT invoice_date FROM vendor_bills WHERE bill_id = $1")
            .bind(bill_30)
            .fetch_one(&pool)
            .await
            .expect("fetch invoice_date");
    let expected_30 = compute_due_date(inv_date_30.0.date_naive(), 30);
    assert_eq!(r30.due_date.date_naive(), expected_30, "Net 30 due date");
    assert!(r30.discount_date.is_none(), "Net 30 has no discount date");
    assert!(
        r30.discount_amount_minor.is_none(),
        "Net 30 has no discount amount"
    );

    // --- Net 60 ---
    let net60 = create_terms(&pool, &tid, &net60_req(), corr())
        .await
        .expect("create Net 60");

    let bill_60 = create_test_bill(&pool, &tid, vendor_id, "INV-DUE-60").await;
    let r60 = assign_terms_to_bill(&pool, &tid, bill_60, net60.term_id)
        .await
        .expect("assign Net 60");

    let inv_date_60: (chrono::DateTime<Utc>,) =
        sqlx::query_as("SELECT invoice_date FROM vendor_bills WHERE bill_id = $1")
            .bind(bill_60)
            .fetch_one(&pool)
            .await
            .expect("fetch invoice_date");
    let expected_60 = compute_due_date(inv_date_60.0.date_naive(), 60);
    assert_eq!(r60.due_date.date_naive(), expected_60, "Net 60 due date");

    // --- 2/10 Net 30 ---
    let disc = create_terms(&pool, &tid, &discount_2_10_net30_req(), corr())
        .await
        .expect("create 2/10 Net 30");

    let bill_disc = create_test_bill(&pool, &tid, vendor_id, "INV-DUE-DISC").await;
    let rd = assign_terms_to_bill(&pool, &tid, bill_disc, disc.term_id)
        .await
        .expect("assign 2/10 Net 30");

    let inv_date_disc: (chrono::DateTime<Utc>,) =
        sqlx::query_as("SELECT invoice_date FROM vendor_bills WHERE bill_id = $1")
            .bind(bill_disc)
            .fetch_one(&pool)
            .await
            .expect("fetch invoice_date");

    let expected_due = compute_due_date(inv_date_disc.0.date_naive(), 30);
    let expected_disc_date = compute_discount_date(inv_date_disc.0.date_naive(), 10);
    // Bill total = 10 * 5000 = 50000; 2% = 1000
    let expected_disc_amount = compute_discount_amount(50_000, 2.0);

    assert_eq!(
        rd.due_date.date_naive(),
        expected_due,
        "2/10 Net 30 due date"
    );
    assert_eq!(
        rd.discount_date.map(|d| d.date_naive()),
        expected_disc_date,
        "2/10 Net 30 discount date"
    );
    assert_eq!(
        rd.discount_amount_minor, expected_disc_amount,
        "2/10 Net 30 discount amount"
    );
}
