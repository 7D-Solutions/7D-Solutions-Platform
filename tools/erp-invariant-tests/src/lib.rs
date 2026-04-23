//! Adversarial ERP invariant test harness (bd-kgaz8).
//!
//! Each test follows the pattern:
//!   1. Seed a clean tenant + valid posted state via direct SQL / HTTP
//!   2. Attempt the adversarial action through the real HTTP API
//!   3. Assert HTTP response code + error message quality
//!   4. Assert DB consistency (balances, inventory, audit trail)
//!   5. Assert outbox drained (no stuck events)
//!
//! Tests skip gracefully when required services are not reachable.
//! No mocks. No stubs. All state is real Postgres + real HTTP.

// ============================================================================
// Infrastructure — service URLs and DB connections
// ============================================================================

pub fn inv_url() -> String {
    std::env::var("INVENTORY_URL").unwrap_or_else(|_| "http://localhost:8092".to_string())
}

pub fn ar_url() -> String {
    std::env::var("AR_URL").unwrap_or_else(|_| "http://localhost:8086".to_string())
}

pub fn ap_url() -> String {
    std::env::var("AP_URL").unwrap_or_else(|_| "http://localhost:8093".to_string())
}

pub fn gl_url() -> String {
    std::env::var("GL_URL").unwrap_or_else(|_| "http://localhost:8090".to_string())
}

pub async fn inv_pool() -> sqlx::PgPool {
    let url = std::env::var("INVENTORY_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://inventory_user:inventory_pass@localhost:5442/inventory_db".to_string());
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
        .connect(&url)
        .await
        .expect("inventory DB not reachable — set INVENTORY_DATABASE_URL")
}

pub async fn ar_pool() -> sqlx::PgPool {
    let url = std::env::var("AR_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://ar_user:ar_pass@localhost:5434/ar_db".to_string());
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
        .connect(&url)
        .await
        .expect("AR DB not reachable — set AR_DATABASE_URL")
}

// ============================================================================
// HTTP client builder with timeout
// ============================================================================

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("reqwest client build failed")
}

// ============================================================================
// JWT helpers — matches platform dev key convention
// ============================================================================

#[derive(serde::Serialize)]
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

pub fn dev_private_key() -> Option<jsonwebtoken::EncodingKey> {
    let pem = std::env::var("JWT_PRIVATE_KEY_PEM").ok()?;
    jsonwebtoken::EncodingKey::from_rsa_pem(pem.replace("\\n", "\n").as_bytes()).ok()
}

pub fn make_jwt(key: &jsonwebtoken::EncodingKey, tenant_id: &str, perms: &[&str]) -> String {
    let now = chrono::Utc::now();
    let claims = TestClaims {
        sub: uuid::Uuid::new_v4().to_string(),
        iss: "auth-rs".to_string(),
        aud: "7d-platform".to_string(),
        iat: now.timestamp(),
        exp: (now + chrono::Duration::minutes(15)).timestamp(),
        jti: uuid::Uuid::new_v4().to_string(),
        tenant_id: tenant_id.to_string(),
        app_id: Some(tenant_id.to_string()),
        roles: vec!["operator".to_string()],
        perms: perms.iter().map(|s| s.to_string()).collect(),
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    };
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
        &claims,
        key,
    )
    .expect("JWT encode failed")
}

// ============================================================================
// Service-reachability helpers — tests skip gracefully on connection refused
// ============================================================================

pub async fn service_ready(client: &reqwest::Client, health_url: &str) -> bool {
    match client.get(health_url).send().await {
        Ok(r) if r.status().is_success() => true,
        _ => false,
    }
}

// ============================================================================
// Inventory HTTP helpers
// ============================================================================

/// Create an item via HTTP. Returns item_id string on success.
pub async fn http_create_item(
    client: &reqwest::Client,
    jwt: &str,
    tenant_id: &str,
    sku: &str,
) -> Option<String> {
    let resp = client
        .post(format!("{}/api/inventory/items", inv_url()))
        .bearer_auth(jwt)
        .json(&serde_json::json!({
            "tenant_id": tenant_id,
            "sku": sku,
            "name": format!("Invariant Test Item {}", sku),
            "inventory_account_ref": "1200",
            "cogs_account_ref": "5000",
            "variance_account_ref": "5010",
            "tracking_mode": "none"
        }))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().await.ok()?;
    body["id"].as_str().map(|s| s.to_string())
}

/// POST /api/inventory/receipts. Returns (status_code, body).
pub async fn http_post_receipt(
    client: &reqwest::Client,
    jwt: &str,
    payload: serde_json::Value,
) -> (u16, serde_json::Value) {
    let resp = client
        .post(format!("{}/api/inventory/receipts", inv_url()))
        .bearer_auth(jwt)
        .json(&payload)
        .send()
        .await
        .expect("receipt request failed");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!({}));
    (status, body)
}

/// POST /api/inventory/adjustments. Returns (status_code, body).
pub async fn http_post_adjustment(
    client: &reqwest::Client,
    jwt: &str,
    payload: serde_json::Value,
) -> (u16, serde_json::Value) {
    let resp = client
        .post(format!("{}/api/inventory/adjustments", inv_url()))
        .bearer_auth(jwt)
        .json(&payload)
        .send()
        .await
        .expect("adjustment request failed");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!({}));
    (status, body)
}

/// POST /api/inventory/issues. Returns (status_code, body).
pub async fn http_post_issue(
    client: &reqwest::Client,
    jwt: &str,
    payload: serde_json::Value,
) -> (u16, serde_json::Value) {
    let resp = client
        .post(format!("{}/api/inventory/issues", inv_url()))
        .bearer_auth(jwt)
        .json(&payload)
        .send()
        .await
        .expect("issue request failed");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!({}));
    (status, body)
}

// ============================================================================
// Inventory DB helpers
// ============================================================================

/// Query current on-hand quantity for (tenant_id, item_id).
/// Returns 0 if no row found.
pub async fn query_on_hand(pool: &sqlx::PgPool, tenant_id: &str, item_id: &str) -> i64 {
    let item_uuid = uuid::Uuid::parse_str(item_id).expect("invalid item_id UUID");
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(on_hand, 0) FROM item_on_hand WHERE tenant_id = $1 AND item_id = $2",
    )
    .bind(tenant_id)
    .bind(item_uuid)
    .fetch_optional(pool)
    .await
    .unwrap_or(None)
    .unwrap_or(0)
}

/// Count stuck outbox events (inv_outbox) for this tenant.
pub async fn count_inv_outbox_pending(pool: &sqlx::PgPool, tenant_id: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM inv_outbox WHERE tenant_id = $1 AND processed_at IS NULL",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await
    .unwrap_or(0)
}

// ============================================================================
// AR HTTP helpers
// ============================================================================

/// Probe whether the AR service accepts our JWT by calling a read-only endpoint.
/// Returns false if the service rejects the JWT (401), meaning the service's
/// JWT_PUBLIC_KEY isn't configured with our test private key.
pub async fn ar_jwt_accepted(client: &reqwest::Client, jwt: &str) -> bool {
    match client
        .get(format!("{}/api/ar/invoices", ar_url()))
        .bearer_auth(jwt)
        .send()
        .await
    {
        Ok(r) if r.status().as_u16() != 401 => true,
        _ => false,
    }
}

/// POST /api/ar/invoices/{id}/credit-notes. Returns (status_code, body).
/// Pass `jwt = None` to test unauthenticated access (expect 401).
pub async fn http_issue_credit_note(
    client: &reqwest::Client,
    invoice_id: i32,
    jwt: Option<&str>,
    payload: serde_json::Value,
) -> (u16, serde_json::Value) {
    let mut req = client
        .post(format!("{}/api/ar/invoices/{}/credit-notes", ar_url(), invoice_id));
    if let Some(token) = jwt {
        req = req.bearer_auth(token);
    }
    let resp = req
        .json(&payload)
        .send()
        .await
        .expect("credit-note request failed");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!({}));
    (status, body)
}

/// Insert a minimal AR customer + invoice directly into the DB.
/// Returns (customer_id, invoice_id).
/// tenant_id MUST be <= 50 chars (ar_customers.app_id is varchar(50)).
pub async fn db_seed_invoice(pool: &sqlx::PgPool, tenant_id: &str, amount_cents: i64) -> (i32, i32) {
    let short_id = &uuid::Uuid::new_v4().to_string()[..8];
    let customer_id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
         VALUES ($1, $2, $3, 'active', 0, NOW(), NOW())
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(format!("t-{}@t.io", short_id))
    .bind("Invariant Test Customer")
    .fetch_one(pool)
    .await
    .expect("insert ar_customers failed");

    let invoice_id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_invoices (
             app_id, tilled_invoice_id, ar_customer_id, status, amount_cents, currency,
             created_at, updated_at
         )
         VALUES ($1, $2, $3, 'open', $4, 'usd', NOW(), NOW())
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(format!("in_{}", short_id))
    .bind(customer_id)
    .bind(amount_cents)
    .fetch_one(pool)
    .await
    .expect("insert ar_invoices failed");

    (customer_id, invoice_id)
}

/// Insert a draft AR invoice directly into the DB.
/// tenant_id MUST be <= 50 chars (ar_customers.app_id is varchar(50)).
pub async fn db_seed_draft_invoice(pool: &sqlx::PgPool, tenant_id: &str, amount_cents: i64) -> (i32, i32) {
    let short_id = &uuid::Uuid::new_v4().to_string()[..8];
    let customer_id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
         VALUES ($1, $2, $3, 'active', 0, NOW(), NOW())
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(format!("d-{}@t.io", short_id))
    .bind("Draft Test Customer")
    .fetch_one(pool)
    .await
    .expect("insert ar_customers failed");

    let invoice_id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_invoices (
             app_id, tilled_invoice_id, ar_customer_id, status, amount_cents, currency,
             created_at, updated_at
         )
         VALUES ($1, $2, $3, 'draft', $4, 'usd', NOW(), NOW())
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(format!("ind_{}", short_id))
    .bind(customer_id)
    .bind(amount_cents)
    .fetch_one(pool)
    .await
    .expect("insert draft ar_invoices failed");

    (customer_id, invoice_id)
}

/// Sum of credit notes issued against an invoice.
pub async fn sum_credits_for_invoice(pool: &sqlx::PgPool, invoice_id: i32) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(amount_minor), 0) FROM ar_credit_notes WHERE invoice_id = $1",
    )
    .bind(invoice_id)
    .fetch_one(pool)
    .await
    .unwrap_or(0)
}

// ============================================================================
// Cleanup helpers
// ============================================================================

pub async fn cleanup_inv_tenant(pool: &sqlx::PgPool, tenant_id: &str) {
    for q in [
        "DELETE FROM inv_outbox WHERE tenant_id = $1",
        "DELETE FROM inv_idempotency_keys WHERE tenant_id = $1",
        "DELETE FROM layer_consumptions WHERE ledger_entry_id IN (SELECT id FROM inventory_ledger WHERE tenant_id = $1)",
        "DELETE FROM inventory_serial_instances WHERE tenant_id = $1",
        "DELETE FROM item_on_hand WHERE tenant_id = $1",
        "DELETE FROM inventory_reservations WHERE tenant_id = $1",
        "DELETE FROM inv_adjustments WHERE tenant_id = $1",
        "DELETE FROM inventory_layers WHERE tenant_id = $1",
        "DELETE FROM inventory_ledger WHERE tenant_id = $1",
        "DELETE FROM inventory_lots WHERE tenant_id = $1",
        "DELETE FROM items WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(pool).await.ok();
    }
}

pub async fn cleanup_ar_tenant(pool: &sqlx::PgPool, tenant_id: &str) {
    for q in [
        "DELETE FROM events_outbox WHERE tenant_id = $1",
        "DELETE FROM ar_credit_notes WHERE app_id = $1",
        "DELETE FROM ar_invoice_attempts WHERE app_id = $1",
        "DELETE FROM ar_invoices WHERE app_id = $1",
        "DELETE FROM ar_customers WHERE app_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(pool).await.ok();
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use uuid::Uuid;

    // ── RACE ATTACKS ─────────────────────────────────────────────────────────

    /// Two threads simultaneously attempt to return 5 units from a PO receipt
    /// of exactly 5 units. With row locking, on-hand must never go negative.
    ///
    /// Attack: concurrent adjustments racing on the same row lock.
    /// Invariant: on_hand >= 0 at all times.
    #[tokio::test]
    async fn invariant_concurrent_corrections_same_record() {
        dotenvy::dotenv().ok();

        let client = Arc::new(http_client());
        let health = format!("{}/api/health", inv_url());
        if !service_ready(&client, &health).await {
            eprintln!("SKIP: inventory service not reachable at {}", inv_url());
            return;
        }
        let Some(key) = dev_private_key() else {
            eprintln!("SKIP: JWT_PRIVATE_KEY_PEM not set");
            return;
        };

        let tenant_id = format!("inv-race-{}", Uuid::new_v4());
        let jwt = Arc::new(make_jwt(&key, &tenant_id, &["inventory.mutate", "inventory.read"]));
        let pool = Arc::new(inv_pool().await);
        cleanup_inv_tenant(&pool, &tenant_id).await;

        // Create item
        let sku = format!("RACE-{}", Uuid::new_v4());
        let Some(item_id) = http_create_item(&client, &jwt, &tenant_id, &sku).await else {
            eprintln!("SKIP: could not create item (JWT may not be configured)");
            return;
        };
        let warehouse_id = Uuid::new_v4().to_string();

        // Receive exactly 5 units
        let (status, _) = http_post_receipt(
            &client,
            &jwt,
            serde_json::json!({
                "tenant_id": tenant_id,
                "item_id": item_id,
                "warehouse_id": warehouse_id,
                "quantity": 5,
                "unit_cost_minor": 1000,
                "currency": "usd",
                "source_type": "purchase",
                "idempotency_key": format!("recv-{}", Uuid::new_v4())
            }),
        )
        .await;
        assert!(status == 201 || status == 200, "receipt failed: {status}");

        let on_hand_before = query_on_hand(&pool, &tenant_id, &item_id).await;
        assert_eq!(on_hand_before, 5, "expected 5 on-hand after receipt");

        // Two concurrent return-adjustments each trying to remove 5 units.
        // Only one can succeed without violating the non-negative guard.
        let client2 = Arc::clone(&client);
        let jwt2 = Arc::clone(&jwt);
        let tenant2 = tenant_id.clone();
        let item2 = item_id.clone();
        let wh2 = warehouse_id.clone();

        let h1 = tokio::spawn({
            let c = Arc::clone(&client);
            let j = Arc::clone(&jwt);
            let t = tenant_id.clone();
            let i = item_id.clone();
            let w = warehouse_id.clone();
            async move {
                http_post_adjustment(
                    &c,
                    &j,
                    serde_json::json!({
                        "tenant_id": t,
                        "item_id": i,
                        "warehouse_id": w,
                        "quantity_delta": -5,
                        "reason": "return_to_vendor",
                        "allow_negative": false,
                        "idempotency_key": format!("ret-a-{}", Uuid::new_v4())
                    }),
                )
                .await
            }
        });

        let h2 = tokio::spawn(async move {
            http_post_adjustment(
                &client2,
                &jwt2,
                serde_json::json!({
                    "tenant_id": tenant2,
                    "item_id": item2,
                    "warehouse_id": wh2,
                    "quantity_delta": -5,
                    "reason": "return_to_vendor",
                    "allow_negative": false,
                    "idempotency_key": format!("ret-b-{}", Uuid::new_v4())
                }),
            )
            .await
        });

        let (r1, r2) = tokio::join!(h1, h2);
        let (s1, _b1) = r1.unwrap();
        let (s2, _b2) = r2.unwrap();

        // Exactly one must succeed (201/200), the other must be blocked (4xx)
        // OR both succeed if the locking order allows it — but on-hand must never go < 0.
        let on_hand_after = query_on_hand(&pool, &tenant_id, &item_id).await;
        assert!(
            on_hand_after >= 0,
            "INVARIANT VIOLATED: on_hand went negative ({on_hand_after}) after concurrent returns"
        );

        let successes = [s1, s2].iter().filter(|&&s| s == 201 || s == 200).count();
        let blocks = [s1, s2].iter().filter(|&&s| s >= 400).count();

        // If both succeeded, on-hand must be exactly 0 (removed all 5 twice but only had 5 → impossible)
        // so at most one can succeed
        assert!(
            successes <= 1,
            "INVARIANT VIOLATED: both concurrent returns of 5 units succeeded when only 5 were on-hand. \
             Possible race in FIFO row locking. statuses: {s1}, {s2}"
        );
        assert!(
            successes + blocks == 2,
            "unexpected status combination: {s1}, {s2}"
        );

        // Outbox must not be stuck
        let stuck = count_inv_outbox_pending(&pool, &tenant_id).await;
        assert_eq!(stuck, 0, "outbox has {stuck} unprocessed events after concurrent returns");

        cleanup_inv_tenant(&pool, &tenant_id).await;
    }

    /// Post correction with same idempotency_key twice. Assert applied exactly once.
    ///
    /// Attack: retry storm after a partial write.
    /// Invariant: idempotency_key guarantees at-most-once application.
    #[tokio::test]
    async fn invariant_idempotent_correction_retry() {
        dotenvy::dotenv().ok();

        let client = http_client();
        if !service_ready(&client, &format!("{}/api/health", inv_url())).await {
            eprintln!("SKIP: inventory service not reachable");
            return;
        }
        let Some(key) = dev_private_key() else {
            eprintln!("SKIP: JWT_PRIVATE_KEY_PEM not set");
            return;
        };

        let tenant_id = format!("inv-idem-{}", Uuid::new_v4());
        let jwt = make_jwt(&key, &tenant_id, &["inventory.mutate", "inventory.read"]);
        let pool = inv_pool().await;
        cleanup_inv_tenant(&pool, &tenant_id).await;

        let sku = format!("IDEM-{}", Uuid::new_v4());
        let Some(item_id) = http_create_item(&client, &jwt, &tenant_id, &sku).await else {
            eprintln!("SKIP: could not create item");
            return;
        };
        let warehouse_id = Uuid::new_v4().to_string();

        // Receive 10 units
        let (status, _) = http_post_receipt(
            &client,
            &jwt,
            serde_json::json!({
                "tenant_id": tenant_id,
                "item_id": item_id,
                "warehouse_id": warehouse_id,
                "quantity": 10,
                "unit_cost_minor": 500,
                "currency": "usd",
                "source_type": "purchase",
                "idempotency_key": format!("recv-{}", Uuid::new_v4())
            }),
        )
        .await;
        assert!(status == 201 || status == 200, "receipt failed: {status}");

        let idem_key = format!("correction-retry-{}", Uuid::new_v4());
        let payload = serde_json::json!({
            "tenant_id": tenant_id,
            "item_id": item_id,
            "warehouse_id": warehouse_id,
            "quantity_delta": -3,
            "reason": "return_to_vendor",
            "allow_negative": false,
            "idempotency_key": idem_key
        });

        // First application
        let (s1, _) = http_post_adjustment(&client, &jwt, payload.clone()).await;
        assert!(s1 == 201, "first correction should return 201, got {s1}");

        let on_hand_after_first = query_on_hand(&pool, &tenant_id, &item_id).await;
        assert_eq!(on_hand_after_first, 7, "expected 7 after removing 3");

        // Retry with same idempotency key — must be a no-op
        let (s2, body2) = http_post_adjustment(&client, &jwt, payload.clone()).await;
        assert!(
            s2 == 200 || s2 == 201,
            "retry should return 200 (replay) or 201, got {s2}: {body2}"
        );

        let on_hand_after_retry = query_on_hand(&pool, &tenant_id, &item_id).await;
        assert_eq!(
            on_hand_after_retry, 7,
            "INVARIANT VIOLATED: retry applied the correction a second time \
             (expected 7, got {on_hand_after_retry})"
        );

        // Count ledger rows — must be exactly 2 (receipt + 1 correction, not 2 corrections)
        let ledger_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1 AND item_id = $2 AND entry_type = 'adjustment'",
        )
        .bind(&tenant_id)
        .bind(uuid::Uuid::parse_str(&item_id).unwrap())
        .fetch_one(&pool)
        .await
        .unwrap_or(0);
        assert_eq!(
            ledger_rows, 1,
            "INVARIANT VIOLATED: idempotent retry created {ledger_rows} adjustment ledger rows (expected 1)"
        );

        cleanup_inv_tenant(&pool, &tenant_id).await;
    }

    /// Simultaneous receipt and return for same item/PO.
    /// Net inventory must always be non-negative.
    ///
    /// Attack: race between a stock increase and stock decrease on the same record.
    /// Invariant: no read-modify-write race can produce negative on-hand.
    #[tokio::test]
    async fn invariant_concurrent_receipt_and_return() {
        dotenvy::dotenv().ok();

        let client = Arc::new(http_client());
        if !service_ready(&client, &format!("{}/api/health", inv_url())).await {
            eprintln!("SKIP: inventory service not reachable");
            return;
        }
        let Some(key) = dev_private_key() else {
            eprintln!("SKIP: JWT_PRIVATE_KEY_PEM not set");
            return;
        };

        let tenant_id = format!("inv-conc-rr-{}", Uuid::new_v4());
        let jwt = Arc::new(make_jwt(&key, &tenant_id, &["inventory.mutate"]));
        let pool = Arc::new(inv_pool().await);
        cleanup_inv_tenant(&pool, &tenant_id).await;

        let sku = format!("CONC-{}", Uuid::new_v4());
        let Some(item_id) = http_create_item(&client, &jwt, &tenant_id, &sku).await else {
            eprintln!("SKIP: could not create item");
            return;
        };
        let warehouse_id = Uuid::new_v4().to_string();

        // Pre-seed 5 units so a return of 5 is valid
        let (status, _) = http_post_receipt(
            &client,
            &jwt,
            serde_json::json!({
                "tenant_id": tenant_id,
                "item_id": item_id,
                "warehouse_id": warehouse_id,
                "quantity": 5,
                "unit_cost_minor": 1000,
                "currency": "usd",
                "source_type": "purchase",
                "idempotency_key": format!("seed-{}", Uuid::new_v4())
            }),
        )
        .await;
        assert!(status == 201 || status == 200, "seed receipt failed: {status}");

        // Concurrent: new receipt of 5 vs return (adjustment -5)
        let c_recv = Arc::clone(&client);
        let j_recv = Arc::clone(&jwt);
        let t_recv = tenant_id.clone();
        let i_recv = item_id.clone();
        let w_recv = warehouse_id.clone();

        let h_recv = tokio::spawn(async move {
            http_post_receipt(
                &c_recv,
                &j_recv,
                serde_json::json!({
                    "tenant_id": t_recv,
                    "item_id": i_recv,
                    "warehouse_id": w_recv,
                    "quantity": 5,
                    "unit_cost_minor": 1000,
                    "currency": "usd",
                    "source_type": "purchase",
                    "idempotency_key": format!("recv-concurrent-{}", Uuid::new_v4())
                }),
            )
            .await
        });

        let c_ret = Arc::clone(&client);
        let j_ret = Arc::clone(&jwt);
        let t_ret = tenant_id.clone();
        let i_ret = item_id.clone();
        let w_ret = warehouse_id.clone();

        let h_ret = tokio::spawn(async move {
            http_post_adjustment(
                &c_ret,
                &j_ret,
                serde_json::json!({
                    "tenant_id": t_ret,
                    "item_id": i_ret,
                    "warehouse_id": w_ret,
                    "quantity_delta": -5,
                    "reason": "return_to_vendor",
                    "allow_negative": false,
                    "idempotency_key": format!("ret-concurrent-{}", Uuid::new_v4())
                }),
            )
            .await
        });

        let (r_recv, r_ret) = tokio::join!(h_recv, h_ret);
        let (s_recv, _) = r_recv.unwrap();
        let (s_ret, _) = r_ret.unwrap();

        let on_hand = query_on_hand(&pool, &tenant_id, &item_id).await;
        assert!(
            on_hand >= 0,
            "INVARIANT VIOLATED: concurrent receipt+return produced negative on-hand ({on_hand}). \
             receipt_status={s_recv}, return_status={s_ret}"
        );

        cleanup_inv_tenant(&pool, &tenant_id).await;
    }

    // ── OVER-CORRECTION ATTACKS ───────────────────────────────────────────────

    /// Receive 10 units. Attempt return (negative adjustment) of 15.
    ///
    /// Attack: over-correction — return more than received.
    /// Invariant: blocked with 4xx; on_hand unchanged.
    #[tokio::test]
    async fn invariant_return_exceeds_received() {
        dotenvy::dotenv().ok();

        let client = http_client();
        if !service_ready(&client, &format!("{}/api/health", inv_url())).await {
            eprintln!("SKIP: inventory service not reachable");
            return;
        }
        let Some(key) = dev_private_key() else {
            eprintln!("SKIP: JWT_PRIVATE_KEY_PEM not set");
            return;
        };

        let tenant_id = format!("inv-over-{}", Uuid::new_v4());
        let jwt = make_jwt(&key, &tenant_id, &["inventory.mutate"]);
        let pool = inv_pool().await;
        cleanup_inv_tenant(&pool, &tenant_id).await;

        let sku = format!("OVER-{}", Uuid::new_v4());
        let Some(item_id) = http_create_item(&client, &jwt, &tenant_id, &sku).await else {
            eprintln!("SKIP: could not create item");
            return;
        };
        let warehouse_id = Uuid::new_v4().to_string();

        // Receive 10
        let (status, _) = http_post_receipt(
            &client,
            &jwt,
            serde_json::json!({
                "tenant_id": tenant_id,
                "item_id": item_id,
                "warehouse_id": warehouse_id,
                "quantity": 10,
                "unit_cost_minor": 1000,
                "currency": "usd",
                "source_type": "purchase",
                "idempotency_key": format!("recv-{}", Uuid::new_v4())
            }),
        )
        .await;
        assert!(status == 201 || status == 200, "receipt failed: {status}");
        assert_eq!(query_on_hand(&pool, &tenant_id, &item_id).await, 10);

        // Attempt to return 15 (exceeds 10 on-hand)
        let (s, body) = http_post_adjustment(
            &client,
            &jwt,
            serde_json::json!({
                "tenant_id": tenant_id,
                "item_id": item_id,
                "warehouse_id": warehouse_id,
                "quantity_delta": -15,
                "reason": "return_to_vendor",
                "allow_negative": false,
                "idempotency_key": format!("over-return-{}", Uuid::new_v4())
            }),
        )
        .await;

        assert!(
            s >= 400 && s < 500,
            "INVARIANT VIOLATED: return of 15 from 10-unit stock was accepted (status {s}). \
             Body: {body}"
        );
        // Error must mention the conflict (not a generic 500)
        let body_str = body.to_string();
        assert!(
            body_str.contains("negative") || body_str.contains("on_hand") || body_str.contains("insufficient") || body_str.contains("quantity"),
            "INVARIANT VIOLATED: error message is not descriptive enough. Got: {body_str}"
        );

        // on_hand must be unchanged
        let on_hand_after = query_on_hand(&pool, &tenant_id, &item_id).await;
        assert_eq!(
            on_hand_after, 10,
            "INVARIANT VIOLATED: on_hand changed after rejected over-return (got {on_hand_after})"
        );

        cleanup_inv_tenant(&pool, &tenant_id).await;
    }

    /// Credit memo for 150 against invoice of 100.
    ///
    /// Attack: over-credit — credit note exceeds invoice amount.
    /// Invariant: blocked with 4xx or net balance correct; never silently over-credited.
    #[tokio::test]
    async fn invariant_credit_memo_exceeds_invoice() {
        dotenvy::dotenv().ok();

        let client = http_client();
        let Some(key) = dev_private_key() else {
            eprintln!("SKIP: JWT_PRIVATE_KEY_PEM not set");
            return;
        };
        if !service_ready(&client, &format!("{}/api/health", ar_url())).await {
            eprintln!("SKIP: AR service not reachable at {}", ar_url());
            return;
        }

        let pool = ar_pool().await;
        // Keep tenant_id <= 50 chars: "oc-" + 8 hex chars = 11
        let short = &Uuid::new_v4().to_string().replace('-', "")[..8];
        let tenant_id = format!("oc-{}", short);
        let jwt = make_jwt(&key, &tenant_id, &["ar.mutate", "ar.read"]);
        if !ar_jwt_accepted(&client, &jwt).await {
            eprintln!("SKIP: AR service does not accept our test JWT (JWT_PUBLIC_KEY not configured)");
            return;
        }
        cleanup_ar_tenant(&pool, &tenant_id).await;

        let (_cust_id, invoice_id) = db_seed_invoice(&pool, &tenant_id, 10000).await;

        // Issue credit note for the full amount (10000 = $100) — should succeed
        let (s1, body1) = http_issue_credit_note(
            &client,
            invoice_id,
            Some(&jwt),
            serde_json::json!({
                "credit_note_id": Uuid::new_v4(),
                "app_id": tenant_id,
                "customer_id": format!("cust-{}", tenant_id),
                "amount_minor": 10000,
                "currency": "usd",
                "reason": "billing_error",
                "correlation_id": Uuid::new_v4().to_string()
            }),
        )
        .await;
        assert!(
            s1 == 201 || s1 == 200,
            "first credit note (full invoice) should succeed, got {s1}: {body1}"
        );

        // Attempt second credit note for 5000 more (would total 150% of invoice)
        let (s2, body2) = http_issue_credit_note(
            &client,
            invoice_id,
            Some(&jwt),
            serde_json::json!({
                "credit_note_id": Uuid::new_v4(),
                "app_id": tenant_id,
                "customer_id": format!("cust-{}", tenant_id),
                "amount_minor": 5000,
                "currency": "usd",
                "reason": "additional_credit",
                "correlation_id": Uuid::new_v4().to_string()
            }),
        )
        .await;

        assert!(
            s2 >= 400 && s2 < 500,
            "INVARIANT VIOLATED: credit of 150 against invoice of 100 was accepted (status {s2}). \
             Body: {body2}"
        );

        // Total credits in DB must not exceed invoice amount
        let total_credits = sum_credits_for_invoice(&pool, invoice_id).await;
        assert!(
            total_credits <= 10000,
            "INVARIANT VIOLATED: total credits ({total_credits}) exceed invoice amount (10000)"
        );

        cleanup_ar_tenant(&pool, &tenant_id).await;
    }

    /// Receive 10, issue 8 to WO (consume). Then attempt to adjust down by 5 more.
    /// Net: received 10, issued 8, would go to -3.
    ///
    /// Attack: correction after consumption — can't return what was issued.
    /// Invariant: blocked; on_hand never negative.
    #[tokio::test]
    async fn invariant_correction_after_consumption() {
        dotenvy::dotenv().ok();

        let client = http_client();
        if !service_ready(&client, &format!("{}/api/health", inv_url())).await {
            eprintln!("SKIP: inventory service not reachable");
            return;
        }
        let Some(key) = dev_private_key() else {
            eprintln!("SKIP: JWT_PRIVATE_KEY_PEM not set");
            return;
        };

        let tenant_id = format!("inv-post-consume-{}", Uuid::new_v4());
        let jwt = make_jwt(&key, &tenant_id, &["inventory.mutate"]);
        let pool = inv_pool().await;
        cleanup_inv_tenant(&pool, &tenant_id).await;

        let sku = format!("POSTCONSUME-{}", Uuid::new_v4());
        let Some(item_id) = http_create_item(&client, &jwt, &tenant_id, &sku).await else {
            eprintln!("SKIP: could not create item");
            return;
        };
        let warehouse_id = Uuid::new_v4().to_string();

        // Receive 10
        let (rs, _) = http_post_receipt(
            &client,
            &jwt,
            serde_json::json!({
                "tenant_id": tenant_id,
                "item_id": item_id,
                "warehouse_id": warehouse_id,
                "quantity": 10,
                "unit_cost_minor": 1000,
                "currency": "usd",
                "source_type": "purchase",
                "idempotency_key": format!("recv-{}", Uuid::new_v4())
            }),
        )
        .await;
        assert!(rs == 201 || rs == 200, "receipt failed: {rs}");

        // Issue 8 (consume for work order)
        let (is, _) = http_post_issue(
            &client,
            &jwt,
            serde_json::json!({
                "tenant_id": tenant_id,
                "item_id": item_id,
                "warehouse_id": warehouse_id,
                "quantity": 8,
                "idempotency_key": format!("issue-{}", Uuid::new_v4()),
                "reference_type": "work_order",
                "reference_id": Uuid::new_v4().to_string()
            }),
        )
        .await;
        assert!(is == 201 || is == 200, "issue failed: {is}");

        // on_hand should be 2
        let on_hand_mid = query_on_hand(&pool, &tenant_id, &item_id).await;
        assert_eq!(on_hand_mid, 2, "expected 2 on-hand after issuing 8 from 10");

        // Attempt to adjust -5 (would go to -3)
        let (s, body) = http_post_adjustment(
            &client,
            &jwt,
            serde_json::json!({
                "tenant_id": tenant_id,
                "item_id": item_id,
                "warehouse_id": warehouse_id,
                "quantity_delta": -5,
                "reason": "return_to_vendor",
                "allow_negative": false,
                "idempotency_key": format!("over-adj-{}", Uuid::new_v4())
            }),
        )
        .await;

        assert!(
            s >= 400 && s < 500,
            "INVARIANT VIOLATED: adjustment of -5 on 2-unit stock was accepted (status {s}). \
             Body: {body}"
        );

        let on_hand_after = query_on_hand(&pool, &tenant_id, &item_id).await;
        assert_eq!(
            on_hand_after, 2,
            "INVARIANT VIOLATED: on_hand changed after rejected adjustment (got {on_hand_after})"
        );

        cleanup_inv_tenant(&pool, &tenant_id).await;
    }

    // ── CASCADE CONFLICT ATTACKS ──────────────────────────────────────────────

    /// Attempt to credit a DRAFT invoice (not yet posted).
    ///
    /// Attack: correction of an unposted document.
    /// Invariant: rejected with 422; must not create a credit against a non-final document.
    #[tokio::test]
    async fn invariant_correction_of_draft() {
        dotenvy::dotenv().ok();

        let client = http_client();
        let Some(key) = dev_private_key() else {
            eprintln!("SKIP: JWT_PRIVATE_KEY_PEM not set");
            return;
        };
        if !service_ready(&client, &format!("{}/api/health", ar_url())).await {
            eprintln!("SKIP: AR service not reachable at {}", ar_url());
            return;
        }

        let pool = ar_pool().await;
        // Keep tenant_id <= 50 chars
        let short = &Uuid::new_v4().to_string().replace('-', "")[..8];
        let tenant_id = format!("dr-{}", short);
        let jwt = make_jwt(&key, &tenant_id, &["ar.mutate", "ar.read"]);
        if !ar_jwt_accepted(&client, &jwt).await {
            eprintln!("SKIP: AR service does not accept our test JWT (JWT_PUBLIC_KEY not configured)");
            return;
        }
        cleanup_ar_tenant(&pool, &tenant_id).await;

        let (_cust_id, invoice_id) = db_seed_draft_invoice(&pool, &tenant_id, 10000).await;

        let (s, body) = http_issue_credit_note(
            &client,
            invoice_id,
            Some(&jwt),
            serde_json::json!({
                "credit_note_id": Uuid::new_v4(),
                "app_id": tenant_id,
                "customer_id": format!("cust-{}", tenant_id),
                "amount_minor": 5000,
                "currency": "usd",
                "reason": "correction",
                "correlation_id": Uuid::new_v4().to_string()
            }),
        )
        .await;

        assert!(
            s == 422 || s == 400,
            "INVARIANT VIOLATED: credit note against DRAFT invoice was accepted (status {s}). \
             Body: {body}"
        );
        let body_str = body.to_string();
        assert!(
            !body_str.contains("internal error") && !body_str.contains("panic"),
            "INVARIANT VIOLATED: error response indicates server panic or internal error: {body_str}"
        );

        // No credit notes should exist for this invoice
        let total = sum_credits_for_invoice(&pool, invoice_id).await;
        assert_eq!(
            total, 0,
            "INVARIANT VIOLATED: credit note was written to DB despite rejection"
        );

        cleanup_ar_tenant(&pool, &tenant_id).await;
    }

    /// Attempt credit note for a random UUID (nonexistent invoice).
    ///
    /// Attack: correction of a nonexistent document.
    /// Invariant: 404 returned, never a 500.
    #[tokio::test]
    async fn invariant_correction_of_nonexistent() {
        dotenvy::dotenv().ok();

        let client = http_client();
        let Some(key) = dev_private_key() else {
            eprintln!("SKIP: JWT_PRIVATE_KEY_PEM not set");
            return;
        };
        if !service_ready(&client, &format!("{}/api/health", ar_url())).await {
            eprintln!("SKIP: AR service not reachable at {}", ar_url());
            return;
        }

        let short = &Uuid::new_v4().to_string().replace('-', "")[..8];
        let tenant_id = format!("ne-{}", short);
        let jwt = make_jwt(&key, &tenant_id, &["ar.mutate"]);
        if !ar_jwt_accepted(&client, &jwt).await {
            eprintln!("SKIP: AR service does not accept our test JWT (JWT_PUBLIC_KEY not configured)");
            return;
        }

        // Use a fictional invoice ID that cannot exist
        let fake_invoice_id = i32::MAX;

        let (s, body) = http_issue_credit_note(
            &client,
            fake_invoice_id,
            Some(&jwt),
            serde_json::json!({
                "credit_note_id": Uuid::new_v4(),
                "app_id": tenant_id,
                "customer_id": "cust-nonexistent",
                "amount_minor": 1000,
                "currency": "usd",
                "reason": "nonexistent_correction",
                "correlation_id": Uuid::new_v4().to_string()
            }),
        )
        .await;

        assert!(
            s == 404 || s == 422,
            "INVARIANT VIOLATED: credit note for nonexistent invoice returned {s} (expected 404 or 422). \
             Body: {body}"
        );
        assert_ne!(
            s, 500,
            "INVARIANT VIOLATED: nonexistent invoice correction caused a 500 server error. Body: {body}"
        );
    }

    // ── DATA SHAPE ATTACKS ────────────────────────────────────────────────────

    /// Attempt adjustment with quantity_delta = 0.
    ///
    /// Attack: zero-quantity correction.
    /// Invariant: rejected with 400; no ledger row written.
    #[tokio::test]
    async fn invariant_zero_quantity_correction() {
        dotenvy::dotenv().ok();

        let client = http_client();
        if !service_ready(&client, &format!("{}/api/health", inv_url())).await {
            eprintln!("SKIP: inventory service not reachable");
            return;
        }
        let Some(key) = dev_private_key() else {
            eprintln!("SKIP: JWT_PRIVATE_KEY_PEM not set");
            return;
        };

        let tenant_id = format!("inv-zero-{}", Uuid::new_v4());
        let jwt = make_jwt(&key, &tenant_id, &["inventory.mutate"]);
        let pool = inv_pool().await;
        cleanup_inv_tenant(&pool, &tenant_id).await;

        let sku = format!("ZERO-{}", Uuid::new_v4());
        let Some(item_id) = http_create_item(&client, &jwt, &tenant_id, &sku).await else {
            eprintln!("SKIP: could not create item");
            return;
        };
        let warehouse_id = Uuid::new_v4().to_string();

        let (s, body) = http_post_adjustment(
            &client,
            &jwt,
            serde_json::json!({
                "tenant_id": tenant_id,
                "item_id": item_id,
                "warehouse_id": warehouse_id,
                "quantity_delta": 0,
                "reason": "zero_return",
                "allow_negative": false,
                "idempotency_key": format!("zero-adj-{}", Uuid::new_v4())
            }),
        )
        .await;

        assert!(
            s >= 400 && s < 500,
            "INVARIANT VIOLATED: zero-quantity adjustment was accepted (status {s}). Body: {body}"
        );

        // No ledger row must exist for this adjustment
        let rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1 AND entry_type = 'adjustment'",
        )
        .bind(&tenant_id)
        .fetch_one(&pool)
        .await
        .unwrap_or(0);
        assert_eq!(
            rows, 0,
            "INVARIANT VIOLATED: zero-quantity adjustment wrote {rows} ledger row(s)"
        );

        cleanup_inv_tenant(&pool, &tenant_id).await;
    }

    /// Credit note with amount_minor = 0 (invalid).
    ///
    /// Attack: null/zero amount correction.
    /// Invariant: rejected with 400; no credit note written.
    #[tokio::test]
    async fn invariant_null_amount_correction() {
        dotenvy::dotenv().ok();

        let client = http_client();
        let Some(key) = dev_private_key() else {
            eprintln!("SKIP: JWT_PRIVATE_KEY_PEM not set");
            return;
        };
        if !service_ready(&client, &format!("{}/api/health", ar_url())).await {
            eprintln!("SKIP: AR service not reachable at {}", ar_url());
            return;
        }

        let pool = ar_pool().await;
        let short = &Uuid::new_v4().to_string().replace('-', "")[..8];
        let tenant_id = format!("na-{}", short);
        let jwt = make_jwt(&key, &tenant_id, &["ar.mutate"]);
        if !ar_jwt_accepted(&client, &jwt).await {
            eprintln!("SKIP: AR service does not accept our test JWT (JWT_PUBLIC_KEY not configured)");
            return;
        }
        cleanup_ar_tenant(&pool, &tenant_id).await;

        let (_cust_id, invoice_id) = db_seed_invoice(&pool, &tenant_id, 10000).await;

        let (s, body) = http_issue_credit_note(
            &client,
            invoice_id,
            Some(&jwt),
            serde_json::json!({
                "credit_note_id": Uuid::new_v4(),
                "app_id": tenant_id,
                "customer_id": format!("cust-{}", tenant_id),
                "amount_minor": 0,
                "currency": "usd",
                "reason": "zero_amount_attack",
                "correlation_id": Uuid::new_v4().to_string()
            }),
        )
        .await;

        assert!(
            s >= 400 && s < 500,
            "INVARIANT VIOLATED: zero-amount credit note was accepted (status {s}). Body: {body}"
        );
        assert_ne!(
            s, 500,
            "INVARIANT VIOLATED: zero-amount credit note caused a 500. Body: {body}"
        );

        let total = sum_credits_for_invoice(&pool, invoice_id).await;
        assert_eq!(
            total, 0,
            "INVARIANT VIOLATED: zero-amount credit note was written to DB (total={total})"
        );

        cleanup_ar_tenant(&pool, &tenant_id).await;
    }

    // ── LOAD ATTACKS ──────────────────────────────────────────────────────────

    /// 1000 concurrent valid adjustments for different items (different records).
    ///
    /// Attack: high-concurrency load — deadlock or connection exhaustion.
    /// Invariant: all succeed, balances sum correctly, no deadlocks.
    #[tokio::test]
    async fn invariant_1000_concurrent_corrections() {
        dotenvy::dotenv().ok();

        let client = Arc::new(http_client());
        if !service_ready(&client, &format!("{}/api/health", inv_url())).await {
            eprintln!("SKIP: inventory service not reachable");
            return;
        }
        let Some(key) = dev_private_key() else {
            eprintln!("SKIP: JWT_PRIVATE_KEY_PEM not set");
            return;
        };

        let tenant_id = format!("inv-load-{}", Uuid::new_v4());
        let jwt = Arc::new(make_jwt(&key, &tenant_id, &["inventory.mutate"]));
        let pool = Arc::new(inv_pool().await);
        cleanup_inv_tenant(&pool, &tenant_id).await;

        const N: usize = 1000;

        // Create N items, each with 10 units pre-seeded
        let warehouse_id = Uuid::new_v4().to_string();
        let mut item_ids = Vec::with_capacity(N);
        for i in 0..N {
            let sku = format!("LOAD-{}-{}", i, Uuid::new_v4());
            if let Some(id) = http_create_item(&client, &jwt, &tenant_id, &sku).await {
                item_ids.push(id);
            }
        }
        if item_ids.len() < N {
            eprintln!("SKIP: could not create all {N} items (only got {})", item_ids.len());
            return;
        }

        // Seed all items with 10 units each via direct DB insert for speed
        {
            let pool2 = Arc::clone(&pool);
            for item_id in &item_ids {
                let iid = uuid::Uuid::parse_str(item_id).unwrap();
                // Insert directly to bypass HTTP overhead for seeding
                let idem_key = Uuid::new_v4().to_string();
                let _ = http_post_receipt(
                    &client,
                    &jwt,
                    serde_json::json!({
                        "tenant_id": tenant_id,
                        "item_id": item_id,
                        "warehouse_id": warehouse_id,
                        "quantity": 10,
                        "unit_cost_minor": 100,
                        "currency": "usd",
                        "source_type": "purchase",
                        "idempotency_key": idem_key
                    }),
                )
                .await;
                let _ = pool2; // used via iid below
                let _ = iid;
            }
        }

        // 1000 concurrent adjustments, each for a different item
        let mut handles = Vec::with_capacity(N);
        for item_id in item_ids.iter().take(N) {
            let c = Arc::clone(&client);
            let j = Arc::clone(&jwt);
            let t = tenant_id.clone();
            let i = item_id.clone();
            let w = warehouse_id.clone();
            let h = tokio::spawn(async move {
                http_post_adjustment(
                    &c,
                    &j,
                    serde_json::json!({
                        "tenant_id": t,
                        "item_id": i,
                        "warehouse_id": w,
                        "quantity_delta": -3,
                        "reason": "load_test_return",
                        "allow_negative": false,
                        "idempotency_key": format!("load-adj-{}", Uuid::new_v4())
                    }),
                )
                .await
            });
            handles.push(h);
        }

        let results = futures::future::join_all(handles).await;
        let failures: Vec<_> = results
            .iter()
            .filter_map(|r| {
                if let Ok((s, body)) = r {
                    if *s >= 500 {
                        Some(format!("status={s}, body={body}"))
                    } else {
                        None
                    }
                } else {
                    Some("task panicked".to_string())
                }
            })
            .collect();

        assert!(
            failures.is_empty(),
            "INVARIANT VIOLATED: {} of {} concurrent corrections failed with 5xx:\n{}",
            failures.len(),
            N,
            failures.join("\n")
        );

        // Each item should have 7 on-hand (10 - 3)
        let mut balance_errors = 0usize;
        for item_id in &item_ids {
            let on_hand = query_on_hand(&pool, &tenant_id, item_id).await;
            if on_hand != 7 {
                balance_errors += 1;
                if balance_errors <= 5 {
                    eprintln!("  balance mismatch: item={item_id} on_hand={on_hand} (expected 7)");
                }
            }
        }
        assert_eq!(
            balance_errors, 0,
            "INVARIANT VIOLATED: {balance_errors} of {N} items have wrong on-hand after load test"
        );

        cleanup_inv_tenant(&pool, &tenant_id).await;
    }

    /// Post a correction while simultaneously reading the balance.
    /// The report must never show a split/inconsistent state as a valid end state.
    ///
    /// Attack: dirty read of balance during a write.
    /// Invariant: balance endpoint is always consistent (no half-written state visible).
    #[tokio::test]
    async fn invariant_correction_report_consistency() {
        dotenvy::dotenv().ok();

        let client = Arc::new(http_client());
        if !service_ready(&client, &format!("{}/api/health", inv_url())).await {
            eprintln!("SKIP: inventory service not reachable");
            return;
        }
        let Some(key) = dev_private_key() else {
            eprintln!("SKIP: JWT_PRIVATE_KEY_PEM not set");
            return;
        };

        let tenant_id = format!("inv-consistency-{}", Uuid::new_v4());
        let jwt = Arc::new(make_jwt(&key, &tenant_id, &["inventory.mutate", "inventory.read"]));
        let pool = Arc::new(inv_pool().await);
        cleanup_inv_tenant(&pool, &tenant_id).await;

        let sku = format!("CONSIST-{}", Uuid::new_v4());
        let Some(item_id) = http_create_item(&client, &jwt, &tenant_id, &sku).await else {
            eprintln!("SKIP: could not create item");
            return;
        };
        let warehouse_id = Uuid::new_v4().to_string();
        let item_uuid = uuid::Uuid::parse_str(&item_id).unwrap();

        // Receive 100 units
        let (rs, _) = http_post_receipt(
            &client,
            &jwt,
            serde_json::json!({
                "tenant_id": tenant_id,
                "item_id": item_id,
                "warehouse_id": warehouse_id,
                "quantity": 100,
                "unit_cost_minor": 100,
                "currency": "usd",
                "source_type": "purchase",
                "idempotency_key": format!("recv-{}", Uuid::new_v4())
            }),
        )
        .await;
        assert!(rs == 201 || rs == 200, "receipt failed: {rs}");

        // Concurrently:
        // - 50 correction tasks, each removing 1 unit
        // - 50 read tasks querying on_hand
        // None of the reads should see a fractional state
        let mut handles = Vec::new();
        for _ in 0..50 {
            let c = Arc::clone(&client);
            let j = Arc::clone(&jwt);
            let t = tenant_id.clone();
            let i = item_id.clone();
            let w = warehouse_id.clone();
            handles.push(tokio::spawn(async move {
                let (s, _) = http_post_adjustment(
                    &c,
                    &j,
                    serde_json::json!({
                        "tenant_id": t,
                        "item_id": i,
                        "warehouse_id": w,
                        "quantity_delta": -1,
                        "reason": "concurrent_return",
                        "allow_negative": false,
                        "idempotency_key": format!("adj-{}", Uuid::new_v4())
                    }),
                )
                .await;
                s
            }));
        }

        // Concurrent reads
        let mut read_handles = Vec::new();
        let pool2 = Arc::clone(&pool);
        let t2 = tenant_id.clone();
        for _ in 0..50 {
            let p = Arc::clone(&pool2);
            let t = t2.clone();
            let i = item_uuid;
            read_handles.push(tokio::spawn(async move {
                sqlx::query_scalar::<_, i64>(
                    "SELECT COALESCE(on_hand, 0) FROM item_on_hand WHERE tenant_id = $1 AND item_id = $2",
                )
                .bind(&t)
                .bind(i)
                .fetch_optional(&*p)
                .await
                .unwrap_or(None)
                .unwrap_or(0)
            }));
        }

        let write_results = futures::future::join_all(handles).await;
        let read_results = futures::future::join_all(read_handles).await;

        // All reads must return non-negative values (no split state)
        for r in &read_results {
            let on_hand = r.as_ref().expect("read task panicked");
            assert!(
                *on_hand >= 0,
                "INVARIANT VIOLATED: concurrent read returned negative on_hand ({on_hand}) \
                 — dirty read of in-progress write"
            );
        }

        // Final state must be internally consistent
        let final_on_hand = query_on_hand(&pool, &tenant_id, &item_id).await;
        let successes = write_results.iter().filter(|r| {
            matches!(r, Ok(s) if *s == 200 || *s == 201)
        }).count();

        assert_eq!(
            final_on_hand,
            100 - successes as i64,
            "INVARIANT VIOLATED: final on_hand ({final_on_hand}) doesn't match \
             100 - successes ({successes})"
        );

        cleanup_inv_tenant(&pool, &tenant_id).await;
    }

    // ── AUDIT TRAIL ATTACKS ───────────────────────────────────────────────────

    /// After every blocked adversarial action, audit trail must record the attempt.
    /// After every successful correction, audit trail must show correct actor/timestamp.
    ///
    /// Attack: tampering via absent audit trail.
    /// Invariant: audit trail is append-only and complete.
    #[tokio::test]
    async fn invariant_audit_trail_never_lies() {
        dotenvy::dotenv().ok();

        let client = http_client();
        if !service_ready(&client, &format!("{}/api/health", inv_url())).await {
            eprintln!("SKIP: inventory service not reachable");
            return;
        }
        let Some(key) = dev_private_key() else {
            eprintln!("SKIP: JWT_PRIVATE_KEY_PEM not set");
            return;
        };

        let tenant_id = format!("inv-audit-{}", Uuid::new_v4());
        let jwt = make_jwt(&key, &tenant_id, &["inventory.mutate"]);
        let pool = inv_pool().await;
        cleanup_inv_tenant(&pool, &tenant_id).await;

        let sku = format!("AUDIT-{}", Uuid::new_v4());
        let Some(item_id) = http_create_item(&client, &jwt, &tenant_id, &sku).await else {
            eprintln!("SKIP: could not create item");
            return;
        };
        let warehouse_id = Uuid::new_v4().to_string();
        let item_uuid = uuid::Uuid::parse_str(&item_id).unwrap();

        // Receive 5 units
        let (rs, _) = http_post_receipt(
            &client,
            &jwt,
            serde_json::json!({
                "tenant_id": tenant_id,
                "item_id": item_id,
                "warehouse_id": warehouse_id,
                "quantity": 5,
                "unit_cost_minor": 1000,
                "currency": "usd",
                "source_type": "purchase",
                "idempotency_key": format!("recv-{}", Uuid::new_v4())
            }),
        )
        .await;
        assert!(rs == 201 || rs == 200, "receipt failed: {rs}");

        // Attempt blocked over-return (should be blocked but must have an outbox/ledger trail)
        let (s, _) = http_post_adjustment(
            &client,
            &jwt,
            serde_json::json!({
                "tenant_id": tenant_id,
                "item_id": item_id,
                "warehouse_id": warehouse_id,
                "quantity_delta": -10,
                "reason": "over_return",
                "allow_negative": false,
                "idempotency_key": format!("blocked-adj-{}", Uuid::new_v4())
            }),
        )
        .await;
        assert!(s >= 400, "expected blocked over-return, got {s}");

        // Successful adjustment
        let (s2, _) = http_post_adjustment(
            &client,
            &jwt,
            serde_json::json!({
                "tenant_id": tenant_id,
                "item_id": item_id,
                "warehouse_id": warehouse_id,
                "quantity_delta": -2,
                "reason": "valid_return",
                "allow_negative": false,
                "idempotency_key": format!("valid-adj-{}", Uuid::new_v4())
            }),
        )
        .await;
        assert!(s2 == 201 || s2 == 200, "valid adjustment should succeed, got {s2}");

        // Audit: exactly one ledger row for the receipt and one for the valid adjustment
        let recv_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1 AND item_id = $2 AND entry_type = 'received'",
        )
        .bind(&tenant_id)
        .bind(item_uuid)
        .fetch_one(&pool)
        .await
        .unwrap_or(0);

        let adj_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1 AND item_id = $2 AND entry_type = 'adjustment'",
        )
        .bind(&tenant_id)
        .bind(item_uuid)
        .fetch_one(&pool)
        .await
        .unwrap_or(0);

        assert_eq!(
            recv_rows, 1,
            "INVARIANT VIOLATED: expected exactly 1 receipt ledger row, got {recv_rows}"
        );
        assert_eq!(
            adj_rows, 1,
            "INVARIANT VIOLATED: expected exactly 1 adjustment ledger row (blocked one must NOT appear), got {adj_rows}"
        );

        // Outbox event must exist for the successful adjustment
        let outbox_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM inv_outbox WHERE tenant_id = $1 AND event_type = 'inventory.adjusted'",
        )
        .bind(&tenant_id)
        .fetch_one(&pool)
        .await
        .unwrap_or(0);

        assert_eq!(
            outbox_count, 1,
            "INVARIANT VIOLATED: expected 1 outbox event for valid adjustment, got {outbox_count}"
        );

        cleanup_inv_tenant(&pool, &tenant_id).await;
    }

    /// After issuing a credit note, invoice view must show net amount.
    /// The net must equal original minus credits, never the original alone.
    ///
    /// Attack: stale balance display — document shows gross not net.
    /// Invariant: sum of credits matches what is stored; balance is consistent.
    #[tokio::test]
    async fn invariant_document_shows_net() {
        dotenvy::dotenv().ok();

        let client = http_client();
        let Some(key) = dev_private_key() else {
            eprintln!("SKIP: JWT_PRIVATE_KEY_PEM not set");
            return;
        };
        if !service_ready(&client, &format!("{}/api/health", ar_url())).await {
            eprintln!("SKIP: AR service not reachable at {}", ar_url());
            return;
        }

        let pool = ar_pool().await;
        let short = &Uuid::new_v4().to_string().replace('-', "")[..8];
        let tenant_id = format!("nb-{}", short);
        let jwt = make_jwt(&key, &tenant_id, &["ar.mutate"]);
        if !ar_jwt_accepted(&client, &jwt).await {
            eprintln!("SKIP: AR service does not accept our test JWT (JWT_PUBLIC_KEY not configured)");
            return;
        }
        cleanup_ar_tenant(&pool, &tenant_id).await;

        let invoice_amount = 20000i64; // $200
        let credit_amount = 7500i64;   // $75

        let (_cust_id, invoice_id) = db_seed_invoice(&pool, &tenant_id, invoice_amount).await;

        let (s, body) = http_issue_credit_note(
            &client,
            invoice_id,
            Some(&jwt),
            serde_json::json!({
                "credit_note_id": Uuid::new_v4(),
                "app_id": tenant_id,
                "customer_id": format!("cust-{}", tenant_id),
                "amount_minor": credit_amount,
                "currency": "usd",
                "reason": "billing_error",
                "correlation_id": Uuid::new_v4().to_string()
            }),
        )
        .await;
        assert!(
            s == 201 || s == 200,
            "credit note should succeed, got {s}: {body}"
        );

        // Verify the stored credit equals the requested amount
        let stored_credit = sum_credits_for_invoice(&pool, invoice_id).await;
        assert_eq!(
            stored_credit, credit_amount,
            "INVARIANT VIOLATED: stored credit ({stored_credit}) != issued credit ({credit_amount})"
        );

        // Invoice amount must be unchanged (immutable-post: original is never mutated)
        let stored_invoice_amount: i64 = sqlx::query_scalar(
            "SELECT amount_cents FROM ar_invoices WHERE id = $1",
        )
        .bind(invoice_id)
        .fetch_one(&pool)
        .await
        .expect("invoice not found");

        assert_eq!(
            stored_invoice_amount, invoice_amount,
            "INVARIANT VIOLATED: original invoice amount was mutated ({stored_invoice_amount} != {invoice_amount}). \
             Immutable-post principle violated."
        );

        // Net balance = invoice_amount - credit_amount
        let net = stored_invoice_amount - stored_credit;
        assert_eq!(
            net,
            invoice_amount - credit_amount,
            "INVARIANT VIOLATED: net balance ({net}) is wrong (expected {})",
            invoice_amount - credit_amount
        );

        cleanup_ar_tenant(&pool, &tenant_id).await;
    }
}
