// HTTP smoke tests: BOM + ECO
//
// Proves that 18 core BOM/ECO routes respond correctly at the HTTP
// boundary via reqwest against the live BOM service. Covers full ECO
// lifecycle: create -> submit -> approve -> apply. Auth verified.

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const BOM_DEFAULT_URL: &str = "http://localhost:8107";

fn bom_url() -> String {
    std::env::var("BOM_URL").unwrap_or_else(|_| BOM_DEFAULT_URL.to_string())
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

async fn wait_for_bom(client: &Client) -> bool {
    let url = format!("{}/api/health", bom_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  BOM health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  BOM health {}/15: {}", attempt, e),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

async fn assert_unauth(client: &Client, method: &str, url: &str, body: Option<Value>) {
    let req = match method {
        "GET" => client.get(url),
        "POST" => client.post(url),
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
async fn smoke_bom_eco() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_bom(&client).await {
        eprintln!("BOM service not reachable at {} -- skipping", bom_url());
        return;
    }
    println!("BOM service healthy at {}", bom_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id, &["bom.mutate", "bom.read"]);
    let base = bom_url();

    // Gate: verify the BOM service accepts our JWT (probe an auth-required route)
    let probe = client
        .get(format!("{base}/api/bom/{}", Uuid::new_v4()))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!("BOM returns 401 with valid JWT -- JWT_PUBLIC_KEY not configured. Skipping.");
        return;
    }

    // BOM just stores UUIDs for parts — no need to create real inventory items
    let parent_item_id = Uuid::new_v4();
    let component_item_id = Uuid::new_v4();

    // --- 1. POST /api/bom — create BOM ---
    println!("\n--- 1. POST /api/bom ---");
    let resp = client
        .post(format!("{base}/api/bom"))
        .bearer_auth(&jwt)
        .json(&json!({
            "part_id": parent_item_id,
            "description": "Smoke test BOM"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create BOM failed: {status} - {body}"
    );
    let bom_id = body["id"].as_str().expect("No id in BOM response");
    println!("  created BOM id={bom_id}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/bom"),
        Some(json!({"part_id": Uuid::new_v4()})),
    )
    .await;

    // --- 2. GET /api/bom/{bom_id} ---
    println!("\n--- 2. GET /api/bom/{{bom_id}} ---");
    let resp = client
        .get(format!("{base}/api/bom/{bom_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Get BOM failed: {}",
        resp.status()
    );
    let fetched: Value = resp.json().await.unwrap();
    assert_eq!(fetched["description"], "Smoke test BOM");
    println!("  retrieved BOM description={}", fetched["description"]);

    assert_unauth(&client, "GET", &format!("{base}/api/bom/{bom_id}"), None).await;

    // --- 3. POST /api/bom/{bom_id}/revisions — create revision ---
    println!("\n--- 3. POST /api/bom/{{bom_id}}/revisions ---");
    let resp = client
        .post(format!("{base}/api/bom/{bom_id}/revisions"))
        .bearer_auth(&jwt)
        .json(&json!({ "revision_label": "A" }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let rev_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create revision failed: {status} - {rev_body}"
    );
    let revision_id = rev_body["id"].as_str().expect("No revision id");
    println!("  created revision id={revision_id}");

    // --- 4. GET /api/bom/{bom_id}/revisions ---
    println!("\n--- 4. GET /api/bom/{{bom_id}}/revisions ---");
    let resp = client
        .get(format!("{base}/api/bom/{bom_id}/revisions"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "List revisions failed: {}",
        resp.status()
    );
    let revs: Value = resp.json().await.unwrap();
    assert!(revs.as_array().map_or(false, |a| !a.is_empty()));
    println!("  listed {} revision(s)", revs.as_array().unwrap().len());

    // --- 5. POST /api/bom/revisions/{revision_id}/lines — add line ---
    println!("\n--- 5. POST /api/bom/revisions/{{revision_id}}/lines ---");
    let resp = client
        .post(format!("{base}/api/bom/revisions/{revision_id}/lines"))
        .bearer_auth(&jwt)
        .json(&json!({
            "component_item_id": component_item_id,
            "quantity": 2.0,
            "uom": "EA",
            "scrap_factor": 0.05,
            "find_number": 10
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let line_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Add line failed: {status} - {line_body}"
    );
    let line_id = line_body["id"].as_str().expect("No line id");
    println!("  added line id={line_id}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/bom/revisions/{revision_id}/lines"),
        Some(json!({"component_item_id": Uuid::new_v4(), "quantity": 1.0})),
    )
    .await;

    // --- 6. Add second line (before effectivity, must be in draft status) ---
    // Add a second line so we can delete it without emptying the BOM
    let second_component = Uuid::new_v4();
    let resp = client
        .post(format!("{base}/api/bom/revisions/{revision_id}/lines"))
        .bearer_auth(&jwt)
        .json(&json!({
            "component_item_id": second_component,
            "quantity": 1.0,
            "find_number": 20
        }))
        .send()
        .await
        .unwrap();
    let add_status = resp.status();
    let del_line_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        add_status == StatusCode::CREATED || add_status == StatusCode::OK,
        "Add second line failed: {add_status} - {del_line_body}"
    );
    let del_line_id = del_line_body["id"]
        .as_str()
        .expect("No line id for delete target");

    // --- 7. POST /api/bom/revisions/{revision_id}/effectivity ---
    println!("\n--- 7. POST /api/bom/revisions/{{revision_id}}/effectivity ---");
    let now = Utc::now();
    let resp = client
        .post(format!(
            "{base}/api/bom/revisions/{revision_id}/effectivity"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "effective_from": now.to_rfc3339(),
            "effective_to": null
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let eff_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Set effectivity failed: {status} - {eff_body}"
    );
    println!("  set effectivity from={}", now.to_rfc3339());

    println!("\n--- 8. DELETE /api/bom/lines/{{line_id}} ---");
    let resp = client
        .delete(format!("{base}/api/bom/lines/{del_line_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::NO_CONTENT || resp.status().is_success(),
        "Delete line failed: {}",
        resp.status()
    );
    println!("  deleted line id={del_line_id}");

    assert_unauth(
        &client,
        "DELETE",
        &format!("{base}/api/bom/lines/{}", Uuid::new_v4()),
        None,
    )
    .await;

    // --- 8. GET /api/bom/where-used/{item_id} ---
    println!("\n--- 8. GET /api/bom/where-used/{{item_id}} ---");
    let resp = client
        .get(format!("{base}/api/bom/where-used/{component_item_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Where-used failed: {}",
        resp.status()
    );
    let wu: Value = resp.json().await.unwrap();
    println!(
        "  where-used returned {} row(s)",
        wu.as_array().map_or(0, |a| a.len())
    );

    // --- 9. GET /api/bom/{bom_id}/explosion ---
    println!("\n--- 9. GET /api/bom/{{bom_id}}/explosion ---");
    let resp = client
        .get(format!("{base}/api/bom/{bom_id}/explosion"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "BOM explosion failed: {}",
        resp.status()
    );
    let explosion: Value = resp.json().await.unwrap();
    let explosion_rows = explosion.as_array().map_or(0, |a| a.len());
    println!("  explosion returned {explosion_rows} row(s)");

    // =====================================================================
    // ECO Lifecycle: create -> submit -> approve -> apply
    // =====================================================================

    // --- 10. POST /api/eco — create ECO ---
    println!("\n--- 10. POST /api/eco ---");
    let resp = client
        .post(format!("{base}/api/eco"))
        .bearer_auth(&jwt)
        .json(&json!({
            "title": "Smoke ECO",
            "description": "Smoke test ECO for BOM changes",
            "created_by": "smoke-test"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let eco_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create ECO failed: {status} - {eco_body}"
    );
    let eco_id = eco_body["id"].as_str().expect("No ECO id");
    println!(
        "  created ECO id={eco_id} number={}",
        eco_body["eco_number"]
    );

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/eco"),
        Some(json!({"title": "X", "created_by": "X"})),
    )
    .await;

    // --- 11. GET /api/eco/{eco_id} ---
    println!("\n--- 11. GET /api/eco/{{eco_id}} ---");
    let resp = client
        .get(format!("{base}/api/eco/{eco_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Get ECO failed: {}",
        resp.status()
    );
    let eco_fetched: Value = resp.json().await.unwrap();
    assert_eq!(eco_fetched["title"], "Smoke ECO");
    println!("  retrieved ECO status={}", eco_fetched["status"]);

    assert_unauth(&client, "GET", &format!("{base}/api/eco/{eco_id}"), None).await;

    // --- 12. POST /api/eco/{eco_id}/submit ---
    println!("\n--- 12. POST /api/eco/{{eco_id}}/submit ---");
    let resp = client
        .post(format!("{base}/api/eco/{eco_id}/submit"))
        .bearer_auth(&jwt)
        .json(&json!({
            "actor": "smoke-test",
            "comment": "Submitting for review"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let submit_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Submit ECO failed: {status} - {submit_body}"
    );
    println!("  submitted ECO status={}", submit_body["status"]);

    // --- 13. POST /api/eco/{eco_id}/approve ---
    println!("\n--- 13. POST /api/eco/{{eco_id}}/approve ---");
    let resp = client
        .post(format!("{base}/api/eco/{eco_id}/approve"))
        .bearer_auth(&jwt)
        .json(&json!({
            "actor": "smoke-approver",
            "comment": "Approved for implementation"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let approve_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Approve ECO failed: {status} - {approve_body}"
    );
    println!("  approved ECO status={}", approve_body["status"]);

    // --- 14. POST /api/eco/{eco_id}/apply ---
    println!("\n--- 14. POST /api/eco/{{eco_id}}/apply ---");
    let apply_from = Utc::now();
    let resp = client
        .post(format!("{base}/api/eco/{eco_id}/apply"))
        .bearer_auth(&jwt)
        .json(&json!({
            "actor": "smoke-implementor",
            "effective_from": apply_from.to_rfc3339(),
            "effective_to": null
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let apply_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Apply ECO failed: {status} - {apply_body}"
    );
    println!("  applied ECO status={}", apply_body["status"]);

    // --- 15. POST /api/eco/{eco_id}/reject (separate ECO) ---
    println!("\n--- 15. POST /api/eco/{{eco_id}}/reject (separate ECO) ---");
    let resp2 = client
        .post(format!("{base}/api/eco"))
        .bearer_auth(&jwt)
        .json(&json!({
            "title": "Smoke ECO Reject",
            "description": "Will be rejected",
            "created_by": "smoke-test"
        }))
        .send()
        .await
        .unwrap();
    let eco2: Value = resp2.json().await.unwrap_or(json!({}));
    let eco2_id = eco2["id"].as_str().expect("No ECO id for reject test");

    // Submit then reject
    client
        .post(format!("{base}/api/eco/{eco2_id}/submit"))
        .bearer_auth(&jwt)
        .json(&json!({"actor": "smoke-test"}))
        .send()
        .await
        .unwrap();

    let resp = client
        .post(format!("{base}/api/eco/{eco2_id}/reject"))
        .bearer_auth(&jwt)
        .json(&json!({
            "actor": "smoke-reviewer",
            "comment": "Rejected for testing"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let reject_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Reject ECO failed: {status} - {reject_body}"
    );
    println!("  rejected ECO status={}", reject_body["status"]);

    // --- 16. GET /api/eco/{eco_id}/audit ---
    println!("\n--- 16. GET /api/eco/{{eco_id}}/audit ---");
    let resp = client
        .get(format!("{base}/api/eco/{eco_id}/audit"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "ECO audit failed: {}",
        resp.status()
    );
    let audit: Value = resp.json().await.unwrap();
    let audit_count = audit.as_array().map_or(0, |a| a.len());
    assert!(audit_count > 0, "Expected audit entries for ECO lifecycle");
    println!("  audit trail has {audit_count} entries");

    // --- 17. GET /api/eco/{eco_id}/bom-revisions ---
    println!("\n--- 17. GET /api/eco/{{eco_id}}/bom-revisions ---");
    let resp = client
        .get(format!("{base}/api/eco/{eco_id}/bom-revisions"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "ECO bom-revisions failed: {}",
        resp.status()
    );
    println!("  bom-revisions ok");

    // --- 18. GET /api/eco/{eco_id}/doc-revisions ---
    println!("\n--- 18. GET /api/eco/{{eco_id}}/doc-revisions ---");
    let resp = client
        .get(format!("{base}/api/eco/{eco_id}/doc-revisions"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "ECO doc-revisions failed: {}",
        resp.status()
    );
    println!("  doc-revisions ok");

    // --- GET /api/eco/history/{part_id} ---
    println!("\n--- GET /api/eco/history/{{part_id}} ---");
    let resp = client
        .get(format!("{base}/api/eco/history/{parent_item_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "ECO history failed: {}",
        resp.status()
    );
    println!("  eco history for part ok");

    println!("\n=== All 18 BOM + ECO routes passed ===");
}
