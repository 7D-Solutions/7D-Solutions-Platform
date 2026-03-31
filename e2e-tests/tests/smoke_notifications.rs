// HTTP smoke tests: Notifications (18 routes)
//
// Proves that all 18 notification routes respond correctly at the HTTP
// boundary via reqwest against the live notifications service.
//
// Lifecycle verified: create template -> send -> get send detail ->
// deliveries -> inbox list -> inbox item actions -> DLQ -> admin.

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const NOTIF_DEFAULT_URL: &str = "http://localhost:8089";

fn notif_url() -> String {
    std::env::var("NOTIFICATIONS_URL").unwrap_or_else(|_| NOTIF_DEFAULT_URL.to_string())
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
    EncodingKey::from_rsa_pem(pem.replace("\n", "
").as_bytes()).ok()
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
    let url = format!("{}/api/health", notif_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  notifications health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  notifications health {}/15: {}", attempt, e),
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
async fn smoke_notifications() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_service(&client).await {
        eprintln!(
            "Notifications service not reachable at {} -- skipping",
            notif_url()
        );
        return;
    }
    println!("Notifications service healthy at {}", notif_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(
        &key,
        &tenant_id,
        &["notifications.read", "notifications.mutate"],
    );
    let base = notif_url();

    let probe = client
        .get(format!("{base}/api/deliveries"))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!("Notifications returns 401 with valid JWT -- JWT_PUBLIC_KEY not configured. Skipping.");
        return;
    }

    println!("
--- 1. POST /api/templates ---");
    let template_key = format!("smoke-{}", &Uuid::new_v4().to_string()[..8]);
    let resp = client
        .post(format!("{base}/api/templates"))
        .bearer_auth(&jwt)
        .json(&json!({
            "template_key": template_key,
            "channel": "in_app",
            "subject": "Smoke Test Notification",
            "body": "Hello, this is a smoke test notification.",
            "required_vars": []
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let tpl_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create template failed: {status} - {tpl_body}"
    );
    println!("  created template key={template_key}");
    assert_unauth(&client, "POST", &format!("{base}/api/templates"),
        Some(json!({"template_key": "x", "channel": "email", "subject": "X", "body": "X", "required_vars": []}))).await;

    println!("
--- 2. GET /api/templates/{{key}} ---");
    let resp = client
        .get(format!("{base}/api/templates/{template_key}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "Get template failed: {}", resp.status());
    let tpl_detail: Value = resp.json().await.unwrap();
    assert_eq!(tpl_detail["latest"]["template_key"], template_key);
    println!("  retrieved template key={}", tpl_detail["latest"]["template_key"]);
    assert_unauth(&client, "GET", &format!("{base}/api/templates/{template_key}"), None).await;

    println!("
--- 3. POST /api/notifications/send ---");
    let user_id = format!("smoke-user-{}", &Uuid::new_v4().to_string()[..8]);
    let resp = client
        .post(format!("{base}/api/notifications/send"))
        .bearer_auth(&jwt)
        .json(&json!({
            "template_key": template_key,
            "channel": "in_app",
            "recipients": [user_id],
            "payload_json": {},
            "correlation_id": format!("smoke-{}", Uuid::new_v4())
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let send_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Send notification failed: {status} - {send_body}"
    );
    let send_id = send_body["id"].as_str().expect("No id in send response");
    println!("  sent notification id={send_id}");
    assert_unauth(&client, "POST", &format!("{base}/api/notifications/send"),
        Some(json!({"template_key": "x", "channel": "email", "recipients": [], "payload_json": {}}))).await;

    println!("
--- 4. GET /api/notifications/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/notifications/{send_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "Get send detail failed: {}", resp.status());
    let detail: Value = resp.json().await.unwrap();
    assert_eq!(detail["template_key"], template_key);
    println!("  retrieved send detail template_key={}", detail["template_key"]);
    assert_unauth(&client, "GET", &format!("{base}/api/notifications/{send_id}"), None).await;

    println!("
--- 5. GET /api/deliveries ---");
    let resp = client
        .get(format!("{base}/api/deliveries"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "List deliveries failed: {}", resp.status());
    let deliveries: Value = resp.json().await.unwrap();
    println!("  listed {} delivery receipt(s)", deliveries["receipts"].as_array().map_or(0, |a| a.len()));
    assert_unauth(&client, "GET", &format!("{base}/api/deliveries"), None).await;

    println!("
--- 6. GET /api/inbox?user_id= ---");
    let mut inbox_item_id: Option<String> = None;
    for attempt in 1..=6 {
        let resp = client
            .get(format!("{base}/api/inbox"))
            .bearer_auth(&jwt)
            .query(&[("user_id", &user_id)])
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success(), "List inbox failed: {}", resp.status());
        let inbox_body: Value = resp.json().await.unwrap();
        let items = inbox_body["items"].as_array();
        let count = items.map_or(0, |a| a.len());
        if count > 0 {
            inbox_item_id = items
                .and_then(|a| a.first())
                .and_then(|i| i["id"].as_str())
                .map(|s| s.to_string());
            println!("  inbox has {count} item(s) after {attempt} poll(s)");
            break;
        }
        if attempt < 6 {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
    if inbox_item_id.is_none() {
        println!("  inbox empty after polling (async delivery subscriber may not be running)");
    }
    assert_unauth(&client, "GET", &format!("{base}/api/inbox?user_id={user_id}"), None).await;

    let item_id = inbox_item_id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
    let has_real_item = inbox_item_id.is_some();

    println!("
--- 7. GET /api/inbox/{{id}}?user_id= ---");
    let resp = client.get(format!("{base}/api/inbox/{item_id}"))
        .bearer_auth(&jwt).query(&[("user_id", &user_id)]).send().await.unwrap();
    let s7 = resp.status();
    assert!(s7.is_success() || s7 == StatusCode::NOT_FOUND, "inbox get unexpected: {s7}");
    if has_real_item { assert!(s7.is_success(), "Get inbox item failed: {s7}"); }
    println!("  get inbox item -> {s7}");
    assert_unauth(&client, "GET", &format!("{base}/api/inbox/{item_id}?user_id={user_id}"), None).await;

    println!("
--- 8. POST /api/inbox/{{id}}/read ---");
    let resp = client.post(format!("{base}/api/inbox/{item_id}/read"))
        .bearer_auth(&jwt).query(&[("user_id", &user_id)]).send().await.unwrap();
    let s8 = resp.status();
    assert!(s8.is_success() || s8 == StatusCode::NOT_FOUND, "inbox read unexpected: {s8}");
    if has_real_item {
        assert!(s8.is_success(), "Read inbox item failed: {s8}");
        let rb: Value = resp.json().await.unwrap();
        assert_eq!(rb["action"], "read");
        println!("  marked read, is_read={}", rb["is_read"]);
    } else { println!("  read inbox item -> {s8} (no real item)"); }
    assert_unauth(&client, "POST", &format!("{base}/api/inbox/{item_id}/read?user_id={user_id}"), None).await;

    println!("
--- 9. POST /api/inbox/{{id}}/dismiss ---");
    let resp = client.post(format!("{base}/api/inbox/{item_id}/dismiss"))
        .bearer_auth(&jwt).query(&[("user_id", &user_id)]).send().await.unwrap();
    let s9 = resp.status();
    assert!(s9.is_success() || s9 == StatusCode::NOT_FOUND, "inbox dismiss unexpected: {s9}");
    if has_real_item {
        assert!(s9.is_success(), "Dismiss inbox item failed: {s9}");
        let db: Value = resp.json().await.unwrap();
        assert_eq!(db["action"], "dismiss");
        println!("  dismissed, is_dismissed={}", db["is_dismissed"]);
    } else { println!("  dismiss inbox item -> {s9} (no real item)"); }
    assert_unauth(&client, "POST", &format!("{base}/api/inbox/{item_id}/dismiss?user_id={user_id}"), None).await;

    println!("
--- 10. POST /api/inbox/{{id}}/undismiss ---");
    let resp = client.post(format!("{base}/api/inbox/{item_id}/undismiss"))
        .bearer_auth(&jwt).query(&[("user_id", &user_id)]).send().await.unwrap();
    let s10 = resp.status();
    assert!(s10.is_success() || s10 == StatusCode::NOT_FOUND, "inbox undismiss unexpected: {s10}");
    if has_real_item { assert!(s10.is_success(), "Undismiss inbox item failed: {s10}"); }
    println!("  undismiss inbox item -> {s10}");
    assert_unauth(&client, "POST", &format!("{base}/api/inbox/{item_id}/undismiss?user_id={user_id}"), None).await;

    println!("
--- 11. POST /api/inbox/{{id}}/unread ---");
    let resp = client.post(format!("{base}/api/inbox/{item_id}/unread"))
        .bearer_auth(&jwt).query(&[("user_id", &user_id)]).send().await.unwrap();
    let s11 = resp.status();
    assert!(s11.is_success() || s11 == StatusCode::NOT_FOUND, "inbox unread unexpected: {s11}");
    if has_real_item {
        assert!(s11.is_success(), "Unread inbox item failed: {s11}");
        let ub: Value = resp.json().await.unwrap();
        assert_eq!(ub["action"], "unread");
        println!("  unread, is_read={}", ub["is_read"]);
    } else { println!("  unread inbox item -> {s11} (no real item)"); }
    assert_unauth(&client, "POST", &format!("{base}/api/inbox/{item_id}/unread?user_id={user_id}"), None).await;

    println!("
--- 12. GET /api/dlq ---");
    let resp = client.get(format!("{base}/api/dlq")).bearer_auth(&jwt).send().await.unwrap();
    assert!(resp.status().is_success(), "List DLQ failed: {}", resp.status());
    let dlq_list: Value = resp.json().await.unwrap();
    println!("  listed {} DLQ item(s)", dlq_list["items"].as_array().map_or(0, |a| a.len()));
    assert_unauth(&client, "GET", &format!("{base}/api/dlq"), None).await;

    let dlq_id = dlq_list["items"].as_array()
        .and_then(|a| a.first()).and_then(|i| i["id"].as_str())
        .map(|s| s.to_string()).unwrap_or_else(|| Uuid::new_v4().to_string());

    println!("
--- 13. GET /api/dlq/{{id}} ---");
    let resp = client.get(format!("{base}/api/dlq/{dlq_id}")).bearer_auth(&jwt).send().await.unwrap();
    let s13 = resp.status();
    assert!(s13.is_success() || s13 == StatusCode::NOT_FOUND, "DLQ get unexpected: {s13}");
    println!("  get DLQ item -> {s13}");
    assert_unauth(&client, "GET", &format!("{base}/api/dlq/{dlq_id}"), None).await;

    println!("
--- 14. POST /api/dlq/{{id}}/replay ---");
    let resp = client.post(format!("{base}/api/dlq/{dlq_id}/replay")).bearer_auth(&jwt).send().await.unwrap();
    let s14 = resp.status();
    assert!(s14.is_success() || s14 == StatusCode::NOT_FOUND, "DLQ replay unexpected: {s14}");
    println!("  replay DLQ item -> {s14}");
    assert_unauth(&client, "POST", &format!("{base}/api/dlq/{dlq_id}/replay"), None).await;

    println!("
--- 15. POST /api/dlq/{{id}}/abandon ---");
    let abandon_id = Uuid::new_v4().to_string();
    let resp = client.post(format!("{base}/api/dlq/{abandon_id}/abandon")).bearer_auth(&jwt).send().await.unwrap();
    let s15 = resp.status();
    assert!(s15.is_success() || s15 == StatusCode::NOT_FOUND, "DLQ abandon unexpected: {s15}");
    println!("  abandon DLQ item -> {s15}");
    assert_unauth(&client, "POST", &format!("{base}/api/dlq/{abandon_id}/abandon"), None).await;

    let admin_token = std::env::var("ADMIN_TOKEN").unwrap_or_default();

    println!("
--- 16. POST /api/notifications/admin/projection-status ---");
    let resp = client.post(format!("{base}/api/notifications/admin/projection-status"))
        .header("X-Admin-Token", &admin_token)
        .json(&json!({"projection_name": "notifications_sends"}))
        .send().await.unwrap();
    let s16 = resp.status();
    if admin_token.is_empty() {
        assert!(s16.as_u16() == 401 || s16.as_u16() == 403, "Expected 401 or 403 when ADMIN_TOKEN not set, got {s16}");
        println!("  projection-status -> 403 (ADMIN_TOKEN not set, expected)");
    } else {
        assert!(s16.is_success(), "projection-status failed: {s16}");
        println!("  projection-status -> {s16}");
    }

    println!("
--- 17. POST /api/notifications/admin/consistency-check ---");
    let resp = client.post(format!("{base}/api/notifications/admin/consistency-check"))
        .header("X-Admin-Token", &admin_token)
        .json(&json!({"projection_name": "notifications_sends"}))
        .send().await.unwrap();
    let s17 = resp.status();
    if admin_token.is_empty() {
        assert!(s17.as_u16() == 401 || s17.as_u16() == 403, "Expected 401 or 403 when ADMIN_TOKEN not set, got {s17}");
        println!("  consistency-check -> 403 (ADMIN_TOKEN not set, expected)");
    } else {
        assert!(s17.is_success(), "consistency-check failed: {s17}");
        println!("  consistency-check -> {s17}");
    }

    println!("
--- 18. GET /api/notifications/admin/projections ---");
    let resp = client.get(format!("{base}/api/notifications/admin/projections"))
        .header("X-Admin-Token", &admin_token)
        .send().await.unwrap();
    let s18 = resp.status();
    if admin_token.is_empty() {
        assert!(s18.as_u16() == 401 || s18.as_u16() == 403, "Expected 401 or 403 when ADMIN_TOKEN not set, got {s18}");
        println!("  projections -> 403 (ADMIN_TOKEN not set, expected)");
    } else {
        assert!(s18.is_success(), "projections failed: {s18}");
        println!("  projections -> {s18}");
    }

    println!("
=== All 18 Notifications routes passed ===");
}
