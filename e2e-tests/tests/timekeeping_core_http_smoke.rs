// HTTP smoke: Timekeeping Core (bd-15n9i)
//
// Tests 20 routes via reqwest against the live timekeeping service:
// employees, projects, tasks, and entries (full CRUD + lifecycle).
//
// ## Running
// ```bash
// ./scripts/cargo-slot.sh test -p e2e-tests --test timekeeping_core_http_smoke -- --nocapture
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
async fn timekeeping_core_http_smoke() {
    let client = make_client();
    let base = std::env::var("TIMEKEEPING_URL").unwrap_or_else(|_| TK_BASE.to_string());

    let (status, _) = get_json(&client, &format!("{}/healthz", base), "").await;
    assert_eq!(status, 200, "timekeeping service must be healthy");

    let tenant_id = Uuid::new_v4().to_string();
    let token = sign_jwt(&tenant_id, &["timekeeping.mutate"]);

    // Unique codes per run to avoid duplicate_code errors
    let uid = Uuid::new_v4().to_string();
    let uid = uid.split('-').next().unwrap_or("smoke");

    let mut passed = 0u32;
    let total = 20u32;

    // =========================================================================
    // Employees (5 routes)
    // =========================================================================

    // Route 1: POST /api/timekeeping/employees
    let mut employee_id: Option<Uuid> = None;
    {
        let url = format!("{}/api/timekeeping/employees", base);
        let body = json!({
            "employee_code": format!("EMP-{}", uid),
            "first_name": "Smoke",
            "last_name": "Tester",
            "email": format!("smoke-{}@example.com", uid),
            "department": "Engineering",
            "hourly_rate_minor": 10000,
            "currency": "USD"
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!("1. POST employees: {} body_len={}", s, resp_body.len());
        if s == 201 {
            passed += 1;
            let v: Value = serde_json::from_str(&resp_body).unwrap_or_default();
            employee_id = v["id"].as_str().and_then(|s| s.parse().ok());
            println!("   employee_id: {:?}", employee_id);
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Route 2: GET /api/timekeeping/employees
    {
        let url = format!("{}/api/timekeeping/employees", base);
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!("2. GET employees (list): {} body_len={}", s, resp_body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Route 3: GET /api/timekeeping/employees/{id}
    if let Some(eid) = employee_id {
        let url = format!("{}/api/timekeeping/employees/{}", base, eid);
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!("3. GET employees/{}: {} body_len={}", eid, s, resp_body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("3. GET employees/{{id}}: SKIPPED (no employee_id from step 1)");
    }

    // Route 4: PUT /api/timekeeping/employees/{id}
    // update_employee handler takes app_id from request body (no extract_tenant call)
    if let Some(eid) = employee_id {
        let url = format!("{}/api/timekeeping/employees/{}", base, eid);
        let body = json!({
            "app_id": tenant_id,
            "first_name": "SmokeUpdated",
            "department": "QA"
        });
        let (s, resp_body) = put_json(&client, &url, &token, &body).await;
        println!("4. PUT employees/{}: {} body_len={}", eid, s, resp_body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("4. PUT employees/{{id}}: SKIPPED (no employee_id from step 1)");
    }

    // =========================================================================
    // Projects (5 routes)
    // =========================================================================

    // Route 5: POST /api/timekeeping/projects
    let mut project_id: Option<Uuid> = None;
    {
        let url = format!("{}/api/timekeeping/projects", base);
        let body = json!({
            "project_code": format!("PROJ-{}", uid),
            "name": "Smoke Test Project",
            "description": "Created by timekeeping core smoke test",
            "billable": true,
            "gl_account_ref": "5000"
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!("5. POST projects: {} body_len={}", s, resp_body.len());
        if s == 201 {
            passed += 1;
            let v: Value = serde_json::from_str(&resp_body).unwrap_or_default();
            project_id = v["id"].as_str().and_then(|s| s.parse().ok());
            println!("   project_id: {:?}", project_id);
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Route 6: GET /api/timekeeping/projects
    {
        let url = format!("{}/api/timekeeping/projects", base);
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!("6. GET projects (list): {} body_len={}", s, resp_body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Route 7: GET /api/timekeeping/projects/{id}
    if let Some(pid) = project_id {
        let url = format!("{}/api/timekeeping/projects/{}", base, pid);
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!("7. GET projects/{}: {} body_len={}", pid, s, resp_body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("7. GET projects/{{id}}: SKIPPED (no project_id from step 5)");
    }

    // Route 8: PUT /api/timekeeping/projects/{id}
    // update_project takes app_id from request body
    if let Some(pid) = project_id {
        let url = format!("{}/api/timekeeping/projects/{}", base, pid);
        let body = json!({
            "app_id": tenant_id,
            "name": "Smoke Test Project (Updated)"
        });
        let (s, resp_body) = put_json(&client, &url, &token, &body).await;
        println!("8. PUT projects/{}: {} body_len={}", pid, s, resp_body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("8. PUT projects/{{id}}: SKIPPED (no project_id from step 5)");
    }

    // =========================================================================
    // Tasks (5 routes)
    // =========================================================================

    // Route 9: POST /api/timekeeping/tasks
    let mut task_id: Option<Uuid> = None;
    if let Some(pid) = project_id {
        let url = format!("{}/api/timekeeping/tasks", base);
        let body = json!({
            "project_id": pid,
            "task_code": format!("TASK-{}", uid),
            "name": "Smoke Test Task"
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!("9. POST tasks: {} body_len={}", s, resp_body.len());
        if s == 201 {
            passed += 1;
            let v: Value = serde_json::from_str(&resp_body).unwrap_or_default();
            task_id = v["id"].as_str().and_then(|s| s.parse().ok());
            println!("   task_id: {:?}", task_id);
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("9. POST tasks: SKIPPED (no project_id from step 5)");
    }

    // Route 10: GET /api/timekeeping/tasks/{id}
    if let Some(tid) = task_id {
        let url = format!("{}/api/timekeeping/tasks/{}", base, tid);
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!("10. GET tasks/{}: {} body_len={}", tid, s, resp_body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("10. GET tasks/{{id}}: SKIPPED (no task_id from step 9)");
    }

    // Route 11: GET /api/timekeeping/projects/{project_id}/tasks
    if let Some(pid) = project_id {
        let url = format!("{}/api/timekeeping/projects/{}/tasks", base, pid);
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!("11. GET projects/{}/tasks: {} body_len={}", pid, s, resp_body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("11. GET projects/{{id}}/tasks: SKIPPED (no project_id from step 5)");
    }

    // Route 12: PUT /api/timekeeping/tasks/{id}
    // update_task takes app_id from request body
    if let Some(tid) = task_id {
        let url = format!("{}/api/timekeeping/tasks/{}", base, tid);
        let body = json!({
            "app_id": tenant_id,
            "name": "Smoke Test Task (Updated)"
        });
        let (s, resp_body) = put_json(&client, &url, &token, &body).await;
        println!("12. PUT tasks/{}: {} body_len={}", tid, s, resp_body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("12. PUT tasks/{{id}}: SKIPPED (no task_id from step 9)");
    }

    // =========================================================================
    // Entries (5 routes)
    // =========================================================================

    // Route 13: POST /api/timekeeping/entries
    let mut entry_id: Option<Uuid> = None;
    if let Some(eid) = employee_id {
        let url = format!("{}/api/timekeeping/entries", base);
        let body = json!({
            "employee_id": eid,
            "project_id": project_id,
            "task_id": task_id,
            "work_date": "2026-01-15",
            "minutes": 480,
            "description": "Smoke test entry — 8 hours"
        });
        let idempotency_key = Uuid::new_v4().to_string();
        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("idempotency-key", &idempotency_key)
            .json(&body)
            .send()
            .await
            .unwrap_or_else(|e| panic!("POST {} failed: {}", url, e));
        let s = resp.status().as_u16();
        let resp_body = resp.text().await.unwrap_or_default();
        println!("13. POST entries: {} body_len={}", s, resp_body.len());
        if s == 201 {
            passed += 1;
            let v: Value = serde_json::from_str(&resp_body).unwrap_or_default();
            entry_id = v["id"].as_str().and_then(|s| s.parse().ok());
            println!("   entry_id: {:?}", entry_id);
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("13. POST entries: SKIPPED (no employee_id from step 1)");
    }

    // Route 14: GET /api/timekeeping/entries?employee_id=...&from=...&to=...
    if let Some(eid) = employee_id {
        let url = format!(
            "{}/api/timekeeping/entries?employee_id={}&from=2026-01-01&to=2026-01-31",
            base, eid
        );
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!("14. GET entries (list): {} body_len={}", s, resp_body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("14. GET entries (list): SKIPPED (no employee_id from step 1)");
    }

    // Route 15: POST /api/timekeeping/entries/correct
    if let Some(eid) = entry_id {
        let url = format!("{}/api/timekeeping/entries/correct", base);
        let body = json!({
            "entry_id": eid,
            "minutes": 420,
            "description": "Corrected to 7 hours"
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!("15. POST entries/correct: {} body_len={}", s, resp_body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("15. POST entries/correct: SKIPPED (no entry_id from step 13)");
    }

    // Route 16: GET /api/timekeeping/entries/{entry_id}/history
    if let Some(eid) = entry_id {
        let url = format!("{}/api/timekeeping/entries/{}/history", base, eid);
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!("16. GET entries/{}/history: {} body_len={}", eid, s, resp_body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("16. GET entries/{{id}}/history: SKIPPED (no entry_id from step 13)");
    }

    // Route 17: POST /api/timekeeping/entries/void
    if let Some(eid) = entry_id {
        let url = format!("{}/api/timekeeping/entries/void", base);
        let body = json!({
            "entry_id": eid
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!("17. POST entries/void: {} body_len={}", s, resp_body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("17. POST entries/void: SKIPPED (no entry_id from step 13)");
    }

    // =========================================================================
    // Deactivations (3 routes — run after entry tests to avoid FK issues)
    // =========================================================================

    // Route 18: DELETE /api/timekeeping/tasks/{id}
    if let Some(tid) = task_id {
        let url = format!("{}/api/timekeeping/tasks/{}", base, tid);
        let (s, resp_body) = delete_req(&client, &url, &token).await;
        println!("18. DELETE tasks/{}: {} body_len={}", tid, s, resp_body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("18. DELETE tasks/{{id}}: SKIPPED (no task_id from step 9)");
    }

    // Route 19: DELETE /api/timekeeping/projects/{id}
    if let Some(pid) = project_id {
        let url = format!("{}/api/timekeeping/projects/{}", base, pid);
        let (s, resp_body) = delete_req(&client, &url, &token).await;
        println!("19. DELETE projects/{}: {} body_len={}", pid, s, resp_body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("19. DELETE projects/{{id}}: SKIPPED (no project_id from step 5)");
    }

    // Route 20: DELETE /api/timekeeping/employees/{id}
    if let Some(eid) = employee_id {
        let url = format!("{}/api/timekeeping/employees/{}", base, eid);
        let (s, resp_body) = delete_req(&client, &url, &token).await;
        println!("20. DELETE employees/{}: {} body_len={}", eid, s, resp_body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("20. DELETE employees/{{id}}: SKIPPED (no employee_id from step 1)");
    }

    // =========================================================================
    // Auth guard: unauthenticated mutation must return 401
    // =========================================================================
    {
        let url = format!("{}/api/timekeeping/employees", base);
        let resp = client
            .post(&url)
            .json(&json!({}))
            .send()
            .await
            .expect("POST without token failed");
        let s = resp.status().as_u16();
        println!("AUTH: POST employees without token: {}", s);
        assert_eq!(s, 401, "unauthenticated mutation must return 401");
    }

    println!("\n--- Timekeeping Core HTTP Smoke ---");
    println!("  passed: {}/{}", passed, total);

    assert!(
        passed >= 16,
        "at least 16 of {} routes must pass (got {})",
        total,
        passed
    );

    println!("  Timekeeping Core smoke: PASSED");
}
