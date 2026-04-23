//! Comprehensive QBO sandbox smoke test (bd-2tqlt).
//!
//! Hits every endpoint, pushes every boundary, stresses every flow.
//! The goal is to find failures, not confirm success.
//!
//! Run: QBO_SANDBOX=1 ./scripts/cargo-slot.sh test -p integrations-rs -- qbo_smoke_test --nocapture

use integrations_rs::domain::qbo::{
    client::{QboClient, QboCustomerPayload},
    QboError, TokenProvider,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

// ============================================================================
// Token provider (same as qbo_sandbox.rs but self-contained)
// ============================================================================

struct SandboxTokenProvider {
    access_token: RwLock<String>,
    refresh_tok: RwLock<String>,
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
    tokens_path: PathBuf,
}

impl SandboxTokenProvider {
    fn load() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        dotenvy::from_path(root.join(".env.qbo-sandbox")).expect(".env.qbo-sandbox");

        let client_id = std::env::var("QBO_CLIENT_ID").expect("QBO_CLIENT_ID");
        let client_secret = std::env::var("QBO_CLIENT_SECRET").expect("QBO_CLIENT_SECRET");

        let tokens_path = root.join(".qbo-tokens.json");
        let content = std::fs::read_to_string(&tokens_path).expect(".qbo-tokens.json");
        let tokens: Value = serde_json::from_str(&content).expect("parse tokens");

        Self {
            access_token: RwLock::new(
                tokens["access_token"]
                    .as_str()
                    .expect("access_token")
                    .into(),
            ),
            refresh_tok: RwLock::new(
                tokens["refresh_token"]
                    .as_str()
                    .expect("refresh_token")
                    .into(),
            ),
            client_id,
            client_secret,
            http: reqwest::Client::new(),
            tokens_path,
        }
    }

    fn realm_id(&self) -> String {
        let content = std::fs::read_to_string(&self.tokens_path).expect("tokens file");
        let t: Value = serde_json::from_str(&content).expect("parse");
        t["realm_id"].as_str().expect("realm_id").to_string()
    }
}

#[async_trait::async_trait]
impl TokenProvider for SandboxTokenProvider {
    async fn get_token(&self) -> Result<String, QboError> {
        Ok(self.access_token.read().await.clone())
    }

    async fn refresh_token(&self) -> Result<String, QboError> {
        let rt = self.refresh_tok.read().await.clone();
        let resp = self
            .http
            .post("https://oauth.platform.intuit.com/oauth2/v1/tokens/bearer")
            .basic_auth(&self.client_id, Some(&self.client_secret))
            .form(&[("grant_type", "refresh_token"), ("refresh_token", &rt)])
            .send()
            .await
            .map_err(|e| QboError::TokenError(e.to_string()))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(QboError::TokenError(format!("Refresh failed: {}", body)));
        }

        let tr: Value = resp
            .json()
            .await
            .map_err(|e| QboError::TokenError(e.to_string()))?;
        let new_at = tr["access_token"]
            .as_str()
            .ok_or_else(|| QboError::TokenError("no access_token".into()))?
            .to_string();
        let new_rt = tr["refresh_token"]
            .as_str()
            .ok_or_else(|| QboError::TokenError("no refresh_token".into()))?
            .to_string();

        *self.access_token.write().await = new_at.clone();
        *self.refresh_tok.write().await = new_rt.clone();

        if let Ok(content) = std::fs::read_to_string(&self.tokens_path) {
            if let Ok(mut existing) = serde_json::from_str::<Value>(&content) {
                existing["access_token"] = Value::String(new_at.clone());
                existing["refresh_token"] = Value::String(new_rt);
                if let Some(v) = tr.get("expires_in") {
                    existing["expires_in"] = v.clone();
                }
                if let Some(v) = tr.get("x_refresh_token_expires_in") {
                    existing["x_refresh_token_expires_in"] = v.clone();
                }
                let _ = std::fs::write(
                    &self.tokens_path,
                    serde_json::to_string_pretty(&existing).expect("json"),
                );
            }
        }
        Ok(new_at)
    }
}

/// Fixed fake token provider for error case testing.
struct BadTokenProvider;

#[async_trait::async_trait]
impl TokenProvider for BadTokenProvider {
    async fn get_token(&self) -> Result<String, QboError> {
        Ok("deliberately-bad-token-12345".into())
    }
    async fn refresh_token(&self) -> Result<String, QboError> {
        Err(QboError::AuthFailed)
    }
}

fn skip_unless_sandbox() -> bool {
    std::env::var("QBO_SANDBOX").map_or(true, |v| v != "1")
}

fn make_client() -> (QboClient, Arc<SandboxTokenProvider>) {
    let provider = Arc::new(SandboxTokenProvider::load());
    let base_url = std::env::var("QBO_SANDBOX_BASE")
        .unwrap_or_else(|_| "https://sandbox-quickbooks.api.intuit.com/v3".into());
    let realm_id = provider.realm_id();
    let client = QboClient::new(&base_url, &realm_id, provider.clone());
    (client, provider)
}

fn bad_client() -> QboClient {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    dotenvy::from_path(root.join(".env.qbo-sandbox")).ok();
    let tokens_path = root.join(".qbo-tokens.json");
    let content = std::fs::read_to_string(&tokens_path).expect("tokens");
    let t: Value = serde_json::from_str(&content).expect("parse");
    let realm_id = t["realm_id"].as_str().expect("realm_id");
    let base_url = std::env::var("QBO_SANDBOX_BASE")
        .unwrap_or_else(|_| "https://sandbox-quickbooks.api.intuit.com/v3".into());
    QboClient::new(&base_url, realm_id, Arc::new(BadTokenProvider))
}

// ============================================================================
// Helpers
// ============================================================================

// ============================================================================
// Test
// ============================================================================

#[tokio::test]
async fn qbo_smoke_test() {
    if skip_unless_sandbox() {
        eprintln!("Skipping QBO smoke test (set QBO_SANDBOX=1)");
        return;
    }
    let (client, provider) = make_client();
    let mut failures: Vec<String> = Vec::new();

    eprintln!("\n╔══════════════════════════════════════════╗");
    eprintln!("║  QBO SANDBOX COMPREHENSIVE SMOKE TEST    ║");
    eprintln!("╚══════════════════════════════════════════╝\n");

    // === 1. Token refresh cycle ===
    eprintln!("▸ Token refresh cycle");
    let token1 = provider.refresh_token().await;
    if let Err(ref e) = token1 {
        failures.push(format!("Token refresh #1: {}", e));
        eprintln!("  FAIL  First refresh — {:?}", e);
        eprintln!("\n  FATAL: Cannot proceed without valid tokens.");
        panic!("Token refresh failed: {:?}", e);
    }
    eprintln!("  PASS  First refresh");

    // Second refresh to test rotation
    let token2 = provider.refresh_token().await;
    match &token2 {
        Ok(t2) => {
            let t1 = token1.as_ref().expect("checked above");
            if t1 == t2 {
                eprintln!("  WARN  Second refresh returned same access_token (no rotation?)");
            } else {
                eprintln!("  PASS  Second refresh returned new token (rotation works)");
            }
        }
        Err(e) => {
            failures.push(format!("Token refresh #2 (rotation): {}", e));
            eprintln!("  FAIL  Second refresh — {:?}", e);
        }
    }

    // === 2. Entity reads: every type ===
    eprintln!("\n▸ Entity reads (all types)");
    let entity_types = [
        "Customer",
        "Invoice",
        "Payment",
        "Item",
        "Estimate",
        "Vendor",
        "Account",
        "PurchaseOrder",
    ];

    for et in &entity_types {
        let query = format!("SELECT * FROM {} MAXRESULTS 3", et);
        match client.query(&query).await {
            Ok(resp) => {
                let count = resp["QueryResponse"][et]
                    .as_array()
                    .map(|a| a.len())
                    .unwrap_or(0);
                eprintln!("  PASS  {} — {} entities returned", et, count);

                // If we got results, try reading the first one individually
                if count > 0 {
                    let id = resp["QueryResponse"][et][0]["Id"].as_str().unwrap_or("1");
                    match client.get_entity(et, id).await {
                        Ok(_) => eprintln!("  PASS  GET {}/{}", et, id),
                        Err(e) => {
                            failures.push(format!("GET {}/{}: {}", et, id, e));
                            eprintln!("  FAIL  GET {}/{} — {}", et, id, e);
                        }
                    }
                }
            }
            Err(e) => {
                failures.push(format!("Query {}: {}", et, e));
                eprintln!("  FAIL  {} — {}", et, e);
            }
        }
    }

    // === 3. Invoice shipping writeback ===
    eprintln!("\n▸ Invoice shipping writeback");
    let inv_resp = client.query("SELECT * FROM Invoice MAXRESULTS 1").await;
    match inv_resp {
        Ok(resp) => {
            if let Some(inv) = resp["QueryResponse"]["Invoice"]
                .as_array()
                .and_then(|a| a.first())
            {
                let inv_id = inv["Id"].as_str().expect("invoice Id");
                let st = inv["SyncToken"].as_str().expect("SyncToken");

                let update = json!({
                    "Id": inv_id,
                    "SyncToken": st,
                    "sparse": true,
                    "ShipDate": "2026-03-27",
                    "TrackingNum": "SMOKE-TEST-TRK-001",
                    "ShipMethodRef": {"value": "UPS Ground"}
                });

                match client
                    .update_entity("Invoice", update, Uuid::new_v4())
                    .await
                {
                    Ok(_) => {
                        eprintln!(
                            "  PASS  Sparse update invoice {} with shipping fields",
                            inv_id
                        );

                        // Verify
                        match client.get_entity("Invoice", inv_id).await {
                            Ok(re) => {
                                let ri = &re["Invoice"];
                                let ship_ok = ri["ShipDate"].as_str() == Some("2026-03-27");
                                let track_ok =
                                    ri["TrackingNum"].as_str() == Some("SMOKE-TEST-TRK-001");
                                let method_ok = ri["ShipMethodRef"]["value"].as_str().is_some();
                                if ship_ok && track_ok && method_ok {
                                    eprintln!("  PASS  Verified shipping fields persisted");
                                } else {
                                    let msg = format!(
                                        "Shipping verify: ShipDate={} TrackingNum={} ShipMethodRef={}",
                                        ri["ShipDate"], ri["TrackingNum"], ri["ShipMethodRef"]
                                    );
                                    failures.push(msg.clone());
                                    eprintln!("  FAIL  {}", msg);
                                }
                            }
                            Err(e) => {
                                failures.push(format!("Re-read invoice: {}", e));
                                eprintln!("  FAIL  Re-read invoice — {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        failures.push(format!("Shipping update: {}", e));
                        eprintln!("  FAIL  Shipping update — {}", e);
                    }
                }
            } else {
                failures.push("No invoices to test shipping writeback".into());
                eprintln!("  SKIP  No invoices found");
            }
        }
        Err(e) => {
            failures.push(format!("Invoice query for shipping: {}", e));
            eprintln!("  FAIL  Invoice query — {}", e);
        }
    }

    // === 4. SyncToken conflict handling ===
    eprintln!("\n▸ SyncToken conflict handling");
    match client.query("SELECT * FROM Invoice MAXRESULTS 1").await {
        Ok(resp) => {
            if let Some(inv) = resp["QueryResponse"]["Invoice"]
                .as_array()
                .and_then(|a| a.first())
            {
                let inv_id = inv["Id"].as_str().expect("Id");
                // Use SyncToken "0" — guaranteed stale
                let stale_update = json!({
                    "Id": inv_id,
                    "SyncToken": "0",
                    "sparse": true,
                    "ShipDate": "2026-01-01"
                });

                match client
                    .update_entity("Invoice", stale_update, Uuid::new_v4())
                    .await
                {
                    Ok(_) => {
                        // SyncToken retry logic should have recovered
                        eprintln!("  PASS  Stale SyncToken recovered via retry");
                    }
                    Err(QboError::SyncTokenExhausted(_)) => {
                        failures.push("SyncToken exhausted — retry loop didn't recover".into());
                        eprintln!("  FAIL  SyncToken exhausted (retries didn't help)");
                    }
                    Err(e) => {
                        failures.push(format!("SyncToken conflict: {}", e));
                        eprintln!("  FAIL  SyncToken conflict — {}", e);
                    }
                }
            }
        }
        Err(e) => {
            failures.push(format!("Invoice query for SyncToken test: {}", e));
            eprintln!("  FAIL  — {}", e);
        }
    }

    // === 5. CDC endpoint with various time ranges ===
    eprintln!("\n▸ CDC endpoint");
    let time_ranges = [
        ("1 hour ago", chrono::Duration::hours(1)),
        ("24 hours ago", chrono::Duration::hours(24)),
        ("7 days ago", chrono::Duration::days(7)),
    ];

    for (label, dur) in &time_ranges {
        let since = chrono::Utc::now() - *dur;
        match client
            .cdc(&["Customer", "Invoice", "Payment", "Item"], &since)
            .await
        {
            Ok(resp) => {
                let entries = resp["CDCResponse"].as_array().map(|a| a.len()).unwrap_or(0);
                let entity_count = resp["CDCResponse"]
                    .as_array()
                    .and_then(|arr| arr.first())
                    .and_then(|v| v["QueryResponse"].as_array())
                    .map(|qrs| {
                        qrs.iter()
                            .filter_map(|qr| qr.as_object())
                            .flat_map(|obj| obj.values())
                            .filter_map(|v| v.as_array())
                            .map(|a| a.len())
                            .sum::<usize>()
                    })
                    .unwrap_or(0);
                eprintln!(
                    "  PASS  CDC {} — {} entries, {} entities",
                    label, entries, entity_count
                );
            }
            Err(e) => {
                failures.push(format!("CDC {}: {}", label, e));
                eprintln!("  FAIL  CDC {} — {}", label, e);
            }
        }
    }

    // CDC with very old date (should hit 30-day limit)
    let old_since = chrono::Utc::now() - chrono::Duration::days(45);
    match client.cdc(&["Customer"], &old_since).await {
        Ok(_) => eprintln!("  WARN  CDC 45 days ago succeeded (expected rejection)"),
        Err(e) => eprintln!("  PASS  CDC 45 days ago rejected as expected: {}", e),
    }

    // === 6. Pagination ===
    eprintln!("\n▸ Pagination (STARTPOSITION / MAXRESULTS)");
    match client
        .query("SELECT * FROM Customer STARTPOSITION 1 MAXRESULTS 2")
        .await
    {
        Ok(resp) => {
            let page1 = resp["QueryResponse"]["Customer"]
                .as_array()
                .map(|a| a.len())
                .unwrap_or(0);
            eprintln!("  PASS  Page 1 (MAXRESULTS 2): {} customers", page1);

            // Page 2
            match client
                .query("SELECT * FROM Customer STARTPOSITION 3 MAXRESULTS 2")
                .await
            {
                Ok(resp2) => {
                    let page2 = resp2["QueryResponse"]["Customer"]
                        .as_array()
                        .map(|a| a.len())
                        .unwrap_or(0);
                    eprintln!("  PASS  Page 2 (STARTPOSITION 3): {} customers", page2);
                }
                Err(e) => {
                    failures.push(format!("Pagination page 2: {}", e));
                    eprintln!("  FAIL  Page 2 — {}", e);
                }
            }
        }
        Err(e) => {
            failures.push(format!("Pagination: {}", e));
            eprintln!("  FAIL  Pagination — {}", e);
        }
    }

    // query_all test
    match client.query_all("SELECT * FROM Customer", "Customer").await {
        Ok(all) => eprintln!(
            "  PASS  query_all: {} total customers across all pages",
            all.len()
        ),
        Err(e) => {
            failures.push(format!("query_all: {}", e));
            eprintln!("  FAIL  query_all — {}", e);
        }
    }

    // === 7. Error cases ===
    eprintln!("\n▸ Error cases");

    // Bad token
    let bad = bad_client();
    match bad.query("SELECT * FROM Customer MAXRESULTS 1").await {
        Err(QboError::AuthFailed) => eprintln!("  PASS  Bad token → AuthFailed"),
        Err(e) => eprintln!("  WARN  Bad token → unexpected error type: {}", e),
        Ok(_) => {
            failures.push("Bad token accepted — should have failed".into());
            eprintln!("  FAIL  Bad token accepted");
        }
    }

    // Non-existent entity
    match client.get_entity("Invoice", "999999999").await {
        Err(e) => eprintln!("  PASS  Non-existent entity → {}", e),
        Ok(v) => {
            if v["Invoice"].is_null() {
                eprintln!("  PASS  Non-existent entity → null response");
            } else {
                eprintln!("  WARN  Non-existent entity returned data: {}", v);
            }
        }
    }

    // Malformed query
    match client.query("THIS IS NOT VALID SQL SYNTAX").await {
        Err(e) => eprintln!("  PASS  Malformed query → {}", e),
        Ok(_) => {
            failures.push("Malformed query accepted".into());
            eprintln!("  FAIL  Malformed query accepted");
        }
    }

    // Missing required fields in update
    match client
        .update_entity("Invoice", json!({"sparse": true}), Uuid::new_v4())
        .await
    {
        Err(e) => eprintln!("  PASS  Update missing Id → {}", e),
        Ok(_) => {
            failures.push("Update without Id accepted".into());
            eprintln!("  FAIL  Update without Id accepted");
        }
    }

    // Non-existent entity type
    match client.query("SELECT * FROM BogusEntity MAXRESULTS 1").await {
        Err(e) => eprintln!("  PASS  Bogus entity type → {}", e),
        Ok(v) => {
            // QBO may return empty results instead of error for some types
            eprintln!(
                "  WARN  Bogus entity type returned: {}",
                &v.to_string()[..v.to_string().len().min(100)]
            );
        }
    }

    // === 8. Rate limits: burst concurrent requests ===
    eprintln!("\n▸ Rate limit stress (20 concurrent requests)");
    let mut handles = Vec::new();
    let token = provider.get_token().await.expect("get token");
    let http = reqwest::Client::new();
    let base_url = std::env::var("QBO_SANDBOX_BASE")
        .unwrap_or_else(|_| "https://sandbox-quickbooks.api.intuit.com/v3".into());
    let realm_id = provider.realm_id();

    for i in 0..20 {
        let url = format!("{}/company/{}/query?minorversion=75", base_url, realm_id);
        let tok = token.clone();
        let h = http.clone();
        handles.push(tokio::spawn(async move {
            let resp = h
                .post(&url)
                .bearer_auth(&tok)
                .header("Accept", "application/json")
                .header("Content-Type", "application/text")
                .body("SELECT * FROM Customer MAXRESULTS 1")
                .send()
                .await;
            match resp {
                Ok(r) => (i, r.status().as_u16()),
                Err(e) => {
                    eprintln!("    req {} HTTP error: {}", i, e);
                    (i, 0)
                }
            }
        }));
    }

    let mut rate_limited = 0;
    let mut ok_count = 0;
    let mut other_errors = 0;
    for handle in handles {
        let (idx, status) = handle.await.expect("task join");
        match status {
            200 => ok_count += 1,
            401 | 429 => {
                rate_limited += 1;
                eprintln!("    req {} → {} (rate limited)", idx, status);
            }
            0 => other_errors += 1,
            s => {
                other_errors += 1;
                eprintln!("    req {} → {} (unexpected)", idx, s);
            }
        }
    }
    eprintln!(
        "  {}  Burst: {} ok, {} rate-limited, {} errors",
        if rate_limited > 0 || other_errors > 0 {
            "WARN"
        } else {
            "PASS"
        },
        ok_count,
        rate_limited,
        other_errors
    );
    // Rate limiting is expected in sandbox — informational only
    if rate_limited > 15 {
        failures.push(format!(
            "Extreme rate limiting: {}/20 blocked",
            rate_limited
        ));
    }

    // Cooldown after burst to avoid residual rate limiting
    eprintln!("  (cooling down 10s for rate limit recovery...)");
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    // === 9. Create test data ===
    eprintln!("\n▸ Create test data");

    // Create a customer
    let cust_payload = QboCustomerPayload {
        display_name: format!("Smoke Test Customer {}", chrono::Utc::now().timestamp()),
        email: Some("smoke@test.example.com".into()),
        company_name: Some("Smoke Test Corp".into()),
        currency_ref: None,
    };
    match client.create_customer(&cust_payload, Uuid::new_v4()).await {
        Ok(resp) => {
            let cust_id = resp["Id"].as_str().unwrap_or("?");
            eprintln!("  PASS  Created customer ID {}", cust_id);

            // Read it back
            match client.get_entity("Customer", cust_id).await {
                Ok(re) => {
                    let name = re["Customer"]["DisplayName"].as_str().unwrap_or("?");
                    eprintln!("  PASS  Read back customer: {}", name);
                }
                Err(e) => {
                    failures.push(format!("Read back created customer: {}", e));
                    eprintln!("  FAIL  Read back — {}", e);
                }
            }
        }
        Err(e) => {
            failures.push(format!("Create customer: {}", e));
            eprintln!("  FAIL  Create customer — {}", e);
        }
    }

    // === 10. Edge cases ===
    eprintln!("\n▸ Edge cases");

    // Empty result set
    match client
        .query("SELECT * FROM Customer WHERE DisplayName = 'ThisCustomerDoesNotExist_XYZ_99999'")
        .await
    {
        Ok(resp) => {
            let count = resp["QueryResponse"]["Customer"]
                .as_array()
                .map(|a| a.len())
                .unwrap_or(0);
            if count == 0 {
                eprintln!("  PASS  Empty result set handled (0 results)");
            } else {
                eprintln!("  WARN  Expected 0 results, got {}", count);
            }
        }
        Err(e) => {
            failures.push(format!("Empty result query: {}", e));
            eprintln!("  FAIL  Empty result query — {}", e);
        }
    }

    // Special characters in filter
    match client
        .query("SELECT * FROM Customer WHERE DisplayName LIKE '%O\\'Brien%' MAXRESULTS 1")
        .await
    {
        Ok(_) => eprintln!("  PASS  Special chars in filter (apostrophe)"),
        Err(e) => eprintln!("  WARN  Special chars in filter — {} (may be expected)", e),
    }

    // Large MAXRESULTS
    match client.query("SELECT * FROM Customer MAXRESULTS 1000").await {
        Ok(resp) => {
            let count = resp["QueryResponse"]["Customer"]
                .as_array()
                .map(|a| a.len())
                .unwrap_or(0);
            eprintln!("  PASS  Large MAXRESULTS 1000 → {} customers", count);
        }
        Err(e) => {
            failures.push(format!("Large MAXRESULTS: {}", e));
            eprintln!("  FAIL  Large MAXRESULTS — {}", e);
        }
    }

    // STARTPOSITION beyond data
    match client
        .query("SELECT * FROM Customer STARTPOSITION 99999 MAXRESULTS 10")
        .await
    {
        Ok(resp) => {
            let count = resp["QueryResponse"]["Customer"]
                .as_array()
                .map(|a| a.len())
                .unwrap_or(0);
            eprintln!("  PASS  STARTPOSITION beyond data → {} results", count);
        }
        Err(e) => {
            failures.push(format!("STARTPOSITION beyond data: {}", e));
            eprintln!("  FAIL  STARTPOSITION beyond data — {}", e);
        }
    }

    // === Summary ===
    eprintln!("\n╔══════════════════════════════════════════╗");
    eprintln!("║  SMOKE TEST SUMMARY                      ║");
    eprintln!("╚══════════════════════════════════════════╝");
    if failures.is_empty() {
        eprintln!("  ALL TESTS PASSED\n");
    } else {
        eprintln!("  {} FAILURES:", failures.len());
        for f in &failures {
            eprintln!("    - {}", f);
        }
        eprintln!();
        panic!("{} smoke test failures", failures.len());
    }
}
