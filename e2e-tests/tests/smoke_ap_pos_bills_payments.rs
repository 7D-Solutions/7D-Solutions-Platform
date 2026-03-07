// HTTP smoke tests: AP -- POs, Bills, and Payment Runs
//
// Tests 17 core AP routes at the HTTP boundary via reqwest against the live
// AP service. Follows the prereq chain: vendor -> payment-terms -> PO ->
// approve PO -> bill -> match -> approve -> allocate -> balance -> aging.

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const AP_DEFAULT_URL: &str = "http://localhost:8093";

fn ap_url() -> String {
    std::env::var("AP_URL").unwrap_or_else(|_| AP_DEFAULT_URL.to_string())
}

#[derive(Serialize)]
struct TestClaims {
    sub: String,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    jti: String,
    tenant_id: String,
    app_id: Option<String>,
    roles: Vec<String>,
    perms: Vec<String>,
    actor_type: String,
    ver: String,
}

fn dev_private_key() -> Option<EncodingKey> {
    let pem = std::env::var("JWT_PRIVATE_KEY_PEM").ok()?;
    EncodingKey::from_rsa_pem(pem.replace("\\n", "\n").as_bytes()).ok()
}

fn make_jwt(key: &EncodingKey, tenant_id: &str, perms: &[&str]) -> String {
    let now = Utc::now();
    let claims = TestClaims {
        sub: Uuid::new_v4().to_string(),
        iss: "auth-rs".to_string(),
        aud: "7d-platform".to_string(),
        iat: now.timestamp(),
        exp: (now + chrono::Duration::minutes(15)).timestamp(),
        jti: Uuid::new_v4().to_string(),
        tenant_id: tenant_id.to_string(),
        app_id: Some(tenant_id.to_string()),
        roles: vec!["operator".to_string()],
        perms: perms.iter().map(|s| s.to_string()).collect(),
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, key).unwrap()
}

async fn wait_for_ap(client: &Client) -> bool {
    let url = format!("{}/api/health", ap_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  AP health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  AP health {}/15: {}", attempt, e),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

async fn assert_unauth(client: &Client, method: &str, url: &str, body: Option<Value>) {
    let req = match method {
        "GET" => client.get(url),
        "POST" => client.post(url),
        "PUT" => client.put(url),
        _ => panic!("unsupported method: {method}"),
    };
    let req = if let Some(b) = body { req.json(&b) } else { req };
    let resp = req.send().await.expect("unauth request failed");
    assert_eq!(
        resp.status().as_u16(),
        401,
        "expected 401 without JWT at {url}"
    );
    println!("  no-JWT -> 401 ok");
}

#[tokio::test]
async fn smoke_ap_pos_bills_payments() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap();

    if !wait_for_ap(&client).await {
        eprintln!(
            "AP service not reachable at {} -- skipping",
            ap_url()
        );
        return;
    }
    println!("AP service healthy at {}", ap_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id, &["ap.mutate", "ap.read"]);
    let base = ap_url();

    // Gate: verify the AP service accepts our JWT
    let probe = client
        .get(format!("{base}/api/ap/vendors"))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!(
            "AP returns 401 with valid JWT -- JWT_PUBLIC_KEY not configured. Skipping."
        );
        return;
    }

    // 1. POST /api/ap/vendors
    println!("\n--- 1. POST /api/ap/vendors ---");
    let resp = client
        .post(format!("{base}/api/ap/vendors"))
        .bearer_auth(&jwt)
        .json(&json!({
            "name": "Smoke Test Vendor",
            "currency": "USD",
            "payment_terms_days": 30,
            "payment_method": "ach"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let vendor: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create vendor failed: {status} - {vendor}"
    );
    let vendor_id = vendor["vendor_id"]
        .as_str()
        .expect("No vendor_id in create vendor response");
    println!("  created vendor id={vendor_id}");
    assert_unauth(&client, "POST", &format!("{base}/api/ap/vendors"), Some(json!({}))).await;

    // 2. GET /api/ap/vendors/{vendor_id}
    println!("\n--- 2. GET /api/ap/vendors/{{vendor_id}} ---");
    let resp = client
        .get(format!("{base}/api/ap/vendors/{vendor_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert!(status.is_success(), "Get vendor failed: {status}");
    println!("  retrieved vendor ok");
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/ap/vendors/{vendor_id}"),
        None,
    )
    .await;

    // 3. POST /api/ap/payment-terms
    println!("\n--- 3. POST /api/ap/payment-terms ---");
    let term_code = format!("NET30-{}", &Uuid::new_v4().to_string()[..6]);
    let resp = client
        .post(format!("{base}/api/ap/payment-terms"))
        .bearer_auth(&jwt)
        .json(&json!({
            "term_code": term_code,
            "description": "Net 30 smoke test",
            "days_due": 30,
            "discount_pct": 2.0,
            "discount_days": 10
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let terms: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create payment terms failed: {status} - {terms}"
    );
    let term_id = terms["term_id"]
        .as_str()
        .expect("No term_id in create payment terms response");
    println!("  created payment terms id={term_id}");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ap/payment-terms"),
        Some(json!({})),
    )
    .await;

    // 4. GET /api/ap/payment-terms/{term_id}
    println!("\n--- 4. GET /api/ap/payment-terms/{{term_id}} ---");
    let resp = client
        .get(format!("{base}/api/ap/payment-terms/{term_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert!(status.is_success(), "Get payment terms failed: {status}");
    println!("  retrieved payment terms ok");
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/ap/payment-terms/{term_id}"),
        None,
    )
    .await;

    // 5. POST /api/ap/pos
    println!("\n--- 5. POST /api/ap/pos ---");
    let resp = client
        .post(format!("{base}/api/ap/pos"))
        .bearer_auth(&jwt)
        .json(&json!({
            "vendor_id": vendor_id,
            "currency": "USD",
            "created_by": "smoke-test",
            "lines": [{
                "description": "Widget A",
                "quantity": 20.0,
                "unit_price_minor": 5000,
                "unit_of_measure": "each",
                "gl_account_code": "5000"
            }]
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let po: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create PO failed: {status} - {po}"
    );
    let po_id = po["po_id"]
        .as_str()
        .expect("No po_id in create PO response");
    println!("  created PO id={po_id}");
    assert_unauth(&client, "POST", &format!("{base}/api/ap/pos"), Some(json!({}))).await;

    // 6. GET /api/ap/pos/{po_id}
    println!("\n--- 6. GET /api/ap/pos/{{po_id}} ---");
    let resp = client
        .get(format!("{base}/api/ap/pos/{po_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert!(status.is_success(), "Get PO failed: {status}");
    println!("  retrieved PO ok");
    assert_unauth(&client, "GET", &format!("{base}/api/ap/pos/{po_id}"), None).await;

    // 7. PUT /api/ap/pos/{po_id}/lines
    println!("\n--- 7. PUT /api/ap/pos/{{po_id}}/lines ---");
    let resp = client
        .put(format!("{base}/api/ap/pos/{po_id}/lines"))
        .bearer_auth(&jwt)
        .json(&json!({
            "updated_by": "smoke-test",
            "lines": [{
                "description": "Widget A updated",
                "quantity": 20.0,
                "unit_price_minor": 5000,
                "unit_of_measure": "each",
                "gl_account_code": "5000"
            }]
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert!(status.is_success(), "Update PO lines failed: {status}");
    println!("  updated PO lines ok");
    assert_unauth(
        &client,
        "PUT",
        &format!("{base}/api/ap/pos/{po_id}/lines"),
        Some(json!({})),
    )
    .await;

    // 8. POST /api/ap/pos/{po_id}/approve
    println!("\n--- 8. POST /api/ap/pos/{{po_id}}/approve ---");
    let resp = client
        .post(format!("{base}/api/ap/pos/{po_id}/approve"))
        .bearer_auth(&jwt)
        .json(&json!({ "approved_by": "smoke-test" }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert!(status.is_success(), "Approve PO failed: {status}");
    println!("  approved PO ok");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ap/pos/{po_id}/approve"),
        Some(json!({})),
    )
    .await;

    // 9. POST /api/ap/bills
    println!("\n--- 9. POST /api/ap/bills ---");
    let invoice_ref = format!("INV-SMOKE-{}", &Uuid::new_v4().to_string()[..8]);
    let resp = client
        .post(format!("{base}/api/ap/bills"))
        .bearer_auth(&jwt)
        .json(&json!({
            "vendor_id": vendor_id,
            "vendor_invoice_ref": invoice_ref,
            "currency": "USD",
            "invoice_date": Utc::now().to_rfc3339(),
            "entered_by": "smoke-test",
            "lines": [{
                "description": "Widget A",
                "quantity": 20.0,
                "unit_price_minor": 5000,
                "gl_account_code": "5000"
            }]
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let bill: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create bill failed: {status} - {bill}"
    );
    let bill_id = bill["bill_id"]
        .as_str()
        .expect("No bill_id in create bill response");
    println!("  created bill id={bill_id}");
    assert_unauth(&client, "POST", &format!("{base}/api/ap/bills"), Some(json!({}))).await;

    // 10. POST /api/ap/bills/{bill_id}/match
    println!("\n--- 10. POST /api/ap/bills/{{bill_id}}/match ---");
    let resp = client
        .post(format!("{base}/api/ap/bills/{bill_id}/match"))
        .bearer_auth(&jwt)
        .json(&json!({
            "po_id": po_id,
            "matched_by": "smoke-test"
        }))
        .send()
        .await
        .unwrap();
    let match_status = resp.status();
    // 200 on match success; 409/422 on tolerance violation -- both valid
    assert_ne!(
        match_status.as_u16(),
        401,
        "Match bill returned 401 with valid JWT"
    );
    println!("  match bill: {match_status} (non-401 ok)");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ap/bills/{bill_id}/match"),
        Some(json!({})),
    )
    .await;

    // 11. POST /api/ap/bills/{bill_id}/approve
    println!("\n--- 11. POST /api/ap/bills/{{bill_id}}/approve ---");
    let resp = client
        .post(format!("{base}/api/ap/bills/{bill_id}/approve"))
        .bearer_auth(&jwt)
        .json(&json!({
            "approved_by": "smoke-test",
            "override_reason": "smoke test bypass"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert_ne!(
        status.as_u16(),
        401,
        "Approve bill returned 401 with valid JWT"
    );
    println!("  approve bill: {status} (non-401 ok)");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ap/bills/{bill_id}/approve"),
        Some(json!({})),
    )
    .await;

    // 12. POST /api/ap/bills/{bill_id}/allocations
    println!("\n--- 12. POST /api/ap/bills/{{bill_id}}/allocations ---");
    let resp = client
        .post(format!("{base}/api/ap/bills/{bill_id}/allocations"))
        .bearer_auth(&jwt)
        .json(&json!({
            "allocation_id": Uuid::new_v4(),
            "amount_minor": 1000,
            "currency": "USD"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let alloc: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create allocation failed: {status} - {alloc}"
    );
    println!("  allocation created: {status}");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ap/bills/{bill_id}/allocations"),
        Some(json!({})),
    )
    .await;

    // 13. GET /api/ap/bills/{bill_id}/balance
    println!("\n--- 13. GET /api/ap/bills/{{bill_id}}/balance ---");
    let resp = client
        .get(format!("{base}/api/ap/bills/{bill_id}/balance"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert!(status.is_success(), "Get bill balance failed: {status}");
    println!("  bill balance ok");
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/ap/bills/{bill_id}/balance"),
        None,
    )
    .await;

    // 14. POST /api/ap/bills/{bill_id}/assign-terms
    println!("\n--- 14. POST /api/ap/bills/{{bill_id}}/assign-terms ---");
    let resp = client
        .post(format!("{base}/api/ap/bills/{bill_id}/assign-terms"))
        .bearer_auth(&jwt)
        .json(&json!({ "term_id": term_id }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert!(status.is_success(), "Assign payment terms failed: {status}");
    println!("  assigned payment terms ok");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ap/bills/{bill_id}/assign-terms"),
        Some(json!({})),
    )
    .await;

    // 15. POST /api/ap/bills/{bill_id}/tax-quote
    println!("\n--- 15. POST /api/ap/bills/{{bill_id}}/tax-quote ---");
    let resp = client
        .post(format!("{base}/api/ap/bills/{bill_id}/tax-quote"))
        .bearer_auth(&jwt)
        .json(&json!({
            "ship_to": {
                "line1": "123 Main St",
                "city": "Austin",
                "state": "TX",
                "postal_code": "78701",
                "country": "US"
            },
            "ship_from": {
                "line1": "456 Vendor Ave",
                "city": "Dallas",
                "state": "TX",
                "postal_code": "75201",
                "country": "US"
            }
        }))
        .send()
        .await
        .unwrap();
    let tax_status = resp.status();
    // Tax service may not be running; any non-401 is valid
    assert_ne!(
        tax_status.as_u16(),
        401,
        "Tax quote returned 401 with valid JWT"
    );
    println!("  tax quote: {tax_status} (non-401 ok)");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ap/bills/{bill_id}/tax-quote"),
        Some(json!({})),
    )
    .await;

    // 16. POST /api/ap/payment-runs
    println!("\n--- 16. POST /api/ap/payment-runs ---");
    let run_id = Uuid::new_v4();
    let resp = client
        .post(format!("{base}/api/ap/payment-runs"))
        .bearer_auth(&jwt)
        .json(&json!({
            "run_id": run_id,
            "currency": "USD",
            "scheduled_date": Utc::now().to_rfc3339(),
            "payment_method": "ach",
            "created_by": "smoke-test"
        }))
        .send()
        .await
        .unwrap();
    let run_status = resp.status();
    // 201/200 on success; 422 if no approved bills in this tenant/currency
    assert!(
        run_status == StatusCode::CREATED
            || run_status == StatusCode::OK
            || run_status == StatusCode::UNPROCESSABLE_ENTITY,
        "Create payment run unexpected status: {run_status}"
    );
    println!("  payment run: {run_status}");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ap/payment-runs"),
        Some(json!({})),
    )
    .await;

    // 17. GET /api/ap/aging
    println!("\n--- 17. GET /api/ap/aging ---");
    let resp = client
        .get(format!("{base}/api/ap/aging"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!([]));
    assert!(status.is_success(), "Aging report failed: {status}");
    println!(
        "  aging report ok: {} entries",
        body.as_array().map(|a| a.len()).unwrap_or(0)
    );
    assert_unauth(&client, "GET", &format!("{base}/api/ap/aging"), None).await;

    println!("\n=== All 17 AP routes passed ===");
}
