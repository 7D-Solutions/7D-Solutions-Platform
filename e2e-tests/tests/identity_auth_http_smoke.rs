// HTTP smoke: Identity-Auth (bd-28qp9)
//
// Tests 8 untested routes via reqwest against the live identity-auth service.
// Routes: access-review, lifecycle, sod/policies CRUD, forgot-password, reset-password.
//
// ## Running
// ```bash
// ./scripts/cargo-slot.sh test -p e2e-tests --test identity_auth_http_smoke -- --nocapture
// ```

use reqwest::Client;
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use uuid::Uuid;

const AUTH_DEFAULT: &str = "http://localhost:8080";
const AUTH_DB_DEFAULT: &str = "postgres://auth_user:auth_pass@localhost:5433/auth_db";

fn auth_base() -> String {
    std::env::var("IDENTITY_AUTH_URL").unwrap_or_else(|_| AUTH_DEFAULT.to_string())
}

async fn auth_pool() -> sqlx::PgPool {
    dotenvy::from_filename_override(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.env"),
    )
    .ok();
    let url = std::env::var("AUTH_DATABASE_URL").unwrap_or_else(|_| AUTH_DB_DEFAULT.to_string());
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to auth DB")
}

fn make_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("failed to create HTTP client")
}

async fn post_json(client: &Client, url: &str, body: &Value) -> (u16, String) {
    let resp = client
        .post(url)
        .json(body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST {} failed: {}", url, e));
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    (status, text)
}

async fn get_json(client: &Client, url: &str) -> (u16, String) {
    let resp = client
        .get(url)
        .send()
        .await
        .unwrap_or_else(|e| panic!("GET {} failed: {}", url, e));
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    (status, text)
}

async fn delete_req(client: &Client, url: &str) -> (u16, String) {
    let resp = client
        .delete(url)
        .send()
        .await
        .unwrap_or_else(|e| panic!("DELETE {} failed: {}", url, e));
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    (status, text)
}

#[tokio::test]
async fn identity_auth_http_smoke() {
    let client = make_client();
    let base = auth_base();

    let (status, _) = get_json(&client, &format!("{}/healthz", base)).await;
    assert_eq!(status, 200, "identity-auth service must be healthy");

    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let reviewer_id = Uuid::new_v4();
    let short = Uuid::new_v4().to_string();
    let short = short.split('-').next().unwrap_or("smoke");
    let email = format!("smoke-{}@example.com", short);
    let password = "SmokeTest1!Secure";

    let mut passed = 0u32;
    let total = 8u32;

    // Setup: register a user so access-review and lifecycle routes have real data
    {
        let url = format!("{}/api/auth/register", base);
        let body = json!({
            "tenant_id": tenant_id,
            "user_id": user_id,
            "email": email,
            "password": password
        });
        let (s, body_text) = post_json(&client, &url, &body).await;
        println!(
            "SETUP: POST register: {} (tenant={} user={})",
            s, tenant_id, user_id
        );
        if s != 200 && s != 409 {
            println!(
                "  WARNING: registration failed: {}",
                &body_text[..body_text.len().min(300)]
            );
        }
    }

    // Route 1: POST /api/auth/access-review
    {
        let url = format!("{}/api/auth/access-review", base);
        let review_id = Uuid::new_v4();
        let body = json!({
            "tenant_id": tenant_id,
            "user_id": user_id,
            "review_id": review_id,
            "reviewed_by": reviewer_id,
            "decision": "approved",
            "notes": "Quarterly access review - all permissions confirmed",
            "idempotency_key": null,
            "causation_id": null
        });
        let (s, resp_body) = post_json(&client, &url, &body).await;
        println!("1. POST access-review: {} body_len={}", s, resp_body.len());
        if s == 200 {
            passed += 1;
            let lower = resp_body.to_lowercase();
            assert!(
                !lower.contains("syntax error") && !lower.contains("pg_"),
                "SQL trace in access-review response"
            );
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Route 2: GET /api/auth/lifecycle/{tenant_id}/{user_id}
    {
        let url = format!("{}/api/auth/lifecycle/{}/{}", base, tenant_id, user_id);
        let (s, resp_body) = get_json(&client, &url).await;
        println!("2. GET lifecycle: {} body_len={}", s, resp_body.len());
        if s == 200 {
            passed += 1;
            let lower = resp_body.to_lowercase();
            assert!(
                !lower.contains("syntax error") && !lower.contains("pg_"),
                "SQL trace in lifecycle response"
            );
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // SoD setup: create real roles in the DB (sod_policies has FK constraints on role IDs)
    let pool = auth_pool().await;
    let primary_role_id = sqlx::query_scalar::<_, Uuid>(
        r#"INSERT INTO roles (tenant_id, name, description)
           VALUES ($1, $2, 'Smoke primary role')
           RETURNING id"#,
    )
    .bind(tenant_id)
    .bind(format!("smoke-primary-{}", Uuid::new_v4()))
    .fetch_one(&pool)
    .await
    .expect("create primary role");

    let conflicting_role_id = sqlx::query_scalar::<_, Uuid>(
        r#"INSERT INTO roles (tenant_id, name, description)
           VALUES ($1, $2, 'Smoke conflicting role')
           RETURNING id"#,
    )
    .bind(tenant_id)
    .bind(format!("smoke-conflict-{}", Uuid::new_v4()))
    .fetch_one(&pool)
    .await
    .expect("create conflicting role");

    let action_short = Uuid::new_v4().to_string();
    let action_short = action_short.split('-').next().unwrap_or("act");
    let action_key = format!("approve_po_{}", action_short);

    // Route 3: POST /api/auth/sod/policies (upsert)
    let mut sod_policy_id: Option<Uuid> = None;
    {
        let url = format!("{}/api/auth/sod/policies", base);
        let body = json!({
            "tenant_id": tenant_id,
            "action_key": action_key,
            "primary_role_id": primary_role_id,
            "conflicting_role_id": conflicting_role_id,
            "allow_override": false,
            "override_requires_approval": false,
            "actor_user_id": null,
            "idempotency_key": null,
            "causation_id": null
        });
        let (s, resp_body) = post_json(&client, &url, &body).await;
        println!("3. POST sod/policies: {} body_len={}", s, resp_body.len());
        if s == 200 {
            passed += 1;
            let v: Value = serde_json::from_str(&resp_body).unwrap_or_default();
            sod_policy_id = v["policy"]["id"].as_str().and_then(|s| s.parse().ok());
            println!("   policy_id: {:?}", sod_policy_id);
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Route 4: POST /api/auth/sod/evaluate
    {
        let url = format!("{}/api/auth/sod/evaluate", base);
        let body = json!({
            "tenant_id": tenant_id,
            "action_key": action_key,
            "actor_user_id": user_id,
            "subject_user_id": null,
            "override_granted_by": null,
            "override_ticket": null,
            "idempotency_key": null,
            "causation_id": null
        });
        let (s, resp_body) = post_json(&client, &url, &body).await;
        println!("4. POST sod/evaluate: {} body_len={}", s, resp_body.len());
        if s == 200 {
            passed += 1;
            let v: Value = serde_json::from_str(&resp_body).unwrap_or_default();
            let decision = v["decision"].as_str().unwrap_or("unknown");
            println!("   SoD decision: {}", decision);
            assert!(
                ["allow", "deny", "allow_with_override"].contains(&decision),
                "unexpected SoD decision: {}",
                decision
            );
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Route 5: GET /api/auth/sod/policies/{tenant_id}/by-action/{action_key}
    {
        let url = format!(
            "{}/api/auth/sod/policies/{}/by-action/{}",
            base, tenant_id, action_key
        );
        let (s, resp_body) = get_json(&client, &url).await;
        println!(
            "5. GET sod/policies by-action: {} body_len={}",
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Route 6: DELETE /api/auth/sod/policies/{tenant_id}/{rule_id}
    {
        if let Some(pid) = sod_policy_id {
            let url = format!("{}/api/auth/sod/policies/{}/{}", base, tenant_id, pid);
            let (s, resp_body) = delete_req(&client, &url).await;
            println!(
                "6. DELETE sod/policies/{}: {} body_len={}",
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
            println!("6. DELETE sod/policies: SKIPPED (no policy_id from step 3)");
        }
    }

    // Route 7: POST /api/auth/forgot-password
    // Always returns 200 — no user enumeration
    {
        let url = format!("{}/api/auth/forgot-password", base);
        let body = json!({"email": email});
        let (s, resp_body) = post_json(&client, &url, &body).await;
        println!(
            "7. POST forgot-password: {} body_len={}",
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
            // Non-existent email must also return 200
            let body2 = json!({"email": "nonexistent-smoke@nowhere.invalid"});
            let (s2, _) = post_json(&client, &url, &body2).await;
            assert_eq!(
                s2, 200,
                "forgot-password must return 200 for non-existent email (no enumeration)"
            );
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Route 8: POST /api/auth/reset-password
    // Real tokens are published via NATS and not returned over HTTP.
    // Test with an invalid token to prove the route is live.
    // 400 = invalid/expired token = correct rejection; no SQL traces.
    {
        let url = format!("{}/api/auth/reset-password", base);
        let body = json!({
            "token": "invalid-smoke-token-route-liveness",
            "new_password": "NewPassword1!Secure"
        });
        let (s, resp_body) = post_json(&client, &url, &body).await;
        println!(
            "8. POST reset-password (invalid token): {} body_len={}",
            s,
            resp_body.len()
        );
        let lower = resp_body.to_lowercase();
        assert!(
            !lower.contains("syntax error") && !lower.contains("pg_"),
            "SQL trace in reset-password error response"
        );
        if s == 400 || s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    println!("\n--- Identity-Auth HTTP Smoke ---");
    println!("  passed: {}/{}", passed, total);

    assert!(
        passed >= 6,
        "at least 6 of {} routes must pass (got {})",
        total,
        passed
    );

    println!("  Identity-Auth smoke: PASSED");
}
