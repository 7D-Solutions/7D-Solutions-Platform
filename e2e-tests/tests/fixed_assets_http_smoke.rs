// HTTP smoke tests: Fixed Assets
//
// Proves that 17 core Fixed Assets routes respond correctly at the HTTP
// boundary via reqwest against the live Fixed Assets service.
// Full lifecycle: categories → assets → depreciation schedule + run → disposals.

use chrono::{NaiveDate, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const FA_DEFAULT_URL: &str = "http://localhost:8104";

fn fa_url() -> String {
    std::env::var("FIXED_ASSETS_URL").unwrap_or_else(|_| FA_DEFAULT_URL.to_string())
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
    let url = format!("{}/api/health", fa_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  fixed-assets health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  fixed-assets health {}/15: {}", attempt, e),
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
async fn smoke_fixed_assets() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_service(&client).await {
        eprintln!(
            "Fixed Assets service not reachable at {} -- skipping",
            fa_url()
        );
        return;
    }
    println!("Fixed Assets service healthy at {}", fa_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(
        &key,
        &tenant_id,
        &["fixed_assets.mutate", "fixed_assets.read"],
    );
    let base = fa_url();

    // Gate: verify the service accepts our JWT
    let probe = client
        .get(format!("{base}/api/health"))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!(
            "Fixed Assets returns 401 with valid JWT -- JWT_PUBLIC_KEY not configured. Skipping."
        );
        return;
    }

    let today = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let in_service = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();

    // --- 1. POST /api/fixed-assets/categories ---
    println!("\n--- 1. POST /api/fixed-assets/categories ---");
    let resp = client
        .post(format!("{base}/api/fixed-assets/categories"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "code": format!("CAT-{}", &Uuid::new_v4().to_string()[..8]),
            "name": "Smoke Test Machinery",
            "asset_account_ref": "1500",
            "depreciation_expense_ref": "6100",
            "accum_depreciation_ref": "1510"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let cat_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create category failed: {status} - {cat_body}"
    );
    let cat_id = cat_body["id"].as_str().expect("No id in category response");
    println!("  created category id={cat_id}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/fixed-assets/categories"),
        Some(
            json!({"code": "X", "name": "X", "asset_account_ref": "1500",
                    "depreciation_expense_ref": "6100", "accum_depreciation_ref": "1510"}),
        ),
    )
    .await;

    // Create a second category for deactivation test
    println!("\n--- 1b. POST /api/fixed-assets/categories (cat2 for deactivation) ---");
    let resp = client
        .post(format!("{base}/api/fixed-assets/categories"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "code": format!("CAT2-{}", &Uuid::new_v4().to_string()[..8]),
            "name": "Smoke Test Vehicles",
            "asset_account_ref": "1520",
            "depreciation_expense_ref": "6110",
            "accum_depreciation_ref": "1525"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let cat2_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create category2 failed: {status} - {cat2_body}"
    );
    let cat2_id = cat2_body["id"]
        .as_str()
        .expect("No id in category2 response");
    println!("  created category2 id={cat2_id}");

    // --- 2. GET /api/fixed-assets/categories ---
    println!("\n--- 2. GET /api/fixed-assets/categories ---");
    let resp = client
        .get(format!("{base}/api/fixed-assets/categories"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "List categories failed: {}",
        resp.status()
    );
    let cats: Value = resp.json().await.unwrap();
    let cat_count = cats.as_array().map_or(0, |a| a.len());
    assert!(
        cat_count >= 2,
        "Expected at least 2 categories, got {cat_count}"
    );
    println!("  listed {cat_count} category(ies)");

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/fixed-assets/categories"),
        None,
    )
    .await;

    // --- 3. GET /api/fixed-assets/categories/{id} ---
    println!("\n--- 3. GET /api/fixed-assets/categories/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/fixed-assets/categories/{cat_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Get category failed: {}",
        resp.status()
    );
    let fetched_cat: Value = resp.json().await.unwrap();
    assert_eq!(fetched_cat["name"], "Smoke Test Machinery");
    println!("  retrieved category name={}", fetched_cat["name"]);

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/fixed-assets/categories/{cat_id}"),
        None,
    )
    .await;

    // --- 4. PUT /api/fixed-assets/categories/{id} ---
    println!("\n--- 4. PUT /api/fixed-assets/categories/{{id}} ---");
    let resp = client
        .put(format!("{base}/api/fixed-assets/categories/{cat_id}"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "name": "Smoke Test Machinery (updated)"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    assert!(
        status.is_success(),
        "Update category failed: {status} - {body_text}"
    );
    println!("  updated category name");

    assert_unauth(
        &client,
        "PUT",
        &format!("{base}/api/fixed-assets/categories/{cat_id}"),
        Some(json!({"name": "X"})),
    )
    .await;

    // --- 5. DELETE /api/fixed-assets/categories/{id} (deactivate) ---
    println!("\n--- 5. DELETE /api/fixed-assets/categories/{{id}} ---");
    let resp = client
        .delete(format!("{base}/api/fixed-assets/categories/{cat2_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    assert!(
        status.is_success(),
        "Deactivate category failed: {status} - {body_text}"
    );
    println!("  deactivated category2");

    assert_unauth(
        &client,
        "DELETE",
        &format!("{base}/api/fixed-assets/categories/{cat2_id}"),
        None,
    )
    .await;

    // --- 6. POST /api/fixed-assets/assets (asset1 with depreciation config) ---
    println!("\n--- 6. POST /api/fixed-assets/assets (asset1) ---");
    let asset_tag1 = format!("FA-{}", &Uuid::new_v4().to_string()[..8]);
    let resp = client
        .post(format!("{base}/api/fixed-assets/assets"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "category_id": cat_id,
            "asset_tag": asset_tag1,
            "name": "Smoke Test CNC Machine",
            "description": "CNC mill for smoke testing",
            "acquisition_date": today.to_string(),
            "acquisition_cost_minor": 5000000,
            "in_service_date": in_service.to_string(),
            "depreciation_method": "straight_line",
            "useful_life_months": 24
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let asset1_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create asset1 failed: {status} - {asset1_body}"
    );
    let asset1_id = asset1_body["id"]
        .as_str()
        .expect("No id in asset1 response");
    println!("  created asset1 id={asset1_id} tag={asset_tag1}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/fixed-assets/assets"),
        Some(
            json!({"asset_tag": "X", "name": "X", "acquisition_date": today.to_string(),
                    "acquisition_cost_minor": 1000}),
        ),
    )
    .await;

    // --- 7. POST /api/fixed-assets/assets (asset2 for disposal) ---
    println!("\n--- 7. POST /api/fixed-assets/assets (asset2 for disposal) ---");
    let asset_tag2 = format!("FA2-{}", &Uuid::new_v4().to_string()[..8]);
    let resp = client
        .post(format!("{base}/api/fixed-assets/assets"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "category_id": cat_id,
            "asset_tag": asset_tag2,
            "name": "Smoke Test Lathe",
            "acquisition_date": today.to_string(),
            "acquisition_cost_minor": 2000000
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let asset2_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create asset2 failed: {status} - {asset2_body}"
    );
    let asset2_id = asset2_body["id"]
        .as_str()
        .expect("No id in asset2 response");
    println!("  created asset2 id={asset2_id} tag={asset_tag2}");

    // --- 8. GET /api/fixed-assets/assets ---
    println!("\n--- 8. GET /api/fixed-assets/assets ---");
    let resp = client
        .get(format!("{base}/api/fixed-assets/assets"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "List assets failed: {}",
        resp.status()
    );
    let assets_list: Value = resp.json().await.unwrap();
    let asset_count = assets_list.as_array().map_or(0, |a| a.len());
    assert!(
        asset_count >= 2,
        "Expected at least 2 assets, got {asset_count}"
    );
    println!("  listed {asset_count} asset(s)");

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/fixed-assets/assets"),
        None,
    )
    .await;

    // --- 9. GET /api/fixed-assets/assets/{id} ---
    println!("\n--- 9. GET /api/fixed-assets/assets/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/fixed-assets/assets/{asset1_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Get asset failed: {}",
        resp.status()
    );
    let fetched_asset: Value = resp.json().await.unwrap();
    assert_eq!(fetched_asset["name"], "Smoke Test CNC Machine");
    println!("  retrieved asset name={}", fetched_asset["name"]);

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/fixed-assets/assets/{asset1_id}"),
        None,
    )
    .await;

    // --- 10. PUT /api/fixed-assets/assets/{id} ---
    println!("\n--- 10. PUT /api/fixed-assets/assets/{{id}} ---");
    let resp = client
        .put(format!("{base}/api/fixed-assets/assets/{asset1_id}"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "location": "Shop Floor A",
            "department": "Manufacturing"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    assert!(
        status.is_success(),
        "Update asset failed: {status} - {body_text}"
    );
    println!("  updated asset location/department");

    assert_unauth(
        &client,
        "PUT",
        &format!("{base}/api/fixed-assets/assets/{asset1_id}"),
        Some(json!({"location": "X"})),
    )
    .await;

    // --- 11. POST /api/fixed-assets/depreciation/schedule ---
    println!("\n--- 11. POST /api/fixed-assets/depreciation/schedule ---");
    let resp = client
        .post(format!("{base}/api/fixed-assets/depreciation/schedule"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "asset_id": asset1_id
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let schedule_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Generate schedule failed: {status} - {schedule_body}"
    );
    let entry_count = schedule_body.as_array().map_or(1, |a| a.len());
    println!("  generated depreciation schedule with {entry_count} entry(ies)");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/fixed-assets/depreciation/schedule"),
        Some(json!({"asset_id": Uuid::new_v4()})),
    )
    .await;

    // --- 12. POST /api/fixed-assets/depreciation/runs ---
    println!("\n--- 12. POST /api/fixed-assets/depreciation/runs ---");
    let resp = client
        .post(format!("{base}/api/fixed-assets/depreciation/runs"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "as_of_date": today.to_string()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let run_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create depreciation run failed: {status} - {run_body}"
    );
    let run_id = run_body["id"].as_str().expect("No id in run response");
    println!("  created depreciation run id={run_id}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/fixed-assets/depreciation/runs"),
        Some(json!({"as_of_date": today.to_string()})),
    )
    .await;

    // --- 13. GET /api/fixed-assets/depreciation/runs ---
    println!("\n--- 13. GET /api/fixed-assets/depreciation/runs ---");
    let resp = client
        .get(format!("{base}/api/fixed-assets/depreciation/runs"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "List runs failed: {}",
        resp.status()
    );
    let runs: Value = resp.json().await.unwrap();
    let run_count = runs.as_array().map_or(0, |a| a.len());
    assert!(run_count >= 1, "Expected at least 1 run, got {run_count}");
    println!("  listed {run_count} run(s)");

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/fixed-assets/depreciation/runs"),
        None,
    )
    .await;

    // --- 14. GET /api/fixed-assets/depreciation/runs/{id} ---
    println!("\n--- 14. GET /api/fixed-assets/depreciation/runs/{{id}} ---");
    let resp = client
        .get(format!(
            "{base}/api/fixed-assets/depreciation/runs/{run_id}"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Get run failed: {}",
        resp.status()
    );
    let fetched_run: Value = resp.json().await.unwrap();
    println!(
        "  retrieved run id={} status={}",
        fetched_run["id"].as_str().unwrap_or("?"),
        fetched_run["status"].as_str().unwrap_or("?")
    );

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/fixed-assets/depreciation/runs/{run_id}"),
        None,
    )
    .await;

    // --- 15. POST /api/fixed-assets/disposals ---
    println!("\n--- 15. POST /api/fixed-assets/disposals ---");
    let resp = client
        .post(format!("{base}/api/fixed-assets/disposals"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "asset_id": asset2_id,
            "disposal_type": "scrap",
            "disposal_date": today.to_string(),
            "reason": "Smoke test disposal"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let disposal_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Dispose asset failed: {status} - {disposal_body}"
    );
    let disposal_id = disposal_body["id"]
        .as_str()
        .expect("No id in disposal response");
    println!("  disposed asset2 disposal_id={disposal_id}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/fixed-assets/disposals"),
        Some(json!({"asset_id": Uuid::new_v4(), "disposal_type": "scrap",
                    "disposal_date": today.to_string()})),
    )
    .await;

    // --- 16. GET /api/fixed-assets/disposals ---
    println!("\n--- 16. GET /api/fixed-assets/disposals ---");
    let resp = client
        .get(format!("{base}/api/fixed-assets/disposals"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "List disposals failed: {}",
        resp.status()
    );
    let disposals: Value = resp.json().await.unwrap();
    let disposal_count = disposals.as_array().map_or(0, |a| a.len());
    assert!(
        disposal_count >= 1,
        "Expected at least 1 disposal, got {disposal_count}"
    );
    println!("  listed {disposal_count} disposal(s)");

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/fixed-assets/disposals"),
        None,
    )
    .await;

    // --- 17. GET /api/fixed-assets/disposals/{id} ---
    println!("\n--- 17. GET /api/fixed-assets/disposals/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/fixed-assets/disposals/{disposal_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Get disposal failed: {}",
        resp.status()
    );
    let fetched_disposal: Value = resp.json().await.unwrap();
    assert_eq!(
        fetched_disposal["disposal_type"]
            .as_str()
            .unwrap_or("")
            .to_lowercase(),
        "scrap"
    );
    println!(
        "  retrieved disposal type={}",
        fetched_disposal["disposal_type"]
    );

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/fixed-assets/disposals/{disposal_id}"),
        None,
    )
    .await;

    // --- 18. DELETE /api/fixed-assets/assets/{id} (deactivate asset1) ---
    println!("\n--- 18. DELETE /api/fixed-assets/assets/{{id}} (deactivate asset1) ---");
    let resp = client
        .delete(format!("{base}/api/fixed-assets/assets/{asset1_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    assert!(
        status.is_success(),
        "Deactivate asset failed: {status} - {body_text}"
    );
    println!("  deactivated asset1");

    assert_unauth(
        &client,
        "DELETE",
        &format!("{base}/api/fixed-assets/assets/{asset1_id}"),
        None,
    )
    .await;

    println!("\n=== All 17 Fixed Assets routes passed ===");
}
