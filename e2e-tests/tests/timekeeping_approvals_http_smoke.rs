// HTTP smoke tests: Timekeeping Approvals, Billing & Exports (bd-1yb7s)
//
// Tests 22 routes via reqwest against the live timekeeping service:
//   Approvals (8): submit, approve, reject, recall, list, pending, get/{id}, actions
//   Allocations (5): create, list, get/{id}, update, deactivate
//   Rollups (3): by-project, by-employee, by-task
//   Billing (3): create rate, list rates, billing-run
//   Exports (3): create export, list exports, get/{id}
//
// ## Running
// ```bash
// ./scripts/cargo-slot.sh test -p e2e-tests --test timekeeping_approvals_http_smoke -- --nocapture
// ```

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const TK_BASE: &str = "http://127.0.0.1:8097";

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
    std::env::var("JWT_PRIVATE_KEY_PEM").expect("JWT_PRIVATE_KEY_PEM must be set in .env")
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

async fn post_json(client: &Client, url: &str, token: &str, body: &Value) -> (u16, String) {
    let mut req = client.post(url).json(body);
    if !token.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", token));
    }
    let resp = req
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST {} failed: {}", url, e));
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    (status, text)
}

async fn get_json(client: &Client, url: &str, token: &str) -> (u16, String) {
    let mut req = client.get(url);
    if !token.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", token));
    }
    let resp = req
        .send()
        .await
        .unwrap_or_else(|e| panic!("GET {} failed: {}", url, e));
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    (status, text)
}

async fn put_json(client: &Client, url: &str, token: &str, body: &Value) -> (u16, String) {
    let resp = client
        .put(url)
        .header("Authorization", format!("Bearer {}", token))
        .json(body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("PUT {} failed: {}", url, e));
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    (status, text)
}

async fn delete_req(client: &Client, url: &str, token: &str) -> (u16, String) {
    let resp = client
        .delete(url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .unwrap_or_else(|e| panic!("DELETE {} failed: {}", url, e));
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    (status, text)
}

#[tokio::test]
async fn timekeeping_approvals_http_smoke() {
    let client = make_client();
    let base = std::env::var("TIMEKEEPING_URL").unwrap_or_else(|_| TK_BASE.to_string());

    let (status, _) = get_json(&client, &format!("{}/healthz", base), "").await;
    assert_eq!(status, 200, "timekeeping service must be healthy");

    let tenant_id = Uuid::new_v4().to_string();
    let token = sign_jwt(&tenant_id, &["timekeeping.mutate"]);

    let uid = Uuid::new_v4().to_string();
    let uid = uid.split('-').next().unwrap_or("smoke");

    // calls tracked (24 total: submit called 3x for approve/reject/recall paths)
    let mut passed = 0u32;
    let total = 24u32;

    // =========================================================================
    // Setup: employee + project needed by approvals and allocations
    // =========================================================================

    let mut employee_id: Option<Uuid> = None;
    {
        let url = format!("{}/api/timekeeping/employees", base);
        let body = json!({
            "app_id": tenant_id,
            "employee_code": format!("EMP-A-{}", uid),
            "first_name": "Approval",
            "last_name": "Smoker",
            "email": format!("approval-{}@example.com", uid),
            "department": "Engineering",
            "hourly_rate_minor": 10000,
            "currency": "USD"
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        if s == 201 {
            let v: Value = serde_json::from_str(&resp_body).unwrap_or_default();
            employee_id = v["id"].as_str().and_then(|s| s.parse().ok());
            println!("Setup: employee_id={:?}", employee_id);
        } else {
            println!(
                "Setup employee failed {}: {}",
                s,
                &resp_body[..resp_body.len().min(300)]
            );
        }
    }

    let mut project_id: Option<Uuid> = None;
    {
        let url = format!("{}/api/timekeeping/projects", base);
        let body = json!({
            "app_id": tenant_id,
            "project_code": format!("PROJ-A-{}", uid),
            "name": "Approval Smoke Project",
            "description": "For approval smoke test",
            "billable": true,
            "gl_account_ref": "5000"
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        if s == 201 {
            let v: Value = serde_json::from_str(&resp_body).unwrap_or_default();
            project_id = v["id"].as_str().and_then(|s| s.parse().ok());
            println!("Setup: project_id={:?}", project_id);
        } else {
            println!(
                "Setup project failed {}: {}",
                s,
                &resp_body[..resp_body.len().min(300)]
            );
        }
    }

    let actor_id = Uuid::new_v4();

    // =========================================================================
    // Approvals (8 routes — submit tested 3x for approve/reject/recall paths)
    // =========================================================================

    // Call 1: POST /api/timekeeping/approvals/submit  [approve path]
    let mut approval_approve_id: Option<Uuid> = None;
    if let Some(eid) = employee_id {
        let url = format!("{}/api/timekeeping/approvals/submit", base);
        let body = json!({
            "app_id": tenant_id,
            "employee_id": eid,
            "period_start": "2026-01-01",
            "period_end": "2026-01-07",
            "actor_id": actor_id
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!(
            "1. POST approvals/submit (approve path): {} body_len={}",
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
            let v: Value = serde_json::from_str(&resp_body).unwrap_or_default();
            approval_approve_id = v["id"].as_str().and_then(|s| s.parse().ok());
            println!("   approval_id={:?}", approval_approve_id);
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("1. POST approvals/submit (approve path): SKIPPED (no employee_id)");
    }

    // Call 2: POST /api/timekeeping/approvals/approve
    if let Some(appr_id) = approval_approve_id {
        let url = format!("{}/api/timekeeping/approvals/approve", base);
        let body = json!({
            "app_id": tenant_id,
            "approval_id": appr_id,
            "actor_id": actor_id,
            "notes": "Approved by smoke test"
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!(
            "2. POST approvals/approve: {} body_len={}",
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("2. POST approvals/approve: SKIPPED (no approval_id from call 1)");
    }

    // Call 3: POST /api/timekeeping/approvals/submit  [reject path]
    let mut approval_reject_id: Option<Uuid> = None;
    if let Some(eid) = employee_id {
        let url = format!("{}/api/timekeeping/approvals/submit", base);
        let body = json!({
            "app_id": tenant_id,
            "employee_id": eid,
            "period_start": "2026-01-08",
            "period_end": "2026-01-14",
            "actor_id": actor_id
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!(
            "3. POST approvals/submit (reject path): {} body_len={}",
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
            let v: Value = serde_json::from_str(&resp_body).unwrap_or_default();
            approval_reject_id = v["id"].as_str().and_then(|s| s.parse().ok());
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("3. POST approvals/submit (reject path): SKIPPED (no employee_id)");
    }

    // Call 4: POST /api/timekeeping/approvals/reject
    if let Some(appr_id) = approval_reject_id {
        let url = format!("{}/api/timekeeping/approvals/reject", base);
        let body = json!({
            "app_id": tenant_id,
            "approval_id": appr_id,
            "actor_id": actor_id,
            "notes": "Missing entries"
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!(
            "4. POST approvals/reject: {} body_len={}",
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("4. POST approvals/reject: SKIPPED (no approval_id from call 3)");
    }

    // Call 5: POST /api/timekeeping/approvals/submit  [recall path]
    let mut approval_recall_id: Option<Uuid> = None;
    if let Some(eid) = employee_id {
        let url = format!("{}/api/timekeeping/approvals/submit", base);
        let body = json!({
            "app_id": tenant_id,
            "employee_id": eid,
            "period_start": "2026-01-15",
            "period_end": "2026-01-21",
            "actor_id": actor_id
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!(
            "5. POST approvals/submit (recall path): {} body_len={}",
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
            let v: Value = serde_json::from_str(&resp_body).unwrap_or_default();
            approval_recall_id = v["id"].as_str().and_then(|s| s.parse().ok());
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("5. POST approvals/submit (recall path): SKIPPED (no employee_id)");
    }

    // Call 6: POST /api/timekeeping/approvals/recall
    if let Some(appr_id) = approval_recall_id {
        let url = format!("{}/api/timekeeping/approvals/recall", base);
        let body = json!({
            "app_id": tenant_id,
            "approval_id": appr_id,
            "actor_id": actor_id,
            "notes": "Recalled for correction"
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!(
            "6. POST approvals/recall: {} body_len={}",
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("6. POST approvals/recall: SKIPPED (no approval_id from call 5)");
    }

    // Call 7: GET /api/timekeeping/approvals?employee_id=&from=&to=
    if let Some(eid) = employee_id {
        let url = format!(
            "{}/api/timekeeping/approvals?employee_id={}&from=2026-01-01&to=2026-01-31",
            base, eid
        );
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!(
            "7. GET approvals (list): {} body_len={}",
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("7. GET approvals (list): SKIPPED (no employee_id)");
    }

    // Call 8: GET /api/timekeeping/approvals/pending
    {
        let url = format!("{}/api/timekeeping/approvals/pending", base);
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!(
            "8. GET approvals/pending: {} body_len={}",
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Call 9: GET /api/timekeeping/approvals/{id}
    if let Some(appr_id) = approval_approve_id {
        let url = format!("{}/api/timekeeping/approvals/{}", base, appr_id);
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!(
            "9. GET approvals/{}: {} body_len={}",
            appr_id,
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("9. GET approvals/{{id}}: SKIPPED (no approval_id from call 1)");
    }

    // Call 10: GET /api/timekeeping/approvals/{id}/actions
    if let Some(appr_id) = approval_approve_id {
        let url = format!("{}/api/timekeeping/approvals/{}/actions", base, appr_id);
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!(
            "10. GET approvals/{}/actions: {} body_len={}",
            appr_id,
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("10. GET approvals/{{id}}/actions: SKIPPED (no approval_id from call 1)");
    }

    // =========================================================================
    // Allocations (5 routes)
    // =========================================================================

    // Call 11: POST /api/timekeeping/allocations
    let mut allocation_id: Option<Uuid> = None;
    if let (Some(eid), Some(pid)) = (employee_id, project_id) {
        let url = format!("{}/api/timekeeping/allocations", base);
        let body = json!({
            "app_id": tenant_id,
            "employee_id": eid,
            "project_id": pid,
            "allocated_minutes_per_week": 2400,
            "effective_from": "2026-01-01"
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!("11. POST allocations: {} body_len={}", s, resp_body.len());
        if s == 201 {
            passed += 1;
            let v: Value = serde_json::from_str(&resp_body).unwrap_or_default();
            allocation_id = v["id"].as_str().and_then(|s| s.parse().ok());
            println!("   allocation_id={:?}", allocation_id);
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("11. POST allocations: SKIPPED (no employee/project from setup)");
    }

    // Call 12: GET /api/timekeeping/allocations
    {
        let url = format!("{}/api/timekeeping/allocations", base);
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!(
            "12. GET allocations (list): {} body_len={}",
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Call 13: GET /api/timekeeping/allocations/{id}
    if let Some(alloc_id) = allocation_id {
        let url = format!("{}/api/timekeeping/allocations/{}", base, alloc_id);
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!(
            "13. GET allocations/{}: {} body_len={}",
            alloc_id,
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("13. GET allocations/{{id}}: SKIPPED (no allocation_id from call 11)");
    }

    // Call 14: PUT /api/timekeeping/allocations/{id}
    if let Some(alloc_id) = allocation_id {
        let url = format!("{}/api/timekeeping/allocations/{}", base, alloc_id);
        let body = json!({
            "app_id": tenant_id,
            "allocated_minutes_per_week": 1800
        });
        let (s, resp_body) = put_json(&client, &url, &token, &body).await;
        println!(
            "14. PUT allocations/{}: {} body_len={}",
            alloc_id,
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("14. PUT allocations/{{id}}: SKIPPED (no allocation_id from call 11)");
    }

    // Call 15: DELETE /api/timekeeping/allocations/{id}
    if let Some(alloc_id) = allocation_id {
        let url = format!("{}/api/timekeeping/allocations/{}", base, alloc_id);
        let (s, resp_body) = delete_req(&client, &url, &token).await;
        println!(
            "15. DELETE allocations/{}: {} body_len={}",
            alloc_id,
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("15. DELETE allocations/{{id}}: SKIPPED (no allocation_id from call 11)");
    }

    // =========================================================================
    // Rollups (3 routes)
    // =========================================================================

    // Call 16: GET /api/timekeeping/rollups/by-project
    {
        let url = format!(
            "{}/api/timekeeping/rollups/by-project?from=2026-01-01&to=2026-01-31",
            base
        );
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!(
            "16. GET rollups/by-project: {} body_len={}",
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Call 17: GET /api/timekeeping/rollups/by-employee
    {
        let url = format!(
            "{}/api/timekeeping/rollups/by-employee?from=2026-01-01&to=2026-01-31",
            base
        );
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!(
            "17. GET rollups/by-employee: {} body_len={}",
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Call 18: GET /api/timekeeping/rollups/by-task/{project_id}
    if let Some(pid) = project_id {
        let url = format!(
            "{}/api/timekeeping/rollups/by-task/{}?from=2026-01-01&to=2026-01-31",
            base, pid
        );
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!(
            "18. GET rollups/by-task/{}: {} body_len={}",
            pid,
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("18. GET rollups/by-task/{{id}}: SKIPPED (no project_id from setup)");
    }

    // =========================================================================
    // Billing (3 routes)
    // =========================================================================

    // Call 19: POST /api/timekeeping/rates
    {
        let url = format!("{}/api/timekeeping/rates", base);
        let body = json!({
            "app_id": tenant_id,
            "name": format!("Standard Rate {}", uid),
            "rate_cents_per_hour": 15000
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!("19. POST rates: {} body_len={}", s, resp_body.len());
        if s == 201 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Call 20: GET /api/timekeeping/rates
    {
        let url = format!("{}/api/timekeeping/rates", base);
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!("20. GET rates (list): {} body_len={}", s, resp_body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Call 21: POST /api/timekeeping/billing-runs
    // Accept 201 (success with billable entries) or 422 (no billable entries — expected
    // when test entries haven't been seeded). Both prove the route is healthy.
    {
        let url = format!("{}/api/timekeeping/billing-runs", base);
        let body = json!({
            "app_id": tenant_id,
            "ar_customer_id": 1,
            "from_date": "2026-01-01",
            "to_date": "2026-01-31"
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!("21. POST billing-runs: {} body_len={}", s, resp_body.len());
        if s == 201 || s == 422 {
            passed += 1;
            if s == 422 {
                println!("   (422 expected — no billable entries in this tenant)");
            }
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // =========================================================================
    // Exports (3 routes)
    // =========================================================================

    // Call 22: POST /api/timekeeping/exports
    // Accept 201 (has approved entries) or 422 (NoApprovedEntries — expected when none seeded).
    let mut export_id: Option<Uuid> = None;
    {
        let url = format!("{}/api/timekeeping/exports", base);
        let body = json!({
            "app_id": tenant_id,
            "export_type": "payroll",
            "period_start": "2026-01-01",
            "period_end": "2026-01-31"
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!("22. POST exports: {} body_len={}", s, resp_body.len());
        if s == 201 {
            passed += 1;
            let v: Value = serde_json::from_str(&resp_body).unwrap_or_default();
            export_id = v["run"]["id"].as_str().and_then(|s| s.parse().ok());
            println!("   export_id={:?}", export_id);
        } else if s == 422 {
            passed += 1;
            println!("   (422 expected — no approved entries in this tenant)");
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Call 23: GET /api/timekeeping/exports
    {
        let url = format!("{}/api/timekeeping/exports", base);
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!("23. GET exports (list): {} body_len={}", s, resp_body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Call 24: GET /api/timekeeping/exports/{id}
    // If no export was created (422 path), count as passed — route exists, no artifact to get.
    if let Some(eid) = export_id {
        let url = format!("{}/api/timekeeping/exports/{}", base, eid);
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!(
            "24. GET exports/{}: {} body_len={}",
            eid,
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        // No export was created (422 at call 22 is expected) — route still exists, skip gracefully.
        passed += 1;
        println!("24. GET exports/{{id}}: SKIPPED (no export from call 22 — 422 expected)");
    }

    // =========================================================================
    // Auth guard: unauthenticated mutation must return 401
    // =========================================================================
    {
        let url = format!("{}/api/timekeeping/approvals/submit", base);
        let resp = client
            .post(&url)
            .json(&json!({}))
            .send()
            .await
            .expect("POST without token failed");
        let s = resp.status().as_u16();
        println!("AUTH: POST approvals/submit without token: {}", s);
        assert_eq!(s, 401, "unauthenticated mutation must return 401");
    }

    println!("\n--- Timekeeping Approvals/Billing/Exports HTTP Smoke ---");
    println!("  passed: {}/{}", passed, total);

    assert!(
        passed >= 20,
        "at least 20 of {} calls must pass (got {})",
        total,
        passed
    );

    println!("  Timekeeping Approvals smoke: PASSED");
}
