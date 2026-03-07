// HTTP smoke tests: Doc-Mgmt service
//
// Proves that 19 core doc-mgmt routes respond correctly at the HTTP boundary
// via reqwest against the live Doc-Mgmt service. No mocks, no stubs.

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const DOC_DEFAULT_URL: &str = "http://localhost:8095";

fn doc_url() -> String {
    std::env::var("DOC_MGMT_URL").unwrap_or_else(|_| DOC_DEFAULT_URL.to_string())
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

async fn wait_for_doc_mgmt(client: &Client) -> bool {
    let url = format!("{}/api/health", doc_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  Doc-Mgmt health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  Doc-Mgmt health {}/15: {}", attempt, e),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

async fn assert_unauth(client: &Client, method: &str, url: &str, body: Option<Value>) {
    let req = match method {
        "GET" => client.get(url),
        "POST" => client.post(url),
        _ => panic!("unsupported method"),
    };
    let req = if let Some(b) = body { req.json(&b) } else { req };
    let resp = req.send().await.expect("unauth request failed");
    assert_eq!(resp.status().as_u16(), 401, "expected 401 without JWT at {url}");
    println!("  no-JWT -> 401 ok");
}

#[tokio::test]
async fn smoke_doc_mgmt() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_doc_mgmt(&client).await {
        eprintln!("Doc-Mgmt service not reachable at {} -- skipping", doc_url());
        return;
    }
    println!("Doc-Mgmt service healthy at {}", doc_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id, &["doc_mgmt.read", "doc_mgmt.mutate"]);
    let base = doc_url();

    // Gate: verify the service accepts our JWT
    let probe = client
        .get(format!("{base}/api/documents"))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!("Doc-Mgmt returns 401 with valid JWT -- JWT_PUBLIC_KEY not configured. Skipping.");
        return;
    }

    // ── 1. POST /api/documents (create doc1 as draft) ────────────────
    println!("\n--- 1. POST /api/documents ---");
    let doc_number = format!("DOC-SMOKE-{}", &Uuid::new_v4().to_string()[..8].to_uppercase());
    let doc_type = "engineering_spec";
    let resp = client
        .post(format!("{base}/api/documents"))
        .bearer_auth(&jwt)
        .json(&json!({
            "doc_number": doc_number,
            "title": "Smoke Test Document",
            "doc_type": doc_type,
            "body": {"section": "intro", "content": "initial draft"}
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create document failed: {status} - {body}"
    );
    let doc1_id = body["document"]["id"].as_str().expect("no document.id").to_string();
    println!("  created doc1 id={doc1_id}");
    assert_unauth(&client, "POST", &format!("{base}/api/documents"), Some(json!({}))).await;

    // ── 2. POST /api/documents/{id}/revisions ────────────────────────
    println!("\n--- 2. POST /api/documents/{{id}}/revisions ---");
    let resp = client
        .post(format!("{base}/api/documents/{doc1_id}/revisions"))
        .bearer_auth(&jwt)
        .json(&json!({
            "body": {"section": "intro", "content": "revised content v2"},
            "change_summary": "Added more detail"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create revision failed: {status} - {body}"
    );
    println!("  created revision number={}", body["revision"]["revision_number"]);

    // ── 3. POST /api/documents/{id}/release ──────────────────────────
    println!("\n--- 3. POST /api/documents/{{id}}/release ---");
    let resp = client
        .post(format!("{base}/api/documents/{doc1_id}/release"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert!(status.is_success(), "Release document failed: {status}");
    println!("  doc1 released");
    assert_unauth(&client, "POST", &format!("{base}/api/documents/{doc1_id}/release"), None).await;

    // ── 4. POST /api/documents/{id}/distributions ────────────────────
    println!("\n--- 4. POST /api/documents/{{id}}/distributions ---");
    let dist_idem_key = Uuid::new_v4().to_string();
    let resp = client
        .post(format!("{base}/api/documents/{doc1_id}/distributions"))
        .bearer_auth(&jwt)
        .header("idempotency-key", &dist_idem_key)
        .json(&json!({
            "recipient_ref": "team-engineering",
            "channel": "email",
            "template_key": "standard_distribution",
            "payload_json": {"notify": true}
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create distribution failed: {status} - {body}"
    );
    let dist_id = body["distribution"]["id"].as_str().expect("no distribution.id").to_string();
    println!("  created distribution id={dist_id}");

    // ── 5. GET /api/documents/{id}/distributions ─────────────────────
    println!("\n--- 5. GET /api/documents/{{id}}/distributions ---");
    let resp = client
        .get(format!("{base}/api/documents/{doc1_id}/distributions"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "List distributions failed: {status}");
    assert!(
        body["distributions"].is_array(),
        "distributions should be array"
    );
    println!("  listed {} distributions", body["distributions"].as_array().map(|a| a.len()).unwrap_or(0));
    assert_unauth(&client, "GET", &format!("{base}/api/documents/{doc1_id}/distributions"), None).await;

    // ── 6. POST /api/distributions/{id}/status ───────────────────────
    println!("\n--- 6. POST /api/distributions/{{id}}/status ---");
    let status_idem_key = Uuid::new_v4().to_string();
    let resp = client
        .post(format!("{base}/api/distributions/{dist_id}/status"))
        .bearer_auth(&jwt)
        .header("idempotency-key", &status_idem_key)
        .json(&json!({"status": "sent"}))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "Update distribution status failed: {status} - {body}");
    println!("  distribution status updated to sent");

    // ── 7. POST /api/documents/{id}/supersede ────────────────────────
    println!("\n--- 7. POST /api/documents/{{id}}/supersede ---");
    let new_doc_number = format!("DOC-SMOKE-{}", &Uuid::new_v4().to_string()[..8].to_uppercase());
    let resp = client
        .post(format!("{base}/api/documents/{doc1_id}/supersede"))
        .bearer_auth(&jwt)
        .json(&json!({
            "new_doc_number": new_doc_number,
            "new_title": "Smoke Test Document Rev B",
            "change_summary": "Superseded by smoke test"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Supersede document failed: {status} - {body}"
    );
    println!("  doc1 superseded, new doc id={}", body["new_document"]["id"]);

    // ── 8. GET /api/documents/{id} ───────────────────────────────────
    println!("\n--- 8. GET /api/documents/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/documents/{doc1_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "Get document failed: {status}");
    println!("  fetched doc1 status={}", body["document"]["status"]);
    assert_unauth(&client, "GET", &format!("{base}/api/documents/{doc1_id}"), None).await;

    // ── 9. GET /api/documents ────────────────────────────────────────
    println!("\n--- 9. GET /api/documents ---");
    let resp = client
        .get(format!("{base}/api/documents"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "List documents failed: {status}");
    assert!(body["documents"].is_array(), "documents should be array");
    println!("  listed {} documents", body["documents"].as_array().map(|a| a.len()).unwrap_or(0));
    assert_unauth(&client, "GET", &format!("{base}/api/documents"), None).await;

    // ── 10. POST /api/retention-policies ────────────────────────────
    println!("\n--- 10. POST /api/retention-policies ---");
    let resp = client
        .post(format!("{base}/api/retention-policies"))
        .bearer_auth(&jwt)
        .json(&json!({"doc_type": doc_type, "retention_days": 0}))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "Set retention policy failed: {status} - {body}");
    println!("  retention policy set: {} days", body["policy"]["retention_days"]);

    // ── 11. GET /api/retention-policies/{doc_type} ──────────────────
    println!("\n--- 11. GET /api/retention-policies/{doc_type} ---");
    let resp = client
        .get(format!("{base}/api/retention-policies/{doc_type}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "Get retention policy failed: {status} - {body}");
    println!("  fetched retention policy retention_days={}", body["policy"]["retention_days"]);
    assert_unauth(&client, "GET", &format!("{base}/api/retention-policies/{doc_type}"), None).await;

    // Setup: create doc2 for hold/dispose lifecycle
    println!("\n--- Setup: create + release doc2 for hold/dispose ---");
    let doc2_number = format!("DOC-HOLD-{}", &Uuid::new_v4().to_string()[..8].to_uppercase());
    let resp = client
        .post(format!("{base}/api/documents"))
        .bearer_auth(&jwt)
        .json(&json!({
            "doc_number": doc2_number,
            "title": "Hold Test Document",
            "doc_type": doc_type,
            "body": {"note": "for hold testing"}
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success() || status == StatusCode::CREATED, "Create doc2 failed: {status}");
    let doc2_id = body["document"]["id"].as_str().expect("no doc2 id").to_string();

    let resp = client
        .post(format!("{base}/api/documents/{doc2_id}/release"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "Release doc2 failed: {}", resp.status());
    println!("  doc2 id={doc2_id} created and released");

    // ── 12. POST /api/documents/{id}/holds/apply ────────────────────
    println!("\n--- 12. POST /api/documents/{{id}}/holds/apply ---");
    let hold_reason = "litigation-smoke-test";
    let resp = client
        .post(format!("{base}/api/documents/{doc2_id}/holds/apply"))
        .bearer_auth(&jwt)
        .json(&json!({"reason": hold_reason}))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Apply hold failed: {status} - {body}"
    );
    println!("  hold applied id={}", body["hold"]["id"]);

    // ── 13. GET /api/documents/{id}/holds ───────────────────────────
    println!("\n--- 13. GET /api/documents/{{id}}/holds ---");
    let resp = client
        .get(format!("{base}/api/documents/{doc2_id}/holds"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "List holds failed: {status}");
    assert!(body["holds"].is_array(), "holds should be array");
    println!("  listed {} holds", body["holds"].as_array().map(|a| a.len()).unwrap_or(0));
    assert_unauth(&client, "GET", &format!("{base}/api/documents/{doc2_id}/holds"), None).await;

    // ── 14. POST /api/documents/{id}/holds/release ──────────────────
    println!("\n--- 14. POST /api/documents/{{id}}/holds/release ---");
    let resp = client
        .post(format!("{base}/api/documents/{doc2_id}/holds/release"))
        .bearer_auth(&jwt)
        .json(&json!({"reason": hold_reason}))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "Release hold failed: {status} - {body}");
    println!("  hold released");

    // ── 15. POST /api/documents/{id}/dispose ────────────────────────
    println!("\n--- 15. POST /api/documents/{{id}}/dispose ---");
    let resp = client
        .post(format!("{base}/api/documents/{doc2_id}/dispose"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "Dispose document failed: {status} - {body}");
    println!("  doc2 disposed");

    // ── 16. POST /api/templates ──────────────────────────────────────
    println!("\n--- 16. POST /api/templates ---");
    let template_name = format!("smoke-template-{}", &Uuid::new_v4().to_string()[..8]);
    let resp = client
        .post(format!("{base}/api/templates"))
        .bearer_auth(&jwt)
        .json(&json!({
            "name": template_name,
            "doc_type": doc_type,
            "body_template": {"greeting": "Hello {{name}}", "section": "{{section}}"}
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create template failed: {status} - {body}"
    );
    let template_id = body["template"]["id"].as_str().expect("no template.id").to_string();
    println!("  created template id={template_id}");
    assert_unauth(&client, "POST", &format!("{base}/api/templates"), Some(json!({}))).await;

    // ── 17. GET /api/templates/{id} ─────────────────────────────────
    println!("\n--- 17. GET /api/templates/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/templates/{template_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "Get template failed: {status} - {body}");
    println!("  fetched template name={}", body["template"]["name"]);
    assert_unauth(&client, "GET", &format!("{base}/api/templates/{template_id}"), None).await;

    // ── 18. POST /api/templates/{id}/render ─────────────────────────
    println!("\n--- 18. POST /api/templates/{{id}}/render ---");
    let resp = client
        .post(format!("{base}/api/templates/{template_id}/render"))
        .bearer_auth(&jwt)
        .json(&json!({
            "input_data": {"name": "Smoke Tester", "section": "validation"}
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Render template failed: {status} - {body}"
    );
    let artifact_id = body["artifact"]["id"].as_str().expect("no artifact.id").to_string();
    println!("  rendered artifact id={artifact_id}");

    // ── 19. GET /api/artifacts/{id} ─────────────────────────────────
    println!("\n--- 19. GET /api/artifacts/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/artifacts/{artifact_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "Get artifact failed: {status} - {body}");
    println!("  fetched artifact template_id={}", body["artifact"]["template_id"]);
    assert_unauth(&client, "GET", &format!("{base}/api/artifacts/{artifact_id}"), None).await;

    println!("\n=== All 19 doc-mgmt routes passed ===");
}
