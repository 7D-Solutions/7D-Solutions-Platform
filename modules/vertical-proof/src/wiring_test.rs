//! End-to-end wiring tests that call all 5 platform modules via typed clients.
//!
//! Each test constructs clients from `PlatformServices` exactly as a real
//! vertical would via `ctx.platform_client::<T>()`, proving the SDK wiring
//! works without hand-written HTTP code.

use platform_client_ar::{
    CreateCustomerRequest, CreateInvoiceRequest, CustomersClient, InvoicesClient,
};
use platform_client_inventory::{CreateItemRequest, ItemsClient, TrackingMode};
use platform_client_notifications::{SendRequest, SendsClient};
use platform_client_party::{CreateCompanyRequest, PartiesClient};
use platform_client_production::{WorkcentersClient, CreateWorkcenterRequest};
use platform_sdk::{PlatformClient, PlatformService};

use crate::test_claims;

/// Helper: build a PlatformClient for a service from env or default URL.
fn client_for(service: &str, default_port: u16) -> PlatformClient {
    let env_var = format!("{}_BASE_URL", service.to_uppercase());
    let base_url = std::env::var(&env_var)
        .unwrap_or_else(|_| format!("http://localhost:{}", default_port));
    PlatformClient::new(base_url)
}

// ---------------------------------------------------------------------------
// 1. Party — create a company, list parties
// ---------------------------------------------------------------------------

pub async fn test_party_wiring() -> Result<(), String> {
    let client = client_for("party", 8098);
    let parties = PartiesClient::from_platform_client(client);
    let claims = test_claims();

    // Create a company
    let body = CreateCompanyRequest {
        display_name: "Vertical Proof Corp".into(),
        legal_name: "Vertical Proof Corp LLC".into(),
        email: Some("proof@example.com".into()),
        ..default_create_company()
    };
    let created = parties
        .create_company(&claims, &body)
        .await
        .map_err(|e| format!("Party create_company failed: {e}"))?;
    tracing::info!(party_id = %created.party.id, "Party: company created");

    // List parties
    let page = parties
        .list_parties(&claims, None, Some(1), Some(10))
        .await
        .map_err(|e| format!("Party list_parties failed: {e}"))?;
    tracing::info!(count = page.data.len(), "Party: listed parties");

    Ok(())
}

// ---------------------------------------------------------------------------
// 2. AR — create customer, create invoice
// ---------------------------------------------------------------------------

pub async fn test_ar_wiring() -> Result<(), String> {
    let client = client_for("ar", 8086);
    let customers = CustomersClient::from_platform_client(client.clone());
    let invoices = InvoicesClient::from_platform_client(client);
    let claims = test_claims();

    // Create an AR customer
    let cust = customers
        .create_customer(
            &claims,
            &CreateCustomerRequest {
                name: Some("Proof Customer".into()),
                email: Some("proof-ar@example.com".into()),
                external_customer_id: None,
                metadata: None,
                party_id: None,
            },
        )
        .await
        .map_err(|e| format!("AR create_customer failed: {e}"))?;
    tracing::info!(customer_id = cust.id, "AR: customer created");

    // Create an invoice for this customer
    let inv = invoices
        .create_invoice(
            &claims,
            &CreateInvoiceRequest {
                ar_customer_id: cust.id,
                amount_cents: 10000,
                currency: Some("USD".into()),
                due_at: None,
                billing_period_start: None,
                billing_period_end: None,
                compliance_codes: None,
                correlation_id: None,
                line_item_details: None,
                metadata: None,
                party_id: None,
                status: None,
                subscription_id: None,
            },
        )
        .await
        .map_err(|e| format!("AR create_invoice failed: {e}"))?;
    tracing::info!(invoice_id = inv.id, "AR: invoice created");

    Ok(())
}

// ---------------------------------------------------------------------------
// 3. Inventory — create item, list items
// ---------------------------------------------------------------------------

pub async fn test_inventory_wiring() -> Result<(), String> {
    let client = client_for("inventory", 8092);
    let items = ItemsClient::from_platform_client(client);
    let claims = test_claims();

    let tenant_id = claims.tenant_id.to_string();

    let item = items
        .create_item(
            &claims,
            &CreateItemRequest {
                name: "Proof Widget".into(),
                sku: format!("PROOF-{}", uuid::Uuid::new_v4().simple()),
                tenant_id,
                tracking_mode: TrackingMode::None,
                cogs_account_ref: "5000".into(),
                inventory_account_ref: "1200".into(),
                variance_account_ref: "5010".into(),
                description: Some("Test item for vertical proof".into()),
                make_buy: Some("buy".into()),
                uom: None,
            },
        )
        .await
        .map_err(|e| format!("Inventory create_item failed: {e}"))?;
    tracing::info!(item_id = %item.id, "Inventory: item created");

    let page = items
        .list_items(&claims, None, None, None, None, Some(1), Some(10))
        .await
        .map_err(|e| format!("Inventory list_items failed: {e}"))?;
    tracing::info!(count = page.data.len(), "Inventory: listed items");

    Ok(())
}

// ---------------------------------------------------------------------------
// 4. Production — create workcenter
// ---------------------------------------------------------------------------

pub async fn test_production_wiring() -> Result<(), String> {
    let client = client_for("production", 8108);
    let workcenters = WorkcentersClient::from_platform_client(client);
    let claims = test_claims();

    let tenant_id = claims.tenant_id.to_string();

    let wc = workcenters
        .create_workcenter(
            &claims,
            &CreateWorkcenterRequest {
                code: format!("WC-PROOF-{}", &uuid::Uuid::new_v4().simple().to_string()[..6]),
                name: "Proof Workcenter".into(),
                tenant_id,
                capacity: Some(10),
                cost_rate_minor: Some(5000),
                description: Some("Test workcenter for vertical proof".into()),
                idempotency_key: Some(uuid::Uuid::new_v4().to_string()),
            },
        )
        .await
        .map_err(|e| format!("Production create_workcenter failed: {e}"))?;
    tracing::info!(wc_id = %wc.workcenter_id, "Production: workcenter created");

    Ok(())
}

// ---------------------------------------------------------------------------
// 5. Notifications — send a notification
// ---------------------------------------------------------------------------

pub async fn test_notifications_wiring() -> Result<(), String> {
    let client = client_for("notifications", 8089);
    let sends = SendsClient::from_platform_client(client);
    let claims = test_claims();

    let result = sends
        .send_notification(
            &claims,
            &SendRequest {
                template_key: "vertical_proof_test".into(),
                channel: "in_app".into(),
                recipients: vec!["proof-user@example.com".into()],
                payload_json: serde_json::json!({
                    "message": "Vertical proof notification test"
                }),
                correlation_id: Some(uuid::Uuid::new_v4().to_string()),
                causation_id: None,
            },
        )
        .await
        .map_err(|e| format!("Notifications send failed: {e}"))?;
    tracing::info!(send_id = %result.id, "Notifications: notification sent");

    Ok(())
}

// ---------------------------------------------------------------------------
// 6. Outbox — publish an event through the outbox table
// ---------------------------------------------------------------------------

pub async fn test_outbox_publish(pool: &sqlx::PgPool) -> Result<(), String> {
    let event_id = uuid::Uuid::new_v4();
    let payload = serde_json::json!({
        "source": "vertical-proof",
        "action": "wiring_test_complete",
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });

    sqlx::query(
        "INSERT INTO events_outbox (event_id, event_type, payload) VALUES ($1, $2, $3)",
    )
    .bind(event_id)
    .bind("vertical_proof.test_event")
    .bind(&payload)
    .execute(pool)
    .await
    .map_err(|e| format!("Outbox insert failed: {e}"))?;

    tracing::info!(event_id = %event_id, "Outbox: event inserted for publishing");

    // Verify it was inserted
    let row = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM events_outbox WHERE event_id = $1")
        .bind(event_id)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("Outbox count query failed: {e}"))?;
    assert_eq!(row, 1, "event should be in outbox");

    Ok(())
}

// ---------------------------------------------------------------------------
// Run all tests
// ---------------------------------------------------------------------------

pub struct WiringResults {
    pub party: Result<(), String>,
    pub ar: Result<(), String>,
    pub inventory: Result<(), String>,
    pub production: Result<(), String>,
    pub notifications: Result<(), String>,
    pub outbox: Result<(), String>,
}

impl WiringResults {
    pub fn summary(&self) -> String {
        let results = [
            ("Party", &self.party),
            ("AR", &self.ar),
            ("Inventory", &self.inventory),
            ("Production", &self.production),
            ("Notifications", &self.notifications),
            ("Outbox", &self.outbox),
        ];

        let mut out = String::from("=== Vertical Wiring Test Results ===\n");
        let mut pass = 0;
        let mut fail = 0;
        for (name, result) in &results {
            match result {
                Ok(()) => {
                    out.push_str(&format!("  [PASS] {name}\n"));
                    pass += 1;
                }
                Err(e) => {
                    out.push_str(&format!("  [FAIL] {name}: {e}\n"));
                    fail += 1;
                }
            }
        }
        out.push_str(&format!("\n{pass} passed, {fail} failed out of {} tests\n", pass + fail));
        out
    }

    pub fn all_passed(&self) -> bool {
        self.party.is_ok()
            && self.ar.is_ok()
            && self.inventory.is_ok()
            && self.production.is_ok()
            && self.notifications.is_ok()
            && self.outbox.is_ok()
    }
}

pub async fn run_all(pool: &sqlx::PgPool) -> WiringResults {
    WiringResults {
        party: test_party_wiring().await,
        ar: test_ar_wiring().await,
        inventory: test_inventory_wiring().await,
        production: test_production_wiring().await,
        notifications: test_notifications_wiring().await,
        outbox: test_outbox_publish(pool).await,
    }
}

// ---------------------------------------------------------------------------
// Default builders for types with many fields
// ---------------------------------------------------------------------------

fn default_create_company() -> CreateCompanyRequest {
    CreateCompanyRequest {
        display_name: String::new(),
        legal_name: String::new(),
        email: None,
        phone: None,
        website: None,
        address_line1: None,
        address_line2: None,
        city: None,
        state: None,
        postal_code: None,
        country: None,
        tax_id: None,
        registration_number: None,
        industry_code: None,
        employee_count: None,
        annual_revenue_cents: None,
        founded_date: None,
        country_of_incorporation: None,
        currency: None,
        trade_name: None,
        metadata: None,
    }
}
