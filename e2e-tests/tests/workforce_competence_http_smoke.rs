// HTTP smoke tests: Workforce Competence
//
// Proves that 7 core Workforce Competence routes respond correctly at the HTTP
// boundary via reqwest against the live service.
// Full lifecycle: register artifact → assign to operator → check authorization
//                 → grant acceptance authority → check authority → revoke → verify revoked.

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const WC_DEFAULT_URL: &str = "http://localhost:8121";

fn wc_url() -> String {
    std::env::var("WORKFORCE_COMPETENCE_URL").unwrap_or_else(|_| WC_DEFAULT_URL.to_string())
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

async fn wait_for_service(client: &Client) -> bool {
    let url = format!("{}/api/health", wc_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!(
                "  workforce-competence health {}/15: {}",
                attempt,
                r.status()
            ),
            Err(e) => eprintln!("  workforce-competence health {}/15: {}", attempt, e),
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
async fn smoke_workforce_competence() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_service(&client).await {
        eprintln!(
            "Workforce Competence service not reachable at {} -- skipping",
            wc_url()
        );
        return;
    }
    println!("Workforce Competence service healthy at {}", wc_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(
        &key,
        &tenant_id,
        &["workforce_competence.mutate", "workforce_competence.read"],
    );
    let base = wc_url();

    // Gate: verify the service accepts our JWT
    let probe = client
        .get(format!("{base}/api/health"))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!("Workforce Competence returns 401 with valid JWT -- JWT_PUBLIC_KEY not configured. Skipping.");
        return;
    }

    let now = Utc::now();
    let operator_id = Uuid::new_v4();
    let artifact_code = format!("WLD-{}", &Uuid::new_v4().to_string()[..8]);

    // --- 1. POST /api/workforce-competence/artifacts ---
    println!("\n--- 1. POST /api/workforce-competence/artifacts ---");
    let resp = client
        .post(format!("{base}/api/workforce-competence/artifacts"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "artifact_type": "certification",
            "name": "Smoke Test Welding Cert",
            "code": artifact_code,
            "description": "AWS D1.1 Structural Welding",
            "valid_duration_days": 365,
            "idempotency_key": Uuid::new_v4().to_string()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let artifact_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Register artifact failed: {status} - {artifact_body}"
    );
    let artifact_id = artifact_body["id"]
        .as_str()
        .expect("No id in artifact response");
    println!("  registered artifact id={artifact_id} code={artifact_code}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/workforce-competence/artifacts"),
        Some(json!({
            "artifact_type": "certification",
            "name": "X",
            "code": "X",
            "idempotency_key": Uuid::new_v4().to_string()
        })),
    )
    .await;

    // --- 2. GET /api/workforce-competence/artifacts/{id} ---
    println!("\n--- 2. GET /api/workforce-competence/artifacts/{{id}} ---");
    let resp = client
        .get(format!(
            "{base}/api/workforce-competence/artifacts/{artifact_id}"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Get artifact failed: {}",
        resp.status()
    );
    let fetched: Value = resp.json().await.unwrap();
    assert_eq!(fetched["code"], artifact_code);
    println!("  retrieved artifact code={}", fetched["code"]);

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/workforce-competence/artifacts/{artifact_id}"),
        None,
    )
    .await;

    // --- 3. POST /api/workforce-competence/assignments ---
    println!("\n--- 3. POST /api/workforce-competence/assignments ---");
    let resp = client
        .post(format!("{base}/api/workforce-competence/assignments"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "operator_id": operator_id,
            "artifact_id": artifact_id,
            "awarded_at": now.to_rfc3339(),
            "evidence_ref": "training-record-001",
            "awarded_by": "smoke-test",
            "idempotency_key": Uuid::new_v4().to_string()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let assign_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Assign competence failed: {status} - {assign_body}"
    );
    println!("  assigned competence operator_id={operator_id} artifact_id={artifact_id}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/workforce-competence/assignments"),
        Some(json!({
            "operator_id": Uuid::new_v4(),
            "artifact_id": Uuid::new_v4(),
            "awarded_at": now.to_rfc3339(),
            "idempotency_key": Uuid::new_v4().to_string()
        })),
    )
    .await;

    // --- 4. GET /api/workforce-competence/authorization ---
    println!("\n--- 4. GET /api/workforce-competence/authorization ---");
    let at_time = now.to_rfc3339();
    let resp = client
        .get(format!("{base}/api/workforce-competence/authorization"))
        .bearer_auth(&jwt)
        .query(&[
            ("operator_id", operator_id.to_string()),
            ("artifact_code", artifact_code.clone()),
            ("at_time", at_time.clone()),
        ])
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Authorization check failed: {}",
        resp.status()
    );
    let auth_result: Value = resp.json().await.unwrap();
    let authorized = auth_result["authorized"].as_bool().unwrap_or(false);
    assert!(
        authorized,
        "Expected operator to be authorized after assignment"
    );
    println!("  authorization check: authorized={authorized}");

    // --- 5. POST /api/workforce-competence/acceptance-authorities ---
    println!("\n--- 5. POST /api/workforce-competence/acceptance-authorities ---");
    let capability_scope = format!("weld.{}", &Uuid::new_v4().to_string()[..8]);
    let resp = client
        .post(format!(
            "{base}/api/workforce-competence/acceptance-authorities"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "operator_id": operator_id,
            "capability_scope": capability_scope,
            "effective_from": now.to_rfc3339(),
            "granted_by": "smoke-test",
            "idempotency_key": Uuid::new_v4().to_string()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let aa_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Grant authority failed: {status} - {aa_body}"
    );
    let authority_id = aa_body["id"].as_str().expect("No id in authority response");
    println!("  granted authority id={authority_id} scope={capability_scope}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/workforce-competence/acceptance-authorities"),
        Some(json!({
            "operator_id": Uuid::new_v4(),
            "capability_scope": "X",
            "effective_from": now.to_rfc3339(),
            "idempotency_key": Uuid::new_v4().to_string()
        })),
    )
    .await;

    // --- 6. GET /api/workforce-competence/acceptance-authority-check ---
    println!("\n--- 6. GET /api/workforce-competence/acceptance-authority-check ---");
    let resp = client
        .get(format!(
            "{base}/api/workforce-competence/acceptance-authority-check"
        ))
        .bearer_auth(&jwt)
        .query(&[
            ("operator_id", operator_id.to_string()),
            ("capability_scope", capability_scope.clone()),
            ("at_time", at_time.clone()),
        ])
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Acceptance authority check failed: {}",
        resp.status()
    );
    let aa_result: Value = resp.json().await.unwrap();
    let allowed = aa_result["allowed"].as_bool().unwrap_or(false);
    assert!(
        allowed,
        "Expected acceptance authority check to return allowed=true"
    );
    println!("  acceptance authority check: allowed={allowed}");

    // --- 7. POST .../acceptance-authorities/{id}/revoke ---
    println!("\n--- 7. POST .../acceptance-authorities/{{id}}/revoke ---");
    let resp = client
        .post(format!(
            "{base}/api/workforce-competence/acceptance-authorities/{authority_id}/revoke"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "authority_id": authority_id,
            "revocation_reason": "Smoke test revocation",
            "idempotency_key": Uuid::new_v4().to_string()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let revoke_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Revoke authority failed: {status} - {revoke_body}"
    );
    let is_revoked = revoke_body["is_revoked"].as_bool().unwrap_or(false);
    assert!(is_revoked, "Expected is_revoked=true after revocation");
    println!("  revoked authority is_revoked={is_revoked}");

    assert_unauth(
        &client,
        "POST",
        &format!(
            "{base}/api/workforce-competence/acceptance-authorities/{authority_id}/revoke"
        ),
        Some(json!({"authority_id": Uuid::new_v4(), "revocation_reason": "X", "idempotency_key": Uuid::new_v4().to_string()})),
    )
    .await;

    println!("\n=== All 7 Workforce Competence routes passed ===");
}
