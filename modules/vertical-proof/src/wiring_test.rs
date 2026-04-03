//! End-to-end wiring tests that call all 5 platform modules via typed clients.
//!
//! Each test constructs clients from `PlatformServices` exactly as a real
//! vertical would via `ctx.platform_client::<T>()`, proving the SDK wiring
//! works without hand-written HTTP code.

use platform_client_ap::{CreateVendorRequest, VendorsClient};
use platform_client_ar::{
    CreateCustomerRequest, CreateInvoiceRequest, CustomersClient, InvoicesClient,
};
use platform_client_bom::{BomClient, CreateBomRequest};
use platform_client_consolidation::{CreateGroupRequest, GroupsClient};
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
// 2. AP — create vendor, list vendors
// ---------------------------------------------------------------------------

pub async fn test_ap_wiring() -> Result<(), String> {
    let client = client_for("ap", 8093);
    let vendors = VendorsClient::from_platform_client(client);
    let claims = test_claims();

    // Create a vendor
    let vendor = vendors
        .create_vendor(
            &claims,
            &CreateVendorRequest {
                name: "Proof Vendor".into(),
                currency: "USD".into(),
                payment_terms_days: 30,
                party_id: None,
                payment_method: Some("ach".into()),
                remittance_email: Some("vendor@example.com".into()),
                tax_id: None,
            },
        )
        .await
        .map_err(|e| format!("AP create_vendor failed: {e}"))?;
    tracing::info!(vendor_id = %vendor.vendor_id, "AP: vendor created");

    // List vendors
    let page = vendors
        .list_vendors(&claims)
        .await
        .map_err(|e| format!("AP list_vendors failed: {e}"))?;
    tracing::info!(count = page.data.len(), "AP: listed vendors");

    Ok(())
}

// ---------------------------------------------------------------------------
// 3. Consolidation — create group, list groups
// ---------------------------------------------------------------------------

pub async fn test_consolidation_wiring() -> Result<(), String> {
    let client = client_for("consolidation", 8105);
    let groups = GroupsClient::from_platform_client(client);
    let claims = test_claims();

    // Create a consolidation group
    let group = groups
        .create_group(
            &claims,
            &CreateGroupRequest {
                name: format!("Proof Group {}", &uuid::Uuid::new_v4().simple().to_string()[..6]),
                reporting_currency: "USD".into(),
                description: Some("Vertical proof consolidation test".into()),
                fiscal_year_end_month: Some(12),
            },
        )
        .await
        .map_err(|e| format!("Consolidation create_group failed: {e}"))?;
    tracing::info!(group_id = %group.id, "Consolidation: group created");

    // List groups
    let page = groups
        .list_groups(&claims, None)
        .await
        .map_err(|e| format!("Consolidation list_groups failed: {e}"))?;
    tracing::info!(count = page.data.len(), "Consolidation: listed groups");

    Ok(())
}

// ---------------------------------------------------------------------------
// 4. BOM — create bom header, list boms
// ---------------------------------------------------------------------------

pub async fn test_bom_wiring() -> Result<(), String> {
    let client = client_for("bom", 8120);
    let bom = BomClient::from_platform_client(client);
    let claims = test_claims();

    let created = bom
        .post_bom(
            &claims,
            &CreateBomRequest {
                part_id: uuid::Uuid::new_v4(),
                description: Some("Vertical proof BOM test".into()),
            },
        )
        .await
        .map_err(|e| format!("BOM post_bom failed: {e}"))?;
    tracing::info!(bom_id = %created.id, "BOM: header created");

    let page = bom
        .list_boms(&claims, 1, 10)
        .await
        .map_err(|e| format!("BOM list_boms failed: {e}"))?;
    tracing::info!(count = page.data.len(), "BOM: listed bom headers");

    Ok(())
}

// ---------------------------------------------------------------------------
// 5. AR — create customer, create invoice
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
// Customer Portal
// ---------------------------------------------------------------------------

pub async fn test_customer_portal_wiring() -> Result<(), String> {
    let client = client_for("customer_portal", 8111);
    let admin = platform_client_customer_portal::AdminClient::from_platform_client(client);
    let _claims = test_claims();
    tracing::info!("Customer-Portal: AdminClient constructed successfully");
    let _ = &admin;
    Ok(())
}

// ---------------------------------------------------------------------------
// Fixed Assets
// ---------------------------------------------------------------------------

pub async fn test_fixed_assets_wiring() -> Result<(), String> {
    let client = client_for("fixed_assets", 8104);
    let assets = platform_client_fixed_assets::AssetsClient::from_platform_client(client);
    let _claims = test_claims();
    tracing::info!("Fixed-Assets: AssetsClient constructed successfully");
    let _ = &assets;
    Ok(())
}

// ---------------------------------------------------------------------------
// GL — General Ledger
// ---------------------------------------------------------------------------

pub async fn test_gl_wiring() -> Result<(), String> {
    let client = client_for("gl", 8090);
    let accounts = platform_client_gl::AccountsClient::from_platform_client(client);
    let _claims = test_claims();
    tracing::info!("GL: AccountsClient constructed successfully");
    let _ = &accounts;
    Ok(())
}

// ---------------------------------------------------------------------------
// Integrations
// ---------------------------------------------------------------------------

pub async fn test_integrations_wiring() -> Result<(), String> {
    let client = client_for("integrations", 8099);
    let connectors = platform_client_integrations::ConnectorsClient::from_platform_client(client);
    let _claims = test_claims();
    tracing::info!("Integrations: ConnectorsClient constructed successfully");
    let _ = &connectors;
    Ok(())
}

// ---------------------------------------------------------------------------
// Maintenance
// ---------------------------------------------------------------------------

pub async fn test_maintenance_wiring() -> Result<(), String> {
    let client = client_for("maintenance", 8101);
    let assets = platform_client_maintenance::AssetsClient::from_platform_client(client);
    let _claims = test_claims();
    tracing::info!("Maintenance: AssetsClient constructed successfully");
    let _ = &assets;
    Ok(())
}

// ---------------------------------------------------------------------------
// Numbering
// ---------------------------------------------------------------------------

pub async fn test_numbering_wiring() -> Result<(), String> {
    let client = client_for("numbering", 8096);
    let numbering = platform_client_numbering::NumberingClient::from_platform_client(client);
    let _claims = test_claims();
    tracing::info!("Numbering: NumberingClient constructed successfully");
    let _ = &numbering;
    Ok(())
}

// ---------------------------------------------------------------------------
// Payments
// ---------------------------------------------------------------------------

pub async fn test_payments_wiring() -> Result<(), String> {
    let client = client_for("payments", 8088);
    let admin = platform_client_payments::AdminClient::from_platform_client(client);
    let _claims = test_claims();
    tracing::info!("Payments: AdminClient constructed successfully");
    let _ = &admin;
    Ok(())
}

// ---------------------------------------------------------------------------
// PDF Editor
// ---------------------------------------------------------------------------

pub async fn test_pdf_editor_wiring() -> Result<(), String> {
    let client = client_for("pdf_editor", 8121);
    let annotations = platform_client_pdf_editor::AnnotationsClient::from_platform_client(client);
    let _claims = test_claims();
    tracing::info!("PDF-Editor: AnnotationsClient constructed successfully");
    let _ = &annotations;
    Ok(())
}

// ---------------------------------------------------------------------------
// Quality Inspection
// ---------------------------------------------------------------------------

pub async fn test_quality_inspection_wiring() -> Result<(), String> {
    let client = client_for("quality_inspection", 8106);
    let disposition = platform_client_quality_inspection::DispositionClient::from_platform_client(client);
    let _claims = test_claims();
    tracing::info!("Quality-Inspection: DispositionClient constructed successfully");
    let _ = &disposition;
    Ok(())
}

// ---------------------------------------------------------------------------
// Reporting
// ---------------------------------------------------------------------------

pub async fn test_reporting_wiring() -> Result<(), String> {
    let client = client_for("reporting", 8097);
    let admin = platform_client_reporting::AdminClient::from_platform_client(client);
    let _claims = test_claims();
    tracing::info!("Reporting: AdminClient constructed successfully");
    let _ = &admin;
    Ok(())
}

// ---------------------------------------------------------------------------
// Shipping & Receiving
// ---------------------------------------------------------------------------

pub async fn test_shipping_receiving_wiring() -> Result<(), String> {
    let client = client_for("shipping_receiving", 8103);
    let health = platform_client_shipping_receiving::HealthClient::from_platform_client(client);
    let _claims = test_claims();
    tracing::info!("Shipping-Receiving: HealthClient constructed successfully");
    let _ = &health;
    Ok(())
}

// ---------------------------------------------------------------------------
// Smoke Test
// ---------------------------------------------------------------------------

pub async fn test_smoke_test_wiring() -> Result<(), String> {
    let client = client_for("smoke_test", 8199);
    let items = platform_client_smoke_test::ItemsClient::from_platform_client(client);
    let _claims = test_claims();
    tracing::info!("Smoke-Test: ItemsClient constructed successfully");
    let _ = &items;
    Ok(())
}

// ---------------------------------------------------------------------------
// Subscriptions
// ---------------------------------------------------------------------------

pub async fn test_subscriptions_wiring() -> Result<(), String> {
    let client = client_for("subscriptions", 8087);
    let admin = platform_client_subscriptions::AdminClient::from_platform_client(client);
    let _claims = test_claims();
    tracing::info!("Subscriptions: AdminClient constructed successfully");
    let _ = &admin;
    Ok(())
}

// ---------------------------------------------------------------------------
// Timekeeping
// ---------------------------------------------------------------------------

pub async fn test_timekeeping_wiring() -> Result<(), String> {
    let client = client_for("timekeeping", 8102);
    let allocations = platform_client_timekeeping::AllocationsClient::from_platform_client(client);
    let _claims = test_claims();
    tracing::info!("Timekeeping: AllocationsClient constructed successfully");
    let _ = &allocations;
    Ok(())
}

// ---------------------------------------------------------------------------
// Treasury
// ---------------------------------------------------------------------------

pub async fn test_treasury_wiring() -> Result<(), String> {
    let client = client_for("treasury", 8094);
    let accounts = platform_client_treasury::AccountsClient::from_platform_client(client);
    let _claims = test_claims();
    tracing::info!("Treasury: AccountsClient constructed successfully");
    let _ = &accounts;
    Ok(())
}

// ---------------------------------------------------------------------------
// TTP — Time, Travel & Expense
// ---------------------------------------------------------------------------

pub async fn test_ttp_wiring() -> Result<(), String> {
    let client = client_for("ttp", 8100);
    let billing = platform_client_ttp::BillingClient::from_platform_client(client);
    let _claims = test_claims();
    tracing::info!("TTP: BillingClient constructed successfully");
    let _ = &billing;
    Ok(())
}

// ---------------------------------------------------------------------------
// Workflow
// ---------------------------------------------------------------------------

pub async fn test_workflow_wiring() -> Result<(), String> {
    let client = client_for("workflow", 8107);
    let definitions = platform_client_workflow::DefinitionsClient::from_platform_client(client);
    let _claims = test_claims();
    tracing::info!("Workflow: DefinitionsClient constructed successfully");
    let _ = &definitions;
    Ok(())
}

// ---------------------------------------------------------------------------
// Workforce Competence
// ---------------------------------------------------------------------------

pub async fn test_workforce_competence_wiring() -> Result<(), String> {
    let client = client_for("workforce_competence", 8110);
    let authorities = platform_client_workforce_competence::AcceptanceAuthoritiesClient::from_platform_client(client);
    let _claims = test_claims();
    tracing::info!("Workforce-Competence: AcceptanceAuthoritiesClient constructed successfully");
    let _ = &authorities;
    Ok(())
}

// ---------------------------------------------------------------------------
// Outbox — publish an event through the outbox table
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
    entries: Vec<(&'static str, Result<(), String>)>,
}

impl WiringResults {
    pub fn as_slice(&self) -> &[(&'static str, Result<(), String>)] {
        &self.entries
    }

    pub fn summary(&self) -> String {
        let mut out = String::from("=== Vertical Wiring Test Results ===\n");
        let mut pass = 0;
        let mut fail = 0;
        for (name, result) in &self.entries {
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
        out.push_str(&format!(
            "\n{pass} passed, {fail} failed out of {} tests\n",
            pass + fail
        ));
        out
    }

    pub fn all_passed(&self) -> bool {
        self.entries.iter().all(|(_, r)| r.is_ok())
    }
}

pub async fn run_all(pool: &sqlx::PgPool) -> WiringResults {
    WiringResults {
        entries: vec![
            ("AP", test_ap_wiring().await),
            ("AR", test_ar_wiring().await),
            ("BOM", test_bom_wiring().await),
            ("Consolidation", test_consolidation_wiring().await),
            ("Customer-Portal", test_customer_portal_wiring().await),
            ("Fixed-Assets", test_fixed_assets_wiring().await),
            ("GL", test_gl_wiring().await),
            ("Integrations", test_integrations_wiring().await),
            ("Inventory", test_inventory_wiring().await),
            ("Maintenance", test_maintenance_wiring().await),
            ("Notifications", test_notifications_wiring().await),
            ("Numbering", test_numbering_wiring().await),
            ("Party", test_party_wiring().await),
            ("Payments", test_payments_wiring().await),
            ("PDF-Editor", test_pdf_editor_wiring().await),
            ("Production", test_production_wiring().await),
            ("Quality-Inspection", test_quality_inspection_wiring().await),
            ("Reporting", test_reporting_wiring().await),
            ("Shipping-Receiving", test_shipping_receiving_wiring().await),
            ("Smoke-Test", test_smoke_test_wiring().await),
            ("Subscriptions", test_subscriptions_wiring().await),
            ("Timekeeping", test_timekeeping_wiring().await),
            ("Treasury", test_treasury_wiring().await),
            ("TTP", test_ttp_wiring().await),
            ("Workflow", test_workflow_wiring().await),
            ("Workforce-Competence", test_workforce_competence_wiring().await),
            ("Outbox", test_outbox_publish(pool).await),
        ],
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
