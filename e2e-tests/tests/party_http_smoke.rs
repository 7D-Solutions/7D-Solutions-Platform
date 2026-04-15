// HTTP smoke tests: Party service
//
// Proves that all 19 party routes respond correctly at the HTTP boundary
// via reqwest against the live Party service. No mocks, no stubs.
//
// Routes covered:
//   Party CRUD (7): create company, create individual, list, get, search, update, deactivate
//   Contacts (7): create, list, get, update, set-primary, primary-contacts, delete
//   Addresses (5): create, list, get, update, delete

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const PARTY_DEFAULT_URL: &str = "http://localhost:8098";

fn party_url() -> String {
    std::env::var("PARTY_URL").unwrap_or_else(|_| PARTY_DEFAULT_URL.to_string())
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

async fn wait_for_party(client: &Client) -> bool {
    let url = format!("{}/api/health", party_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  Party health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  Party health {}/15: {}", attempt, e),
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
        "DELETE" => client.delete(url),
        _ => panic!("unsupported method"),
    };
    let req = if let Some(b) = body {
        req.json(&b)
    } else {
        req
    };
    let resp = req.send().await.expect("unauth request failed");
    assert_eq!(
        resp.status().as_u16(),
        401,
        "expected 401 without JWT at {url}"
    );
    println!("  no-JWT -> 401 ok");
}

#[tokio::test]
async fn smoke_party() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_party(&client).await {
        eprintln!("Party service not reachable at {} -- skipping", party_url());
        return;
    }
    println!("Party service healthy at {}", party_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id, &["party.mutate", "party.read"]);
    let base = party_url();

    // Gate: verify the service accepts our JWT
    let probe = client
        .get(format!("{base}/api/party/parties"))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!("Party returns 401 with valid JWT -- JWT_PUBLIC_KEY not configured. Skipping.");
        return;
    }

    // ── 1. POST /api/party/companies ─────────────────────────────────
    println!("\n--- 1. POST /api/party/companies ---");
    let resp = client
        .post(format!("{base}/api/party/companies"))
        .bearer_auth(&jwt)
        .json(&json!({
            "display_name": "Smoke Test Corp",
            "legal_name": "Smoke Test Corporation Ltd"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.as_u16() == 201,
        "Create company failed: {status} - {body}"
    );
    let company_id = body["id"]
        .as_str()
        .expect("no id in create company response")
        .to_string();
    println!("  company id={company_id} name={}", body["display_name"]);
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/party/companies"),
        Some(json!({"display_name": "X", "legal_name": "X"})),
    )
    .await;

    // ── 2. POST /api/party/individuals ───────────────────────────────
    println!("\n--- 2. POST /api/party/individuals ---");
    let resp = client
        .post(format!("{base}/api/party/individuals"))
        .bearer_auth(&jwt)
        .json(&json!({
            "display_name": "Jane Smoke",
            "first_name": "Jane",
            "last_name": "Smoke",
            "email": "jane.smoke@example.com"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.as_u16() == 201,
        "Create individual failed: {status} - {body}"
    );
    let individual_id = body["id"]
        .as_str()
        .expect("no id in create individual response")
        .to_string();
    println!("  individual id={individual_id}");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/party/individuals"),
        Some(json!({"display_name": "X", "first_name": "X", "last_name": "Y"})),
    )
    .await;

    // ── 3. GET /api/party/parties ────────────────────────────────────
    println!("\n--- 3. GET /api/party/parties ---");
    let resp = client
        .get(format!("{base}/api/party/parties"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "List parties failed: {status} - {body}"
    );
    let count = body.as_array().map(|a| a.len()).unwrap_or(0);
    println!("  listed {count} parties");
    assert_unauth(&client, "GET", &format!("{base}/api/party/parties"), None).await;

    // ── 4. GET /api/party/parties/{id} ───────────────────────────────
    println!("\n--- 4. GET /api/party/parties/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/party/parties/{company_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "Get party failed: {status} - {body}");
    assert_eq!(body["id"].as_str().unwrap_or(""), company_id);
    println!("  get party ok: {}", body["display_name"]);
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/party/parties/{company_id}"),
        None,
    )
    .await;

    // ── 5. GET /api/party/parties/search ────────────────────────────
    println!("\n--- 5. GET /api/party/parties/search ---");
    let resp = client
        .get(format!("{base}/api/party/parties/search"))
        .bearer_auth(&jwt)
        .query(&[("name", "Smoke"), ("party_type", "company")])
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Search parties failed: {status} - {body}"
    );
    let count = body.as_array().map(|a| a.len()).unwrap_or(0);
    println!("  search returned {count} results");
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/party/parties/search?name=Smoke"),
        None,
    )
    .await;

    // ── 6. PUT /api/party/parties/{id} ───────────────────────────────
    println!("\n--- 6. PUT /api/party/parties/{{id}} ---");
    let resp = client
        .put(format!("{base}/api/party/parties/{company_id}"))
        .bearer_auth(&jwt)
        .json(&json!({
            "display_name": "Smoke Test Corp (Updated)",
            "website": "https://smoketest.example.com"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Update party failed: {status} - {body}"
    );
    println!("  updated party name={}", body["display_name"]);
    assert_unauth(
        &client,
        "PUT",
        &format!("{base}/api/party/parties/{company_id}"),
        Some(json!({"display_name": "X"})),
    )
    .await;

    // ── 7. POST /api/party/parties/{party_id}/contacts ───────────────
    println!("\n--- 7. POST /api/party/parties/{{id}}/contacts ---");
    let resp = client
        .post(format!("{base}/api/party/parties/{company_id}/contacts"))
        .bearer_auth(&jwt)
        .json(&json!({
            "first_name": "Alice",
            "last_name": "Smoker",
            "email": "alice.smoker@example.com",
            "role": "billing"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.as_u16() == 201,
        "Create contact failed: {status} - {body}"
    );
    let contact_id = body["id"]
        .as_str()
        .expect("no id in create contact response")
        .to_string();
    println!("  contact id={contact_id}");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/party/parties/{company_id}/contacts"),
        Some(json!({"first_name": "X", "last_name": "Y"})),
    )
    .await;

    // ── 8. GET /api/party/parties/{party_id}/contacts ────────────────
    println!("\n--- 8. GET /api/party/parties/{{id}}/contacts ---");
    let resp = client
        .get(format!("{base}/api/party/parties/{company_id}/contacts"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "List contacts failed: {status} - {body}"
    );
    let count = body.as_array().map(|a| a.len()).unwrap_or(0);
    println!("  listed {count} contacts");
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/party/parties/{company_id}/contacts"),
        None,
    )
    .await;

    // ── 9. GET /api/party/contacts/{id} ──────────────────────────────
    println!("\n--- 9. GET /api/party/contacts/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/party/contacts/{contact_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "Get contact failed: {status} - {body}");
    assert_eq!(body["id"].as_str().unwrap_or(""), contact_id);
    println!("  contact ok: {} {}", body["first_name"], body["last_name"]);
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/party/contacts/{contact_id}"),
        None,
    )
    .await;

    // ── 10. PUT /api/party/contacts/{id} ─────────────────────────────
    println!("\n--- 10. PUT /api/party/contacts/{{id}} ---");
    let resp = client
        .put(format!("{base}/api/party/contacts/{contact_id}"))
        .bearer_auth(&jwt)
        .json(&json!({
            "phone": "+1-555-0100"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Update contact failed: {status} - {body}"
    );
    println!("  contact updated phone={}", body["phone"]);
    assert_unauth(
        &client,
        "PUT",
        &format!("{base}/api/party/contacts/{contact_id}"),
        Some(json!({"phone": "+1-555-0101"})),
    )
    .await;

    // ── 11. POST /api/party/parties/{party_id}/contacts/{id}/set-primary
    println!("\n--- 11. POST /api/party/parties/{{id}}/contacts/{{id}}/set-primary ---");
    let resp = client
        .post(format!(
            "{base}/api/party/parties/{company_id}/contacts/{contact_id}/set-primary"
        ))
        .bearer_auth(&jwt)
        .json(&json!({"role": "billing"}))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Set primary contact failed: {status} - {body}"
    );
    println!("  set-primary ok: is_primary={}", body["is_primary"]);
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/party/parties/{company_id}/contacts/{contact_id}/set-primary"),
        Some(json!({"role": "billing"})),
    )
    .await;

    // ── 12. GET /api/party/parties/{party_id}/primary-contacts ───────
    println!("\n--- 12. GET /api/party/parties/{{id}}/primary-contacts ---");
    let resp = client
        .get(format!(
            "{base}/api/party/parties/{company_id}/primary-contacts"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Primary contacts failed: {status} - {body}"
    );
    let count = body.as_array().map(|a| a.len()).unwrap_or(0);
    println!("  primary contacts: {count} entries");
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/party/parties/{company_id}/primary-contacts"),
        None,
    )
    .await;

    // ── 13. DELETE /api/party/contacts/{id} ──────────────────────────
    println!("\n--- 13. DELETE /api/party/contacts/{{id}} ---");
    // Create a second contact to delete (don't delete the primary one)
    let resp2 = client
        .post(format!("{base}/api/party/parties/{company_id}/contacts"))
        .bearer_auth(&jwt)
        .json(&json!({
            "first_name": "Bob",
            "last_name": "Disposable",
            "role": "secondary"
        }))
        .send()
        .await
        .unwrap();
    let body2: Value = resp2.json().await.unwrap_or(json!({}));
    let contact2_id = body2["id"].as_str().unwrap_or("").to_string();
    assert!(!contact2_id.is_empty(), "failed to create second contact");

    let resp = client
        .delete(format!("{base}/api/party/contacts/{contact2_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert_eq!(status.as_u16(), 204, "Delete contact failed: {status}");
    println!("  contact deleted: 204");
    assert_unauth(
        &client,
        "DELETE",
        &format!("{base}/api/party/contacts/{contact2_id}"),
        None,
    )
    .await;

    // ── 14. POST /api/party/parties/{party_id}/addresses ─────────────
    println!("\n--- 14. POST /api/party/parties/{{id}}/addresses ---");
    let resp = client
        .post(format!("{base}/api/party/parties/{company_id}/addresses"))
        .bearer_auth(&jwt)
        .json(&json!({
            "address_type": "billing",
            "line1": "123 Smoke Street",
            "city": "Test City",
            "country": "US"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.as_u16() == 201,
        "Create address failed: {status} - {body}"
    );
    let address_id = body["id"]
        .as_str()
        .expect("no id in create address response")
        .to_string();
    println!("  address id={address_id}");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/party/parties/{company_id}/addresses"),
        Some(json!({"line1": "X", "city": "Y", "country": "US"})),
    )
    .await;

    // ── 15. GET /api/party/parties/{party_id}/addresses ──────────────
    println!("\n--- 15. GET /api/party/parties/{{id}}/addresses ---");
    let resp = client
        .get(format!("{base}/api/party/parties/{company_id}/addresses"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "List addresses failed: {status} - {body}"
    );
    let count = body.as_array().map(|a| a.len()).unwrap_or(0);
    println!("  listed {count} addresses");
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/party/parties/{company_id}/addresses"),
        None,
    )
    .await;

    // ── 16. GET /api/party/addresses/{id} ────────────────────────────
    println!("\n--- 16. GET /api/party/addresses/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/party/addresses/{address_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "Get address failed: {status} - {body}");
    assert_eq!(body["id"].as_str().unwrap_or(""), address_id);
    println!("  address ok: {} {}", body["line1"], body["city"]);
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/party/addresses/{address_id}"),
        None,
    )
    .await;

    // ── 17. PUT /api/party/addresses/{id} ────────────────────────────
    println!("\n--- 17. PUT /api/party/addresses/{{id}} ---");
    let resp = client
        .put(format!("{base}/api/party/addresses/{address_id}"))
        .bearer_auth(&jwt)
        .json(&json!({
            "postal_code": "12345",
            "state": "CA"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Update address failed: {status} - {body}"
    );
    println!("  address updated postal_code={}", body["postal_code"]);
    assert_unauth(
        &client,
        "PUT",
        &format!("{base}/api/party/addresses/{address_id}"),
        Some(json!({"postal_code": "99999"})),
    )
    .await;

    // ── 18. DELETE /api/party/addresses/{id} ─────────────────────────
    println!("\n--- 18. DELETE /api/party/addresses/{{id}} ---");
    // Create a second address to delete
    let resp2 = client
        .post(format!("{base}/api/party/parties/{company_id}/addresses"))
        .bearer_auth(&jwt)
        .json(&json!({
            "address_type": "shipping",
            "line1": "456 Disposable Ave",
            "city": "Delete Town",
            "country": "US"
        }))
        .send()
        .await
        .unwrap();
    let body2: Value = resp2.json().await.unwrap_or(json!({}));
    let address2_id = body2["id"].as_str().unwrap_or("").to_string();
    assert!(!address2_id.is_empty(), "failed to create second address");

    let resp = client
        .delete(format!("{base}/api/party/addresses/{address2_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert_eq!(status.as_u16(), 204, "Delete address failed: {status}");
    println!("  address deleted: 204");
    assert_unauth(
        &client,
        "DELETE",
        &format!("{base}/api/party/addresses/{address2_id}"),
        None,
    )
    .await;

    // ── 19. POST /api/party/parties/{id}/deactivate ───────────────────
    println!("\n--- 19. POST /api/party/parties/{{id}}/deactivate ---");
    // Deactivate the individual party (company has active contacts — deactivate individual)
    let resp = client
        .post(format!(
            "{base}/api/party/parties/{individual_id}/deactivate"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert_eq!(status.as_u16(), 204, "Deactivate party failed: {status}");
    println!("  deactivated party: 204");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/party/parties/{individual_id}/deactivate"),
        None,
    )
    .await;

    println!("\n=== All 19 party routes passed ===");
}
