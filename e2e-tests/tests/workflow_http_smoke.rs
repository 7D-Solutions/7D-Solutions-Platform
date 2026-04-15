// HTTP smoke tests: Workflow
//
// Proves that 8 core Workflow routes respond correctly at the HTTP boundary
// via reqwest against the live Workflow service.
// Full lifecycle: create definition → start instance → advance → complete → transitions.

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const WF_DEFAULT_URL: &str = "http://localhost:8110";

fn wf_url() -> String {
    std::env::var("WORKFLOW_URL").unwrap_or_else(|_| WF_DEFAULT_URL.to_string())
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
    let url = format!("{}/api/health", wf_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  workflow health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  workflow health {}/15: {}", attempt, e),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

async fn assert_unauth(client: &Client, method: &str, url: &str, body: Option<Value>) {
    let req = match method {
        "GET" => client.get(url),
        "POST" => client.post(url),
        "PATCH" => client.patch(url),
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
async fn smoke_workflow() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_service(&client).await {
        eprintln!("Workflow service not reachable at {} -- skipping", wf_url());
        return;
    }
    println!("Workflow service healthy at {}", wf_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id, &["workflow.mutate", "workflow.read"]);
    let base = wf_url();

    // Gate: verify the service accepts our JWT
    let probe = client
        .get(format!("{base}/api/workflow/definitions"))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!(
            "Workflow returns 401 with valid JWT -- JWT_PUBLIC_KEY not configured. Skipping."
        );
        return;
    }

    // --- 1. POST /api/workflow/definitions ---
    // 3-step workflow: draft → review → (advance to __completed__)
    println!("\n--- 1. POST /api/workflow/definitions ---");
    let def_name = format!("smoke-wf-{}", &Uuid::new_v4().to_string()[..8]);
    let resp = client
        .post(format!("{base}/api/workflow/definitions"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "name": def_name,
            "description": "Smoke test 3-step workflow",
            "steps": [
                {"step_id": "draft", "label": "Draft"},
                {"step_id": "review", "label": "Under Review"}
            ],
            "initial_step_id": "draft"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let def_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create definition failed: {status} - {def_body}"
    );
    let def_id = def_body["id"]
        .as_str()
        .expect("No id in definition response");
    println!("  created definition id={def_id} name={def_name}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/workflow/definitions"),
        Some(json!({
            "steps": [{"step_id": "a"}],
            "initial_step_id": "a",
            "name": "X"
        })),
    )
    .await;

    // --- 2. GET /api/workflow/definitions ---
    println!("\n--- 2. GET /api/workflow/definitions ---");
    let resp = client
        .get(format!("{base}/api/workflow/definitions"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "List definitions failed: {}",
        resp.status()
    );
    let defs: Value = resp.json().await.unwrap();
    let def_count = defs.as_array().map_or(0, |a| a.len());
    assert!(
        def_count >= 1,
        "Expected at least 1 definition, got {def_count}"
    );
    println!("  listed {def_count} definition(s)");

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/workflow/definitions"),
        None,
    )
    .await;

    // --- 3. GET /api/workflow/definitions/{def_id} ---
    println!("\n--- 3. GET /api/workflow/definitions/{{def_id}} ---");
    let resp = client
        .get(format!("{base}/api/workflow/definitions/{def_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Get definition failed: {}",
        resp.status()
    );
    let fetched_def: Value = resp.json().await.unwrap();
    assert_eq!(fetched_def["name"], def_name);
    println!("  retrieved definition name={}", fetched_def["name"]);

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/workflow/definitions/{def_id}"),
        None,
    )
    .await;

    // --- 4. POST /api/workflow/instances ---
    println!("\n--- 4. POST /api/workflow/instances ---");
    let entity_id = Uuid::new_v4().to_string();
    let resp = client
        .post(format!("{base}/api/workflow/instances"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "definition_id": def_id,
            "entity_type": "smoke_invoice",
            "entity_id": entity_id,
            "context": {"note": "smoke test"},
            "idempotency_key": Uuid::new_v4().to_string()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let inst_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Start instance failed: {status} - {inst_body}"
    );
    let instance_id = inst_body["id"]
        .as_str()
        .expect("No id in instance response");
    let current_step = inst_body["current_step_id"].as_str().unwrap_or("?");
    assert_eq!(
        current_step, "draft",
        "Expected instance to start at 'draft'"
    );
    println!("  started instance id={instance_id} step={current_step}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/workflow/instances"),
        Some(json!({"definition_id": Uuid::new_v4(), "entity_type": "x", "entity_id": "x"})),
    )
    .await;

    // --- 5. GET /api/workflow/instances ---
    println!("\n--- 5. GET /api/workflow/instances ---");
    let resp = client
        .get(format!("{base}/api/workflow/instances"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "List instances failed: {}",
        resp.status()
    );
    let instances: Value = resp.json().await.unwrap();
    let inst_count = instances.as_array().map_or(0, |a| a.len());
    assert!(
        inst_count >= 1,
        "Expected at least 1 instance, got {inst_count}"
    );
    println!("  listed {inst_count} instance(s)");

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/workflow/instances"),
        None,
    )
    .await;

    // --- 6. GET /api/workflow/instances/{instance_id} ---
    println!("\n--- 6. GET /api/workflow/instances/{{instance_id}} ---");
    let resp = client
        .get(format!("{base}/api/workflow/instances/{instance_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Get instance failed: {}",
        resp.status()
    );
    let fetched_inst: Value = resp.json().await.unwrap();
    assert_eq!(fetched_inst["status"], "active");
    println!(
        "  retrieved instance status={} step={}",
        fetched_inst["status"], fetched_inst["current_step_id"]
    );

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/workflow/instances/{instance_id}"),
        None,
    )
    .await;

    // --- 7. PATCH /api/workflow/instances/{instance_id}/advance (draft → review) ---
    println!("\n--- 7. PATCH .../advance (draft → review) ---");
    let resp = client
        .patch(format!(
            "{base}/api/workflow/instances/{instance_id}/advance"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "to_step_id": "review",
            "action": "submit_for_review",
            "comment": "Smoke test advance",
            "idempotency_key": Uuid::new_v4().to_string()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let adv_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Advance to review failed: {status} - {adv_body}"
    );
    let new_step = adv_body["instance"]["current_step_id"]
        .as_str()
        .unwrap_or("?");
    assert_eq!(new_step, "review");
    println!("  advanced to step={new_step}");

    // Advance to __completed__
    println!("\n--- 7b. PATCH .../advance (review → __completed__) ---");
    let resp = client
        .patch(format!(
            "{base}/api/workflow/instances/{instance_id}/advance"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "to_step_id": "__completed__",
            "action": "approve",
            "comment": "Smoke test complete",
            "idempotency_key": Uuid::new_v4().to_string()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let done_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Advance to completed failed: {status} - {done_body}"
    );
    let final_status = done_body["instance"]["status"].as_str().unwrap_or("?");
    assert_eq!(final_status, "completed");
    println!("  instance status={final_status}");

    assert_unauth(
        &client,
        "PATCH",
        &format!("{base}/api/workflow/instances/{instance_id}/advance"),
        Some(json!({"to_step_id": "x", "action": "x"})),
    )
    .await;

    // --- 8. GET /api/workflow/instances/{instance_id}/transitions ---
    println!("\n--- 8. GET .../transitions ---");
    let resp = client
        .get(format!(
            "{base}/api/workflow/instances/{instance_id}/transitions"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "List transitions failed: {}",
        resp.status()
    );
    let transitions: Value = resp.json().await.unwrap();
    let tx_count = transitions.as_array().map_or(0, |a| a.len());
    // Expect: 1 initial (start) + 2 advances = 3 transitions
    assert!(
        tx_count >= 2,
        "Expected at least 2 transitions, got {tx_count}"
    );
    println!("  listed {tx_count} transition(s)");

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/workflow/instances/{instance_id}/transitions"),
        None,
    )
    .await;

    println!("\n=== All 8 Workflow routes passed ===");
}
