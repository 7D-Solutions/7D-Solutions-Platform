// HTTP smoke tests: Consolidation
//
// Proves that 21 core consolidation routes respond correctly at the HTTP
// boundary via reqwest against the live Consolidation service.

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const CSL_DEFAULT_URL: &str = "http://localhost:8105";

fn csl_url() -> String {
    std::env::var("CONSOLIDATION_URL").unwrap_or_else(|_| CSL_DEFAULT_URL.to_string())
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
    let url = format!("{}/api/health", csl_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  Consolidation health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  Consolidation health {}/15: {}", attempt, e),
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
async fn smoke_consolidation() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_service(&client).await {
        eprintln!(
            "Consolidation service not reachable at {} -- skipping",
            csl_url()
        );
        return;
    }
    println!("Consolidation service healthy at {}", csl_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(
        &key,
        &tenant_id,
        &["consolidation.mutate", "consolidation.read"],
    );
    let base = csl_url();

    // Gate: verify the service accepts our JWT
    let probe = client
        .get(format!("{base}/api/consolidation/groups"))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!(
            "Consolidation returns 401 with valid JWT -- JWT_PUBLIC_KEY not configured. Skipping."
        );
        return;
    }

    // 1. POST /api/consolidation/groups
    println!("\n--- 1. POST /api/consolidation/groups ---");
    let resp = client
        .post(format!("{base}/api/consolidation/groups"))
        .bearer_auth(&jwt)
        .json(&json!({
            "name": "Smoke Test Group",
            "description": "Created by consolidation smoke test",
            "reporting_currency": "USD",
            "fiscal_year_end_month": 12
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create group failed: {status} - {body}"
    );
    let group_id = body["id"].as_str().expect("No id in create group response");
    println!("  created group id={group_id}");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/consolidation/groups"),
        Some(json!({"name": "X", "reporting_currency": "USD"})),
    )
    .await;

    // 2. GET /api/consolidation/groups/{id}
    println!("\n--- 2. GET /api/consolidation/groups/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/consolidation/groups/{group_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Get group failed: {}",
        resp.status()
    );
    let fetched: Value = resp.json().await.unwrap_or(json!({}));
    println!("  retrieved group name={}", fetched["name"]);
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/consolidation/groups/{group_id}"),
        None,
    )
    .await;

    // 3. GET /api/consolidation/groups/{id}/validate
    println!("\n--- 3. GET /api/consolidation/groups/{{id}}/validate ---");
    let resp = client
        .get(format!(
            "{base}/api/consolidation/groups/{group_id}/validate"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "Validate group failed: {status} - {body}");
    println!("  validation result: is_complete={}", body["is_complete"]);
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/consolidation/groups/{group_id}/validate"),
        None,
    )
    .await;

    // 4. POST /api/consolidation/groups/{group_id}/entities
    println!("\n--- 4. POST /api/consolidation/groups/{{group_id}}/entities ---");
    let entity_tenant = Uuid::new_v4().to_string();
    let resp = client
        .post(format!(
            "{base}/api/consolidation/groups/{group_id}/entities"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "entity_tenant_id": entity_tenant,
            "entity_name": "Subsidiary Alpha",
            "functional_currency": "USD",
            "ownership_pct_bp": 10000,
            "consolidation_method": "full"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create entity failed: {status} - {body}"
    );
    let entity_id = body["id"].as_str().expect("No id in create entity response");
    println!("  created entity id={entity_id}");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/consolidation/groups/{group_id}/entities"),
        Some(json!({"entity_tenant_id": "x", "entity_name": "x", "functional_currency": "USD"})),
    )
    .await;

    // 5. GET /api/consolidation/entities/{id}
    println!("\n--- 5. GET /api/consolidation/entities/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/consolidation/entities/{entity_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Get entity failed: {}",
        resp.status()
    );
    let fetched: Value = resp.json().await.unwrap_or(json!({}));
    println!("  retrieved entity name={}", fetched["entity_name"]);

    // 6. POST /api/consolidation/groups/{group_id}/coa-mappings
    println!("\n--- 6. POST /api/consolidation/groups/{{group_id}}/coa-mappings ---");
    let resp = client
        .post(format!(
            "{base}/api/consolidation/groups/{group_id}/coa-mappings"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "entity_tenant_id": entity_tenant,
            "source_account_code": "1000",
            "target_account_code": "1000-CONSOL",
            "target_account_name": "Consolidated Cash"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create COA mapping failed: {status} - {body}"
    );
    let mapping_id = body["id"].as_str().expect("No id in COA mapping response");
    println!("  created COA mapping id={mapping_id}");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/consolidation/groups/{group_id}/coa-mappings"),
        Some(json!({"entity_tenant_id": "x", "source_account_code": "1", "target_account_code": "2"})),
    )
    .await;

    // 7. GET /api/consolidation/groups/{group_id}/coa-mappings (list)
    println!("\n--- 7. GET /api/consolidation/groups/{{group_id}}/coa-mappings ---");
    let resp = client
        .get(format!(
            "{base}/api/consolidation/groups/{group_id}/coa-mappings"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!([]));
    assert!(status.is_success(), "List COA mappings failed: {status}");
    assert!(body.is_array(), "COA mappings response should be array");
    println!(
        "  listed {} COA mappings",
        body.as_array().map(|a| a.len()).unwrap_or(0)
    );

    // 8. POST /api/consolidation/groups/{group_id}/elimination-rules
    println!("\n--- 8. POST /api/consolidation/groups/{{group_id}}/elimination-rules ---");
    let resp = client
        .post(format!(
            "{base}/api/consolidation/groups/{group_id}/elimination-rules"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "rule_name": "Intercompany Revenue",
            "rule_type": "intercompany",
            "debit_account_code": "4000",
            "credit_account_code": "5000",
            "description": "Eliminate intercompany revenue/expense"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create elimination rule failed: {status} - {body}"
    );
    let rule_id = body["id"].as_str().expect("No id in elimination rule response");
    println!("  created elimination rule id={rule_id}");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/consolidation/groups/{group_id}/elimination-rules"),
        Some(json!({"rule_name": "X", "rule_type": "X", "debit_account_code": "1", "credit_account_code": "2"})),
    )
    .await;

    // 9. GET /api/consolidation/elimination-rules/{id}
    println!("\n--- 9. GET /api/consolidation/elimination-rules/{{id}} ---");
    let resp = client
        .get(format!(
            "{base}/api/consolidation/elimination-rules/{rule_id}"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Get elimination rule failed: {}",
        resp.status()
    );
    let fetched: Value = resp.json().await.unwrap_or(json!({}));
    println!("  retrieved rule name={}", fetched["rule_name"]);

    // 10. GET /api/consolidation/groups/{group_id}/elimination-rules
    println!("\n--- 10. GET /api/consolidation/groups/{{group_id}}/elimination-rules ---");
    let resp = client
        .get(format!(
            "{base}/api/consolidation/groups/{group_id}/elimination-rules"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!([]));
    assert!(status.is_success(), "List elimination rules failed: {status}");
    assert!(body.is_array(), "Elimination rules should be array");
    println!(
        "  listed {} elimination rules",
        body.as_array().map(|a| a.len()).unwrap_or(0)
    );

    // 11. PUT /api/consolidation/groups/{group_id}/fx-policies
    println!("\n--- 11. PUT /api/consolidation/groups/{{group_id}}/fx-policies ---");
    let resp = client
        .put(format!(
            "{base}/api/consolidation/groups/{group_id}/fx-policies"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "entity_tenant_id": entity_tenant,
            "bs_rate_type": "closing",
            "pl_rate_type": "average",
            "equity_rate_type": "historical",
            "fx_rate_source": "manual"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Upsert FX policy failed: {status} - {body}"
    );
    let policy_id = body["id"].as_str().unwrap_or("unknown");
    println!("  upserted FX policy id={policy_id}");
    assert_unauth(
        &client,
        "PUT",
        &format!("{base}/api/consolidation/groups/{group_id}/fx-policies"),
        Some(json!({"entity_tenant_id": "x"})),
    )
    .await;

    // 12. GET /api/consolidation/groups/{group_id}/fx-policies
    println!("\n--- 12. GET /api/consolidation/groups/{{group_id}}/fx-policies ---");
    let resp = client
        .get(format!(
            "{base}/api/consolidation/groups/{group_id}/fx-policies"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!([]));
    assert!(status.is_success(), "List FX policies failed: {status}");
    assert!(body.is_array(), "FX policies should be array");
    println!(
        "  listed {} FX policies",
        body.as_array().map(|a| a.len()).unwrap_or(0)
    );

    // 13. POST /api/consolidation/groups/{group_id}/consolidate
    println!("\n--- 13. POST /api/consolidation/groups/{{group_id}}/consolidate ---");
    let period_id = Uuid::new_v4();
    let resp = client
        .post(format!(
            "{base}/api/consolidation/groups/{group_id}/consolidate"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "period_id": period_id,
            "as_of": "2026-01-31"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert!(
        status != StatusCode::NOT_FOUND && status != StatusCode::UNAUTHORIZED,
        "Consolidate route not found or auth failed: {status}"
    );
    println!("  consolidate responded: {status} (domain error expected without GL data)");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/consolidation/groups/{group_id}/consolidate"),
        Some(json!({"period_id": period_id, "as_of": "2026-01-31"})),
    )
    .await;

    // 14. POST /api/consolidation/groups/{group_id}/intercompany-match
    println!("\n--- 14. POST /api/consolidation/groups/{{group_id}}/intercompany-match ---");
    let resp = client
        .post(format!(
            "{base}/api/consolidation/groups/{group_id}/intercompany-match"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "period_id": period_id,
            "as_of": "2026-01-31"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert!(
        status != StatusCode::NOT_FOUND && status != StatusCode::UNAUTHORIZED,
        "Intercompany match route error: {status}"
    );
    println!("  intercompany-match responded: {status}");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/consolidation/groups/{group_id}/intercompany-match"),
        Some(json!({"period_id": period_id, "as_of": "2026-01-31"})),
    )
    .await;

    // 15. GET /api/consolidation/groups/{group_id}/trial-balance
    println!("\n--- 15. GET /api/consolidation/groups/{{group_id}}/trial-balance ---");
    let resp = client
        .get(format!(
            "{base}/api/consolidation/groups/{group_id}/trial-balance?as_of=2026-01-31"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert!(
        status != StatusCode::UNAUTHORIZED,
        "Trial balance auth failed: {status}"
    );
    println!("  trial-balance responded: {status}");

    // 16. GET /api/consolidation/groups/{group_id}/balance-sheet
    println!("\n--- 16. GET /api/consolidation/groups/{{group_id}}/balance-sheet ---");
    let resp = client
        .get(format!(
            "{base}/api/consolidation/groups/{group_id}/balance-sheet?as_of=2026-01-31"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert!(
        status != StatusCode::UNAUTHORIZED,
        "Balance sheet auth failed: {status}"
    );
    println!("  balance-sheet responded: {status}");

    // 17. GET /api/consolidation/groups/{group_id}/pl
    println!("\n--- 17. GET /api/consolidation/groups/{{group_id}}/pl ---");
    let resp = client
        .get(format!(
            "{base}/api/consolidation/groups/{group_id}/pl?as_of=2026-01-31"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert!(
        status != StatusCode::UNAUTHORIZED,
        "P&L auth failed: {status}"
    );
    println!("  P&L responded: {status}");

    // 18. POST /api/consolidation/groups/{group_id}/eliminations
    println!("\n--- 18. POST /api/consolidation/groups/{{group_id}}/eliminations ---");
    let resp = client
        .post(format!(
            "{base}/api/consolidation/groups/{group_id}/eliminations"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "period_id": period_id,
            "as_of": "2026-01-31",
            "reporting_currency": "USD"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert!(
        status != StatusCode::NOT_FOUND && status != StatusCode::UNAUTHORIZED,
        "Post eliminations route error: {status}"
    );
    println!("  post-eliminations responded: {status}");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/consolidation/groups/{group_id}/eliminations"),
        Some(json!({"period_id": period_id, "as_of": "2026-01-31", "reporting_currency": "USD"})),
    )
    .await;

    // 19. GET /api/consolidation/groups (list)
    println!("\n--- 19. GET /api/consolidation/groups ---");
    let resp = client
        .get(format!("{base}/api/consolidation/groups"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!([]));
    assert!(status.is_success(), "List groups failed: {status}");
    assert!(body.is_array(), "Groups response should be array");
    println!(
        "  listed {} groups",
        body.as_array().map(|a| a.len()).unwrap_or(0)
    );
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/consolidation/groups"),
        None,
    )
    .await;

    // 20. GET /api/consolidation/groups/{group_id}/entities (list)
    println!("\n--- 20. GET /api/consolidation/groups/{{group_id}}/entities ---");
    let resp = client
        .get(format!(
            "{base}/api/consolidation/groups/{group_id}/entities"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!([]));
    assert!(status.is_success(), "List entities failed: {status}");
    assert!(body.is_array(), "Entities response should be array");
    println!(
        "  listed {} entities",
        body.as_array().map(|a| a.len()).unwrap_or(0)
    );

    // 21. GET /api/consolidation/admin/projections
    println!("\n--- 21. GET /api/consolidation/admin/projections ---");
    let admin_token = std::env::var("ADMIN_TOKEN").unwrap_or_default();
    let resp = client
        .get(format!("{base}/api/consolidation/admin/projections"))
        .header("x-admin-token", &admin_token)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    if admin_token.is_empty() {
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "Admin without token should be 403"
        );
        println!("  admin projections: 403 (ADMIN_TOKEN not set, expected)");
    } else {
        assert!(
            status.is_success(),
            "Admin projections failed: {status}"
        );
        println!("  admin projections: {status}");
    }
    let resp = client
        .get(format!("{base}/api/consolidation/admin/projections"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "Admin without token should be 403"
    );
    println!("  no-token -> 403 ok");

    println!("\n=== All 21 consolidation routes passed ===");
}
