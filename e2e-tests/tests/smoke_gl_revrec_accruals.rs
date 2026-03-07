// HTTP smoke: GL RevRec + Accruals (bd-2jc8g)
//
// Tests 8 routes via reqwest against the live GL service.
//
// ## Running
// ```bash
// ./scripts/cargo-slot.sh test -p e2e-tests --test smoke_gl_revrec_accruals -- --nocapture
// ```

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const GL_BASE: &str = "http://127.0.0.1:8090";

#[derive(Serialize)]
struct TestClaims {
    sub: String,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    jti: String,
    tenant_id: String,
    roles: Vec<String>,
    perms: Vec<String>,
    actor_type: String,
    ver: String,
}

fn dev_private_key_pem() -> String {
    dotenvy::from_filename_override(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.env"),
    )
    .ok();
    std::env::var("JWT_PRIVATE_KEY_PEM")
        .expect("JWT_PRIVATE_KEY_PEM must be set in .env")
}

fn sign_jwt(tenant_id: &str, perms: &[&str]) -> String {
    let pem = dev_private_key_pem();
    let encoding =
        EncodingKey::from_rsa_pem(pem.as_bytes()).expect("failed to parse JWT_PRIVATE_KEY_PEM");
    let now = Utc::now();
    let claims = TestClaims {
        sub: Uuid::new_v4().to_string(),
        iss: "auth-rs".to_string(),
        aud: "7d-platform".to_string(),
        iat: now.timestamp(),
        exp: (now + chrono::Duration::minutes(15)).timestamp(),
        jti: Uuid::new_v4().to_string(),
        tenant_id: tenant_id.to_string(),
        roles: vec!["operator".to_string()],
        perms: perms.iter().map(|s| s.to_string()).collect(),
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &encoding)
        .expect("failed to sign JWT")
}

fn make_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("failed to create HTTP client")
}

async fn post_json(
    client: &Client,
    url: &str,
    token: &str,
    body: &Value,
) -> (u16, String) {
    let resp = client
        .post(url)
        .header("Authorization", format!("Bearer {}", token))
        .json(body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST {} failed: {}", url, e));
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    (status, text)
}

async fn get_json(client: &Client, url: &str, token: &str) -> (u16, String) {
    let resp = client
        .get(url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap_or_else(|e| panic!("GET {} failed: {}", url, e));
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    (status, text)
}

#[tokio::test]
async fn smoke_gl_revrec_accruals() {
    let client = make_client();

    let (status, _) = get_json(&client, &format!("{}/healthz", GL_BASE), "").await;
    assert_eq!(status, 200, "GL service must be healthy");

    let tenant_id = Uuid::new_v4().to_string();
    let token = sign_jwt(&tenant_id, &["gl.post", "gl.read"]);

    let mut passed = 0u32;
    let total = 8u32;

    let contract_id = Uuid::new_v4();
    let obligation_id = Uuid::new_v4();

    // Route 1: POST /api/gl/revrec/contracts
    {
        let url = format!("{}/api/gl/revrec/contracts", GL_BASE);
        let body = json!({
            "contract_id": contract_id,
            "customer_id": "smoke-cust-001",
            "contract_name": "Smoke Test SaaS Contract",
            "contract_start": "2026-01-01",
            "contract_end": "2026-12-31",
            "total_transaction_price_minor": 120000,
            "currency": "USD",
            "performance_obligations": [
                {
                    "obligation_id": obligation_id,
                    "name": "SaaS License",
                    "description": "12-month SaaS subscription",
                    "allocated_amount_minor": 120000,
                    "recognition_pattern": {"type": "ratable_over_time", "period_months": 12},
                    "satisfaction_start": "2026-01-01",
                    "satisfaction_end": "2026-12-31"
                }
            ]
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!("1. POST revrec/contracts: {} body_len={}", s, resp_body.len());
        if s == 201 || s == 409 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Route 2: POST /api/gl/revrec/schedules
    {
        let url = format!("{}/api/gl/revrec/schedules", GL_BASE);
        let body = json!({
            "contract_id": contract_id,
            "obligation_id": obligation_id
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!("2. POST revrec/schedules: {} body_len={}", s, resp_body.len());
        if s == 201 || s == 409 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Route 3: POST /api/gl/revrec/amendments
    {
        let url = format!("{}/api/gl/revrec/amendments", GL_BASE);
        let modification_id = Uuid::new_v4();
        let body = json!({
            "modification_id": modification_id,
            "contract_id": contract_id,
            "tenant_id": tenant_id,
            "modification_type": "price_change",
            "effective_date": "2026-06-01",
            "new_transaction_price_minor": 144000,
            "added_obligations": [],
            "removed_obligation_ids": [],
            "reallocated_amounts": [
                {
                    "obligation_id": obligation_id,
                    "previous_allocated_minor": 120000,
                    "new_allocated_minor": 144000
                }
            ],
            "reason": "Annual price adjustment",
            "requires_cumulative_catchup": true,
            "modified_at": Utc::now().to_rfc3339()
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!("3. POST revrec/amendments: {} body_len={}", s, resp_body.len());
        if s == 201 || s == 409 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Route 4: POST /api/gl/revrec/recognition-runs
    {
        let url = format!("{}/api/gl/revrec/recognition-runs", GL_BASE);
        let body = json!({
            "period": "2026-01",
            "posting_date": "2026-01-31"
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!("4. POST revrec/recognition-runs: {} body_len={}", s, resp_body.len());
        if s == 200 || s == 201 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Route 5: POST /api/gl/accruals/templates
    let template_id: Option<Uuid>;
    {
        let url = format!("{}/api/gl/accruals/templates", GL_BASE);
        let body = json!({
            "tenant_id": tenant_id,
            "name": "Monthly Rent Accrual",
            "description": "Accrue monthly office rent",
            "debit_account": "6100",
            "credit_account": "2100",
            "amount_minor": 500000,
            "currency": "USD",
            "reversal_policy": {
                "auto_reverse_next_period": true,
                "reverse_on_date": null
            },
            "cashflow_class": "operating"
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!("5. POST accruals/templates: {} body_len={}", s, resp_body.len());
        if s == 201 {
            passed += 1;
            let v: Value = serde_json::from_str(&resp_body).unwrap_or_default();
            template_id = v["template_id"]
                .as_str()
                .and_then(|s| s.parse().ok());
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
            template_id = None;
        }
    }

    // Route 6: POST /api/gl/accruals/create
    {
        if let Some(tid) = template_id {
            let url = format!("{}/api/gl/accruals/create", GL_BASE);
            let body = json!({
                "template_id": tid,
                "tenant_id": tenant_id,
                "period": "2026-01",
                "posting_date": "2026-01-31",
                "correlation_id": Uuid::new_v4().to_string()
            });
            let (s, resp_body) = post_json(&client, &url, &token, &body).await;
            println!("6. POST accruals/create: {} body_len={}", s, resp_body.len());
            if s == 200 || s == 201 {
                passed += 1;
            } else {
                println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
            }
        } else {
            println!("6. POST accruals/create: SKIPPED (no template_id from step 5)");
        }
    }

    // Route 7: POST /api/gl/accruals/reversals/execute
    {
        let url = format!("{}/api/gl/accruals/reversals/execute", GL_BASE);
        let body = json!({
            "tenant_id": tenant_id,
            "target_period": "2026-02",
            "reversal_date": "2026-02-01"
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!("7. POST accruals/reversals/execute: {} body_len={}", s, resp_body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Route 8: POST /api/gl/exports
    {
        let url = format!("{}/api/gl/exports", GL_BASE);
        let body = json!({
            "format": "quickbooks",
            "export_type": "chart_of_accounts",
            "idempotency_key": Uuid::new_v4().to_string()
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!("8. POST exports: {} body_len={}", s, resp_body.len());
        if s == 200 || s == 201 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Auth guard: no token -> 401
    {
        let url = format!("{}/api/gl/revrec/contracts", GL_BASE);
        let resp = client
            .post(&url)
            .json(&json!({}))
            .send()
            .await
            .expect("POST without token failed");
        let s = resp.status().as_u16();
        println!("AUTH: POST revrec/contracts without token: {}", s);
        assert_eq!(s, 401, "unauthenticated request must return 401");
    }

    println!("\n--- GL RevRec + Accruals HTTP Smoke ---");
    println!("  passed: {}/{}", passed, total);

    assert!(
        passed >= 6,
        "at least 6 of {} routes must pass (got {})",
        total,
        passed
    );

    println!("  GL RevRec + Accruals smoke: PASSED");
}
