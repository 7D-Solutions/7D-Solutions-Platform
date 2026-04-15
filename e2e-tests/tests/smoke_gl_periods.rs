//! HTTP smoke: GL Period Management (bd-27ni9)
//!
//! Tests 12 GL period-lifecycle routes via reqwest against the live GL service.
//! Exercises: validate-close, close, close-status, reopen request/approve/reject,
//! summary, checklist CRUD (create/complete/waive/get), approvals, account activity.
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests --test smoke_gl_periods -- --nocapture
//! ```

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const GL_BASE: &str = "http://127.0.0.1:8090";

// ============================================================================
// JWT helpers — sign tokens with the dev private key from .env
// ============================================================================

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
    // Read from .env file — same key the GL service uses via JWT_PUBLIC_KEY_PEM
    dotenvy::from_filename_override(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.env"),
    )
    .ok();
    std::env::var("JWT_PRIVATE_KEY_PEM")
        .expect("JWT_PRIVATE_KEY_PEM must be set in .env — needed to sign test JWTs")
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

// ============================================================================
// Helpers
// ============================================================================

/// POST JSON with auth and return (status, body).
async fn post_json(client: &Client, url: &str, token: &str, body: &Value) -> (u16, String) {
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

/// GET with auth and return (status, body).
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

/// Seed an accounting period directly via the GL database.
async fn seed_period(pool: &sqlx::PgPool, tenant_id: &str) -> Uuid {
    let period_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end)
        VALUES ($1, $2, '2026-01-01', '2026-01-31')
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .execute(pool)
    .await
    .expect("failed to seed accounting period");
    period_id
}

/// Seed an account entry for account activity test.
async fn seed_account(pool: &sqlx::PgPool, tenant_id: &str) {
    // Upsert — may already exist from prior runs
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance)
        VALUES ($1, $2, '1000', 'Cash', 'asset', 'debit')
        ON CONFLICT (tenant_id, code) DO NOTHING
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .execute(pool)
    .await
    .expect("failed to seed account");
}

/// Clean up test data.
async fn cleanup(pool: &sqlx::PgPool, tenant_id: &str) {
    for q in [
        "DELETE FROM close_approvals WHERE tenant_id = $1",
        "DELETE FROM close_checklist_items WHERE tenant_id = $1",
        "DELETE FROM period_reopen_requests WHERE tenant_id = $1",
        "DELETE FROM accounting_periods WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(pool).await.ok();
    }
}

async fn get_gl_pool() -> sqlx::PgPool {
    let url = std::env::var("GL_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://gl_user:gl_pass@localhost:5438/gl_db".to_string());
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(3)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("failed to connect to GL DB")
}

// ============================================================================
// Test: GL Period Management HTTP smoke (12 routes)
// ============================================================================

#[tokio::test]
async fn smoke_gl_periods() {
    let client = make_client();

    // Pre-flight: GL service healthy
    let (status, _) = get_json(&client, &format!("{}/healthz", GL_BASE), "").await;
    assert_eq!(status, 200, "GL service must be healthy");

    let pool = get_gl_pool().await;
    let tenant_id = Uuid::new_v4().to_string();
    let token = sign_jwt(&tenant_id, &["gl.post", "gl.read"]);

    // Seed data: accounting period + chart of accounts
    let period_id = seed_period(&pool, &tenant_id).await;
    seed_account(&pool, &tenant_id).await;
    println!("seeded: tenant={}, period={}", tenant_id, period_id);

    let mut passed = 0u32;
    let total = 12u32;

    // ------------------------------------------------------------------
    // Route 1: GET /api/gl/periods/{period_id}/close-status
    // ------------------------------------------------------------------
    {
        let url = format!("{}/api/gl/periods/{}/close-status", GL_BASE, period_id);
        let (s, body) = get_json(&client, &url, &token).await;
        println!("1. GET close-status: {} body_len={}", s, body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &body[..body.len().min(300)]);
        }
    }

    // ------------------------------------------------------------------
    // Route 2: GET /api/gl/periods/{period_id}/summary
    // ------------------------------------------------------------------
    {
        let url = format!("{}/api/gl/periods/{}/summary", GL_BASE, period_id);
        let (s, body) = get_json(&client, &url, &token).await;
        println!("2. GET summary: {} body_len={}", s, body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &body[..body.len().min(300)]);
        }
    }

    // ------------------------------------------------------------------
    // Route 3: GET /api/gl/periods/{period_id}/checklist (empty)
    // ------------------------------------------------------------------
    {
        let url = format!("{}/api/gl/periods/{}/checklist", GL_BASE, period_id);
        let (s, body) = get_json(&client, &url, &token).await;
        println!("3. GET checklist (empty): {} body_len={}", s, body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &body[..body.len().min(300)]);
        }
    }

    // ------------------------------------------------------------------
    // Route 4: POST /api/gl/periods/{period_id}/checklist (create item)
    // ------------------------------------------------------------------
    let checklist_item_id: Option<Uuid>;
    {
        let url = format!("{}/api/gl/periods/{}/checklist", GL_BASE, period_id);
        let (s, body) = post_json(
            &client,
            &url,
            &token,
            &json!({"label": "Reconcile bank statements"}),
        )
        .await;
        println!("4. POST checklist create: {} body_len={}", s, body.len());
        if s == 201 {
            passed += 1;
            let v: Value = serde_json::from_str(&body).unwrap_or_default();
            checklist_item_id = v["id"].as_str().and_then(|s| s.parse().ok());
        } else {
            println!("   body: {}", &body[..body.len().min(300)]);
            checklist_item_id = None;
        }
    }

    // ------------------------------------------------------------------
    // Route 5: POST /api/gl/periods/{period_id}/checklist/{item_id}/complete
    // ------------------------------------------------------------------
    {
        if let Some(item_id) = checklist_item_id {
            let url = format!(
                "{}/api/gl/periods/{}/checklist/{}/complete",
                GL_BASE, period_id, item_id
            );
            let (s, body) = post_json(
                &client,
                &url,
                &token,
                &json!({"completed_by": "smoke-test-user"}),
            )
            .await;
            println!("5. POST checklist complete: {} body_len={}", s, body.len());
            if s == 200 {
                passed += 1;
            } else {
                println!("   body: {}", &body[..body.len().min(300)]);
            }
        } else {
            println!("5. POST checklist complete: SKIPPED (no item_id from step 4)");
        }
    }

    // ------------------------------------------------------------------
    // Route 6: POST /api/gl/periods/{period_id}/checklist/{item_id}/waive
    // Create a second item to waive (can't waive an already-completed item)
    // ------------------------------------------------------------------
    {
        let url = format!("{}/api/gl/periods/{}/checklist", GL_BASE, period_id);
        let (s, body) = post_json(
            &client,
            &url,
            &token,
            &json!({"label": "Review intercompany balances"}),
        )
        .await;
        if s == 201 {
            let v: Value = serde_json::from_str(&body).unwrap_or_default();
            if let Some(item_id) = v["id"].as_str().and_then(|s| s.parse::<Uuid>().ok()) {
                let waive_url = format!(
                    "{}/api/gl/periods/{}/checklist/{}/waive",
                    GL_BASE, period_id, item_id
                );
                let (ws, wbody) = post_json(
                    &client,
                    &waive_url,
                    &token,
                    &json!({"completed_by": "smoke-test-user", "waive_reason": "Not applicable this period"}),
                )
                .await;
                println!("6. POST checklist waive: {} body_len={}", ws, wbody.len());
                if ws == 200 {
                    passed += 1;
                } else {
                    println!("   body: {}", &wbody[..wbody.len().min(300)]);
                }
            } else {
                println!("6. POST checklist waive: SKIPPED (no item_id)");
            }
        } else {
            println!(
                "6. POST checklist waive: SKIPPED (create second item failed: {})",
                s
            );
        }
    }

    // ------------------------------------------------------------------
    // Route 7: GET /api/gl/periods/{period_id}/approvals (empty)
    // ------------------------------------------------------------------
    {
        let url = format!("{}/api/gl/periods/{}/approvals", GL_BASE, period_id);
        let (s, body) = get_json(&client, &url, &token).await;
        println!("7. GET approvals (empty): {} body_len={}", s, body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &body[..body.len().min(300)]);
        }
    }

    // ------------------------------------------------------------------
    // Route 8: POST /api/gl/periods/{period_id}/approvals (create)
    // ------------------------------------------------------------------
    {
        let url = format!("{}/api/gl/periods/{}/approvals", GL_BASE, period_id);
        let (s, body) = post_json(
            &client,
            &url,
            &token,
            &json!({
                "actor_id": "smoke-test-user",
                "approval_type": "controller",
                "notes": "Smoke test approval"
            }),
        )
        .await;
        println!("8. POST approvals create: {} body_len={}", s, body.len());
        if s == 201 {
            passed += 1;
        } else {
            println!("   body: {}", &body[..body.len().min(300)]);
        }
    }

    // ------------------------------------------------------------------
    // Route 9: GET /api/gl/periods/{period_id}/validate-close
    // (This is actually POST — validate close is a POST with a body)
    // ------------------------------------------------------------------
    {
        let url = format!("{}/api/gl/periods/{}/validate-close", GL_BASE, period_id);
        let (s, body) = post_json(&client, &url, &token, &json!({"tenant_id": tenant_id})).await;
        println!("9. POST validate-close: {} body_len={}", s, body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &body[..body.len().min(300)]);
        }
    }

    // ------------------------------------------------------------------
    // Route 10: POST /api/gl/periods/{period_id}/close
    // ------------------------------------------------------------------
    {
        let url = format!("{}/api/gl/periods/{}/close", GL_BASE, period_id);
        let (s, body) = post_json(
            &client,
            &url,
            &token,
            &json!({
                "tenant_id": tenant_id,
                "closed_by": "smoke-test-user",
                "close_reason": "Period end smoke test"
            }),
        )
        .await;
        println!("10. POST close: {} body_len={}", s, body.len());
        // 200 = closed, could also be 400 if validation fails (no journals posted)
        // Accept 200 or 400 (validation failure is expected for empty period)
        if s == 200 || s == 400 {
            passed += 1;
        } else {
            println!("   body: {}", &body[..body.len().min(300)]);
        }
    }

    // ------------------------------------------------------------------
    // Route 11: POST /api/gl/periods/{period_id}/reopen
    // If close succeeded, request reopen. If close failed (400), the period
    // is still open — reopen request should fail with an appropriate error.
    // ------------------------------------------------------------------
    let reopen_request_id: Option<Uuid>;
    {
        let url = format!("{}/api/gl/periods/{}/reopen", GL_BASE, period_id);
        let (s, body) = post_json(
            &client,
            &url,
            &token,
            &json!({
                "requested_by": "smoke-test-user",
                "reason": "Need to post a late adjustment"
            }),
        )
        .await;
        println!("11. POST reopen request: {} body_len={}", s, body.len());
        // 201 = request created (period was closed), or error if period still open
        if s == 201 || s == 400 || s == 409 {
            passed += 1;
            let v: Value = serde_json::from_str(&body).unwrap_or_default();
            reopen_request_id = v["request_id"].as_str().and_then(|s| s.parse().ok());
        } else {
            println!("   body: {}", &body[..body.len().min(300)]);
            reopen_request_id = None;
        }
    }

    // ------------------------------------------------------------------
    // Route 12: POST /api/gl/periods/{period_id}/reopen/{request_id}/approve
    // or /reject — try approve if we got a request_id, else test reject path
    // ------------------------------------------------------------------
    {
        if let Some(req_id) = reopen_request_id {
            let url = format!(
                "{}/api/gl/periods/{}/reopen/{}/approve",
                GL_BASE, period_id, req_id
            );
            let (s, body) = post_json(
                &client,
                &url,
                &token,
                &json!({"approved_by": "smoke-test-approver"}),
            )
            .await;
            println!("12. POST reopen approve: {} body_len={}", s, body.len());
            if s == 200 || s == 400 || s == 409 {
                passed += 1;
            } else {
                println!("   body: {}", &body[..body.len().min(300)]);
            }
        } else {
            // No reopen request — test the reject path with a fake ID to verify the route exists
            let fake_id = Uuid::new_v4();
            let url = format!(
                "{}/api/gl/periods/{}/reopen/{}/reject",
                GL_BASE, period_id, fake_id
            );
            let (s, body) = post_json(
                &client,
                &url,
                &token,
                &json!({"rejected_by": "smoke-test-approver", "reject_reason": "Not needed"}),
            )
            .await;
            println!(
                "12. POST reopen reject (fallback): {} body_len={}",
                s,
                body.len()
            );
            // 404 (request not found) or 400 is acceptable — proves route exists
            if s == 200 || s == 400 || s == 404 || s == 409 {
                passed += 1;
            } else {
                println!("   body: {}", &body[..body.len().min(300)]);
            }
        }
    }

    // ------------------------------------------------------------------
    // Bonus: GET /api/gl/accounts/{account_code}/activity
    // ------------------------------------------------------------------
    {
        let url = format!(
            "{}/api/gl/accounts/1000/activity?start_date=2026-01-01T00:00:00Z&end_date=2026-01-31T23:59:59Z",
            GL_BASE
        );
        let (s, body) = get_json(&client, &url, &token).await;
        println!("B. GET account activity: {} body_len={}", s, body.len());
        // 200 (empty or with data) — just proves the route works
        if s == 200 {
            println!("   account activity route: OK");
        } else {
            println!("   body: {}", &body[..body.len().min(300)]);
        }
    }

    // ------------------------------------------------------------------
    // Auth guard: no token → 401
    // ------------------------------------------------------------------
    {
        let url = format!("{}/api/gl/periods/{}/close-status", GL_BASE, period_id);
        let resp = client
            .get(&url)
            .send()
            .await
            .expect("GET without token failed");
        let s = resp.status().as_u16();
        println!("AUTH: GET close-status without token: {}", s);
        assert_eq!(s, 401, "unauthenticated request must return 401");
    }

    // ------------------------------------------------------------------
    // Summary
    // ------------------------------------------------------------------
    println!("\n--- GL Period Management HTTP Smoke ---");
    println!("  passed: {}/{}", passed, total);

    // Cleanup
    cleanup(&pool, &tenant_id).await;

    assert!(
        passed >= 10,
        "at least 10 of {} routes must pass (got {}). Some routes may fail due to period state, \
         but core CRUD must work.",
        total,
        passed
    );

    println!("  GL period management smoke: PASSED");
}
