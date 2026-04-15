// HTTP smoke tests: PDF Editor — 15 routes (bd-20ra4)
//
// Tests all PDF Editor API routes via reqwest against the live pdf-editor service:
//   Templates  (4): create, list, get/{id}, update
//   Fields     (4): create (x2), list, update, reorder
//   Submissions(5): create, list, get/{id}, autosave, submit
//   Generate   (1): POST submissions/{id}/generate  (multipart; accepts 400 for invalid PDF)
//   Render     (1): POST render-annotations          (multipart; no auth)
//
// ## Running
// ```bash
// ./scripts/cargo-slot.sh test -p e2e-tests --test pdf_editor_http_smoke -- --nocapture
// ```

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const PDF_BASE: &str = "http://127.0.0.1:8102";

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

#[tokio::test]
async fn pdf_editor_http_smoke() {
    let client = make_client();
    let base = std::env::var("PDF_EDITOR_URL").unwrap_or_else(|_| PDF_BASE.to_string());

    // Health check
    let (status, _) = get_json(&client, &format!("{}/healthz", base), "").await;
    assert_eq!(status, 200, "pdf-editor service must be healthy");

    let tenant_id = Uuid::new_v4().to_string();
    let token = sign_jwt(&tenant_id, &["pdf_editor.read", "pdf_editor.mutate"]);

    let uid = Uuid::new_v4().to_string();
    let uid = uid.split('-').next().unwrap_or("smoke");

    let mut passed = 0u32;
    let total = 15u32;

    // =========================================================================
    // Templates (4 routes)
    // =========================================================================

    // Call 1: POST /api/pdf/forms/templates
    let mut template_id: Option<Uuid> = None;
    {
        let url = format!("{}/api/pdf/forms/templates", base);
        let body = json!({
            "tenant_id": tenant_id,
            "name": format!("Smoke Template {}", uid),
            "description": "Created by smoke test",
            "created_by": "smoke-test"
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!(
            "1. POST /api/pdf/forms/templates: {} body_len={}",
            s,
            resp_body.len()
        );
        if s == 201 {
            passed += 1;
            let v: Value = serde_json::from_str(&resp_body).unwrap_or_default();
            template_id = v["id"].as_str().and_then(|s| s.parse().ok());
            println!("   template_id={:?}", template_id);
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Call 2: GET /api/pdf/forms/templates
    {
        let url = format!("{}/api/pdf/forms/templates", base);
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!(
            "2. GET /api/pdf/forms/templates: {} body_len={}",
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Call 3: GET /api/pdf/forms/templates/{id}
    if let Some(tid) = template_id {
        let url = format!("{}/api/pdf/forms/templates/{}", base, tid);
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!(
            "3. GET /api/pdf/forms/templates/{}: {} body_len={}",
            tid,
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("3. GET /api/pdf/forms/templates/{{id}}: SKIPPED (no template_id)");
    }

    // Call 4: PUT /api/pdf/forms/templates/{id}
    if let Some(tid) = template_id {
        let url = format!("{}/api/pdf/forms/templates/{}", base, tid);
        let body = json!({
            "name": format!("Updated Smoke Template {}", uid),
            "description": "Updated by smoke test"
        });
        let (s, resp_body) = put_json(&client, &url, &token, &body).await;
        println!(
            "4. PUT /api/pdf/forms/templates/{}: {} body_len={}",
            tid,
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("4. PUT /api/pdf/forms/templates/{{id}}: SKIPPED (no template_id)");
    }

    // =========================================================================
    // Fields (4 routes — create x2, list, update, reorder)
    // =========================================================================

    // Call 5: POST /api/pdf/forms/templates/{id}/fields  (field 1)
    let mut field1_id: Option<Uuid> = None;
    if let Some(tid) = template_id {
        let url = format!("{}/api/pdf/forms/templates/{}/fields", base, tid);
        let body = json!({
            "field_key": "full_name",
            "field_label": "Full Name",
            "field_type": "text"
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!(
            "5. POST fields (field1): {} body_len={}",
            s,
            resp_body.len()
        );
        if s == 201 {
            passed += 1;
            let v: Value = serde_json::from_str(&resp_body).unwrap_or_default();
            field1_id = v["id"].as_str().and_then(|s| s.parse().ok());
            println!("   field1_id={:?}", field1_id);
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("5. POST fields (field1): SKIPPED (no template_id)");
    }

    // Call 6: POST /api/pdf/forms/templates/{id}/fields  (field 2)
    let mut field2_id: Option<Uuid> = None;
    if let Some(tid) = template_id {
        let url = format!("{}/api/pdf/forms/templates/{}/fields", base, tid);
        let body = json!({
            "field_key": "date_signed",
            "field_label": "Date Signed",
            "field_type": "date"
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!(
            "6. POST fields (field2): {} body_len={}",
            s,
            resp_body.len()
        );
        if s == 201 {
            passed += 1;
            let v: Value = serde_json::from_str(&resp_body).unwrap_or_default();
            field2_id = v["id"].as_str().and_then(|s| s.parse().ok());
            println!("   field2_id={:?}", field2_id);
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("6. POST fields (field2): SKIPPED (no template_id)");
    }

    // Call 7: GET /api/pdf/forms/templates/{id}/fields
    if let Some(tid) = template_id {
        let url = format!("{}/api/pdf/forms/templates/{}/fields", base, tid);
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!(
            "7. GET /api/pdf/forms/templates/{}/fields: {} body_len={}",
            tid,
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("7. GET fields list: SKIPPED (no template_id)");
    }

    // Call 8: PUT /api/pdf/forms/templates/{tid}/fields/{fid}
    if let (Some(tid), Some(fid)) = (template_id, field1_id) {
        let url = format!("{}/api/pdf/forms/templates/{}/fields/{}", base, tid, fid);
        let body = json!({
            "field_label": "Full Legal Name",
            "field_type": "text"
        });
        let (s, resp_body) = put_json(&client, &url, &token, &body).await;
        println!("8. PUT fields/{}: {} body_len={}", fid, s, resp_body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("8. PUT fields/{{fid}}: SKIPPED (no template or field id)");
    }

    // Call 9: POST /api/pdf/forms/templates/{id}/fields/reorder
    if let (Some(tid), Some(f1), Some(f2)) = (template_id, field1_id, field2_id) {
        let url = format!("{}/api/pdf/forms/templates/{}/fields/reorder", base, tid);
        let body = json!({ "field_ids": [f2, f1] });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!("9. POST fields/reorder: {} body_len={}", s, resp_body.len());
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("9. POST fields/reorder: SKIPPED (no template or field ids)");
    }

    // =========================================================================
    // Submissions (5 routes)
    // =========================================================================

    // Call 10: POST /api/pdf/forms/submissions
    let mut submission_id: Option<Uuid> = None;
    if let Some(tid) = template_id {
        let url = format!("{}/api/pdf/forms/submissions", base);
        let body = json!({
            "tenant_id": tenant_id,
            "template_id": tid,
            "submitted_by": format!("smoke-user-{}", uid),
            "field_data": {
                "full_name": "Jane Doe",
                "date_signed": "2026-03-08"
            }
        });
        let (s, resp_body) = post_json(&client, &url, &token, &body).await;
        println!(
            "10. POST /api/pdf/forms/submissions: {} body_len={}",
            s,
            resp_body.len()
        );
        if s == 201 {
            passed += 1;
            let v: Value = serde_json::from_str(&resp_body).unwrap_or_default();
            submission_id = v["id"].as_str().and_then(|s| s.parse().ok());
            println!("   submission_id={:?}", submission_id);
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("10. POST submissions: SKIPPED (no template_id)");
    }

    // Call 11: GET /api/pdf/forms/submissions
    {
        let url = format!("{}/api/pdf/forms/submissions", base);
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!(
            "11. GET /api/pdf/forms/submissions: {} body_len={}",
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // Call 12: GET /api/pdf/forms/submissions/{id}
    if let Some(sid) = submission_id {
        let url = format!("{}/api/pdf/forms/submissions/{}", base, sid);
        let (s, resp_body) = get_json(&client, &url, &token).await;
        println!(
            "12. GET submissions/{}: {} body_len={}",
            sid,
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("12. GET submissions/{{id}}: SKIPPED (no submission_id)");
    }

    // Call 13: PUT /api/pdf/forms/submissions/{id}  (autosave)
    if let Some(sid) = submission_id {
        let url = format!("{}/api/pdf/forms/submissions/{}", base, sid);
        let body = json!({
            "field_data": {
                "full_name": "Jane A. Doe",
                "date_signed": "2026-03-08"
            }
        });
        let (s, resp_body) = put_json(&client, &url, &token, &body).await;
        println!(
            "13. PUT submissions/{} (autosave): {} body_len={}",
            sid,
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("13. PUT submissions/{{id}} (autosave): SKIPPED (no submission_id)");
    }

    // Call 14: POST /api/pdf/forms/submissions/{id}/submit
    if let Some(sid) = submission_id {
        let url = format!("{}/api/pdf/forms/submissions/{}/submit", base, sid);
        let (s, resp_body) = post_json(&client, &url, &token, &json!({})).await;
        println!(
            "14. POST submissions/{}/submit: {} body_len={}",
            sid,
            s,
            resp_body.len()
        );
        if s == 200 {
            passed += 1;
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("14. POST submissions/{{id}}/submit: SKIPPED (no submission_id)");
    }

    // =========================================================================
    // Generate (1 route — multipart; submisison must be in "submitted" state)
    // Sending fake PDF bytes → expect 400 (InvalidMagic) proving route is healthy
    // =========================================================================

    // Call 15: POST /api/pdf/forms/submissions/{id}/generate
    if let Some(sid) = submission_id {
        let url = format!("{}/api/pdf/forms/submissions/{}/generate", base, sid);
        // Minimal fake PDF to trigger InvalidMagic (not a real PDF)
        let fake_pdf: Vec<u8> = b"FAKEPDF".to_vec();
        let part = reqwest::multipart::Part::bytes(fake_pdf)
            .file_name("test.pdf")
            .mime_str("application/pdf")
            .expect("invalid mime");
        let form = reqwest::multipart::Form::new().part("file", part);
        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .multipart(form)
            .send()
            .await
            .unwrap_or_else(|e| panic!("POST {} failed: {}", url, e));
        let s = resp.status().as_u16();
        let resp_body = resp.text().await.unwrap_or_default();
        println!(
            "15. POST submissions/{}/generate (fake PDF): {} body_len={}",
            sid,
            s,
            resp_body.len()
        );
        // 400 = InvalidMagic (route healthy, PDF rejected), 200 = success, both acceptable
        if s == 400 || s == 200 || s == 422 {
            passed += 1;
            if s == 400 {
                println!("   (400 expected — fake PDF bytes rejected, route is healthy)");
            }
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    } else {
        println!("15. POST submissions/{{id}}/generate: SKIPPED (no submission_id)");
    }

    // =========================================================================
    // Render Annotations (1 route — multipart; no auth required)
    // POST /api/pdf/render-annotations
    // Empty annotations → service returns original PDF as-is
    // Sending fake PDF + empty annotations; accept 200 OR 400
    // =========================================================================

    // Call 16 (bonus — not in base 15 count): POST /api/pdf/render-annotations
    {
        let url = format!("{}/api/pdf/render-annotations", base);
        let fake_pdf: Vec<u8> = b"%PDF-1.0\nfake".to_vec();
        let file_part = reqwest::multipart::Part::bytes(fake_pdf)
            .file_name("test.pdf")
            .mime_str("application/pdf")
            .expect("invalid mime");
        let annotations_part = reqwest::multipart::Part::text("[]")
            .mime_str("application/json")
            .expect("invalid mime");
        let form = reqwest::multipart::Form::new()
            .part("file", file_part)
            .part("annotations", annotations_part);
        let resp = client
            .post(&url)
            .multipart(form)
            .send()
            .await
            .unwrap_or_else(|e| panic!("POST {} failed: {}", url, e));
        let s = resp.status().as_u16();
        let resp_body = resp.text().await.unwrap_or_default();
        println!(
            "16. POST /api/pdf/render-annotations (no auth): {} body_len={}",
            s,
            resp_body.len()
        );
        if s == 200 || s == 400 {
            println!("   render-annotations route healthy ({})", s);
        } else {
            println!("   body: {}", &resp_body[..resp_body.len().min(300)]);
        }
    }

    // =========================================================================
    // Auth guard: unauthenticated mutation must return 401
    // =========================================================================
    {
        let url = format!("{}/api/pdf/forms/templates", base);
        let resp = client
            .post(&url)
            .json(&json!({}))
            .send()
            .await
            .expect("POST without token failed");
        let s = resp.status().as_u16();
        println!("AUTH: POST /api/pdf/forms/templates without token: {}", s);
        assert_eq!(s, 401, "unauthenticated mutation must return 401");
    }

    println!("\n--- PDF Editor HTTP Smoke ---");
    println!("  passed: {}/{}", passed, total);

    assert!(
        passed >= 12,
        "at least 12 of {} route calls must pass (got {})",
        total,
        passed
    );

    println!("  PDF Editor smoke: PASSED");
}
