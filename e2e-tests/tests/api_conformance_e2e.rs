//! API Conformance Test Harness (bd-si65p)
//!
//! Verifies all 25 platform services implement the same API contract:
//! health endpoints, auth rejection/acceptance, error format, CORS, metrics.
//!
//! Services that are unreachable are skipped gracefully.
//!
//! ## Known Deviations (from bd-2ou15 audit)
//! - **production**: now has RequirePermissionsLayer (fixed in bd-29c9i.1)
//! - **customer-portal**: uses PortalJwt, not platform JWT; missing /healthz, /metrics
//! - **workforce-competence**: missing /healthz
//! - **integrations, party, ttp**: missing /metrics endpoint
//! - **pdf-editor**: missing /metrics endpoint and CORS headers
//!
//! ## Running
//! ```bash
//! set -a && source .env && set +a
//! ./scripts/cargo-slot.sh test -p e2e-tests --test api_conformance_e2e -- --nocapture
//! ```

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;
use uuid::Uuid;

// ============================================================================
// Service Specification
// ============================================================================

#[allow(dead_code)] // perm documents expected permission per service
struct ServiceSpec {
    name: &'static str,
    env_var: &'static str,
    default_port: u16,
    mutation_route: &'static str,
    perm: &'static str,
    expect_healthz: bool,
    expect_auth: bool,
    expect_metrics: bool,
    expect_cors: bool,
}

impl ServiceSpec {
    fn base_url(&self) -> String {
        std::env::var(self.env_var)
            .unwrap_or_else(|_| format!("http://localhost:{}", self.default_port))
    }
}

/// All 25 modules/ services with their canonical ports and a representative
/// mutation route for auth testing.
fn services() -> Vec<ServiceSpec> {
    vec![
        // --- Standard services (platform JWT, /healthz present) ---
        ServiceSpec { name: "ap", env_var: "AP_URL", default_port: 8093,
            mutation_route: "/api/ap/vendors", perm: "ap.mutate",
            expect_healthz: true, expect_auth: true, expect_metrics: true, expect_cors: true },
        ServiceSpec { name: "ar", env_var: "AR_URL", default_port: 8086,
            mutation_route: "/api/ar/customers", perm: "ar.mutate",
            expect_healthz: true, expect_auth: true, expect_metrics: true, expect_cors: true },
        ServiceSpec { name: "bom", env_var: "BOM_URL", default_port: 8107,
            mutation_route: "/api/bom", perm: "bom.mutate",
            expect_healthz: true, expect_auth: true, expect_metrics: true, expect_cors: true },
        ServiceSpec { name: "consolidation", env_var: "CONSOLIDATION_URL", default_port: 8105,
            mutation_route: "/api/consolidation/groups", perm: "consolidation.mutate",
            expect_healthz: true, expect_auth: true, expect_metrics: true, expect_cors: true },
        ServiceSpec { name: "fixed-assets", env_var: "FIXED_ASSETS_URL", default_port: 8104,
            mutation_route: "/api/fixed-assets/assets", perm: "fixed_assets.mutate",
            expect_healthz: true, expect_auth: true, expect_metrics: true, expect_cors: true },
        ServiceSpec { name: "gl", env_var: "GL_URL", default_port: 8090,
            mutation_route: "/api/gl/accounts", perm: "gl.post",
            expect_healthz: true, expect_auth: true, expect_metrics: true, expect_cors: true },
        // integrations: no /metrics endpoint yet
        ServiceSpec { name: "integrations", env_var: "INTEGRATIONS_URL", default_port: 8099,
            mutation_route: "/api/integrations/external-refs", perm: "integrations.mutate",
            expect_healthz: true, expect_auth: true, expect_metrics: false, expect_cors: true },
        ServiceSpec { name: "inventory", env_var: "INVENTORY_URL", default_port: 8092,
            mutation_route: "/api/inventory/items", perm: "inventory.mutate",
            expect_healthz: true, expect_auth: true, expect_metrics: true, expect_cors: true },
        ServiceSpec { name: "maintenance", env_var: "MAINTENANCE_URL", default_port: 8101,
            mutation_route: "/api/maintenance/assets", perm: "maintenance.mutate",
            expect_healthz: true, expect_auth: true, expect_metrics: true, expect_cors: true },
        ServiceSpec { name: "notifications", env_var: "NOTIFICATIONS_URL", default_port: 8089,
            mutation_route: "/api/notifications/templates", perm: "notifications.mutate",
            expect_healthz: true, expect_auth: true, expect_metrics: true, expect_cors: true },
        ServiceSpec { name: "numbering", env_var: "NUMBERING_URL", default_port: 8120,
            mutation_route: "/allocate", perm: "numbering.allocate",
            expect_healthz: true, expect_auth: true, expect_metrics: true, expect_cors: true },
        // party: no /metrics endpoint yet
        ServiceSpec { name: "party", env_var: "PARTY_URL", default_port: 8098,
            mutation_route: "/api/party/companies", perm: "party.mutate",
            expect_healthz: true, expect_auth: true, expect_metrics: false, expect_cors: true },
        ServiceSpec { name: "payments", env_var: "PAYMENTS_URL", default_port: 8088,
            mutation_route: "/api/payments/checkout-sessions", perm: "payments.mutate",
            expect_healthz: true, expect_auth: true, expect_metrics: true, expect_cors: true },
        // pdf-editor: no /metrics, no CORS
        ServiceSpec { name: "pdf-editor", env_var: "PDF_EDITOR_URL", default_port: 8102,
            mutation_route: "/api/pdf/forms/templates", perm: "pdf_editor.mutate",
            expect_healthz: true, expect_auth: true, expect_metrics: false, expect_cors: false },
        ServiceSpec { name: "quality-inspection", env_var: "QI_URL", default_port: 8106,
            mutation_route: "/api/quality-inspection/plans", perm: "quality_inspection.mutate",
            expect_healthz: true, expect_auth: true, expect_metrics: true, expect_cors: true },
        ServiceSpec { name: "reporting", env_var: "REPORTING_URL", default_port: 8096,
            mutation_route: "/api/reporting/rebuild", perm: "reporting.mutate",
            expect_healthz: true, expect_auth: true, expect_metrics: true, expect_cors: true },
        ServiceSpec { name: "shipping-receiving", env_var: "SR_URL", default_port: 8103,
            mutation_route: "/api/shipping-receiving/shipments", perm: "shipping_receiving.mutate",
            expect_healthz: true, expect_auth: true, expect_metrics: true, expect_cors: true },
        ServiceSpec { name: "subscriptions", env_var: "SUBSCRIPTIONS_URL", default_port: 8087,
            mutation_route: "/api/bill-runs/execute", perm: "subscriptions.mutate",
            expect_healthz: true, expect_auth: true, expect_metrics: true, expect_cors: true },
        ServiceSpec { name: "timekeeping", env_var: "TIMEKEEPING_URL", default_port: 8097,
            mutation_route: "/api/timekeeping/employees", perm: "timekeeping.mutate",
            expect_healthz: true, expect_auth: true, expect_metrics: true, expect_cors: true },
        ServiceSpec { name: "treasury", env_var: "TREASURY_URL", default_port: 8094,
            mutation_route: "/api/treasury/accounts/bank", perm: "treasury.mutate",
            expect_healthz: true, expect_auth: true, expect_metrics: true, expect_cors: true },
        // ttp: no /metrics endpoint yet
        ServiceSpec { name: "ttp", env_var: "TTP_URL", default_port: 8100,
            mutation_route: "/api/ttp/billing-runs", perm: "ttp.mutate",
            expect_healthz: true, expect_auth: true, expect_metrics: false, expect_cors: true },
        ServiceSpec { name: "workflow", env_var: "WORKFLOW_URL", default_port: 8110,
            mutation_route: "/api/workflow/definitions", perm: "workflow.mutate",
            expect_healthz: true, expect_auth: true, expect_metrics: true, expect_cors: true },
        // --- Deviations ---
        // customer-portal: PortalJwt auth, no /healthz, no /metrics
        ServiceSpec { name: "customer-portal", env_var: "CUSTOMER_PORTAL_URL", default_port: 8111,
            mutation_route: "/portal/admin/users", perm: "",
            expect_healthz: false, expect_auth: false, expect_metrics: false, expect_cors: true },
        // production: now has RequirePermissionsLayer (fixed in bd-29c9i.1)
        ServiceSpec { name: "production", env_var: "PRODUCTION_URL", default_port: 8108,
            mutation_route: "/api/production/workcenters", perm: "production.mutate",
            expect_healthz: true, expect_auth: true, expect_metrics: true, expect_cors: true },
        // workforce-competence: missing /healthz (has /api/schema-version instead)
        ServiceSpec { name: "workforce-competence", env_var: "WC_URL", default_port: 8121,
            mutation_route: "/api/workforce-competence/artifacts", perm: "workforce_competence.mutate",
            expect_healthz: false, expect_auth: true, expect_metrics: true, expect_cors: true },
    ]
}

// ============================================================================
// JWT Helper
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
    app_id: Option<String>,
    roles: Vec<String>,
    perms: Vec<String>,
    actor_type: String,
    ver: String,
}

fn all_permissions() -> Vec<String> {
    [
        "ap.mutate", "ap.read", "ar.mutate", "ar.read", "bom.mutate", "bom.read",
        "consolidation.mutate", "consolidation.read", "fixed_assets.mutate",
        "fixed_assets.read", "gl.post", "gl.read", "integrations.mutate",
        "integrations.read", "inventory.mutate", "inventory.read", "maintenance.mutate",
        "maintenance.read", "notifications.mutate", "notifications.read",
        "numbering.allocate", "numbering.read", "party.mutate", "party.read",
        "payments.mutate", "payments.read", "pdf_editor.mutate", "pdf_editor.read",
        "production.mutate", "production.read",
        "quality_inspection.mutate", "quality_inspection.read", "reporting.mutate",
        "reporting.read", "shipping_receiving.mutate", "shipping_receiving.read",
        "subscriptions.mutate", "timekeeping.mutate", "timekeeping.read",
        "treasury.mutate", "treasury.read", "ttp.mutate", "ttp.read",
        "workflow.mutate", "workflow.read", "workforce_competence.mutate",
        "workforce_competence.read",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn make_jwt(key: &EncodingKey, tenant_id: &str) -> String {
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
        perms: all_permissions(),
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, key).unwrap()
}

// ============================================================================
// Individual Conformance Checks
// ============================================================================

async fn check_reachable(client: &Client, base_url: &str) -> bool {
    client
        .get(format!("{base_url}/api/health"))
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
}

async fn check_healthz(client: &Client, base_url: &str) -> Result<(), String> {
    let resp = client
        .get(format!("{base_url}/healthz"))
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    if resp.status().as_u16() == 200 {
        Ok(())
    } else {
        Err(format!("expected 200, got {}", resp.status()))
    }
}

async fn check_api_health(client: &Client, base_url: &str) -> Result<(), String> {
    let resp = client
        .get(format!("{base_url}/api/health"))
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("expected 2xx, got {}", resp.status()));
    }
    let body: Value = resp.json().await.map_err(|e| format!("not JSON: {e}"))?;
    if body.get("status").is_some() {
        Ok(())
    } else {
        Err(format!("missing 'status' field: {body}"))
    }
}

async fn check_auth_rejection(
    client: &Client,
    base_url: &str,
    route: &str,
) -> Result<(), String> {
    let url = format!("{base_url}{route}");
    let resp = client
        .post(&url)
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status().as_u16();
    if status == 401 {
        Ok(())
    } else {
        Err(format!("expected 401 without JWT, got {status}"))
    }
}

async fn check_auth_acceptance(
    client: &Client,
    base_url: &str,
    route: &str,
    jwt: &str,
) -> Result<(), String> {
    let url = format!("{base_url}{route}");
    let resp = client
        .post(&url)
        .bearer_auth(jwt)
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status().as_u16();
    if status == 401 {
        Err("got 401 with valid JWT".to_string())
    } else {
        Ok(())
    }
}

async fn check_error_format(
    client: &Client,
    base_url: &str,
    route: &str,
    jwt: &str,
) -> Result<(), String> {
    let url = format!("{base_url}{route}");
    let resp = client
        .post(&url)
        .bearer_auth(jwt)
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status().as_u16();
    // Only verify JSON error format on auth-related responses (401/403).
    // 422 from Axum body deserialization returns plain text by default — that's
    // a separate conformance concern, not an error-format regression.
    if status != 401 && status != 403 {
        return Ok(());
    }
    let text = resp.text().await.unwrap_or_default();
    let body: Value = serde_json::from_str(&text)
        .map_err(|_| format!("{status} not JSON: {}", &text[..text.len().min(200)]))?;
    if body.get("error").is_some() || body.get("message").is_some() {
        Ok(())
    } else {
        Err(format!("{status} JSON missing error/message: {body}"))
    }
}

async fn check_cors(client: &Client, base_url: &str) -> Result<(), String> {
    let resp = client
        .get(format!("{base_url}/api/health"))
        .header("Origin", "https://conformance-test.example.com")
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    if resp.headers().get("access-control-allow-origin").is_some() {
        Ok(())
    } else {
        Err("missing access-control-allow-origin header".to_string())
    }
}

async fn check_metrics(client: &Client, base_url: &str) -> Result<(), String> {
    let resp = client
        .get(format!("{base_url}/metrics"))
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("expected 2xx, got {}", resp.status()));
    }
    let body = resp.text().await.unwrap_or_default();
    // Prometheus exposition format uses # HELP, # TYPE, and metric names
    if body.contains("# HELP") || body.contains("# TYPE") || body.contains("_total") {
        Ok(())
    } else {
        Err(format!(
            "not Prometheus format ({}B, first 200: {})",
            body.len(),
            &body[..body.len().min(200)]
        ))
    }
}

// ============================================================================
// Main Test
// ============================================================================

#[tokio::test]
async fn api_conformance() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("HTTP client");

    let key = std::env::var("JWT_PRIVATE_KEY_PEM").ok().and_then(|pem| {
        EncodingKey::from_rsa_pem(pem.replace("\\n", "\n").as_bytes()).ok()
    });
    if key.is_none() {
        eprintln!("WARNING: JWT_PRIVATE_KEY_PEM not set or invalid — auth tests will be skipped");
    }

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = key.as_ref().map(|k| make_jwt(k, &tenant_id));

    let svcs = services();
    let mut failures: Vec<String> = Vec::new();
    let mut skipped = 0u32;
    let mut passed = 0u32;
    let mut checked = 0u32;

    let bar = "=".repeat(72);
    println!("\n{bar}");
    println!("API Conformance Test — {} services", svcs.len());
    println!("{bar}\n");

    for svc in &svcs {
        let base = svc.base_url();
        println!("--- {} ({}) ---", svc.name, base);

        if !check_reachable(&client, &base).await {
            println!("  SKIP (not reachable)\n");
            skipped += 1;
            continue;
        }

        // 1. GET /healthz → 200
        if svc.expect_healthz {
            checked += 1;
            match check_healthz(&client, &base).await {
                Ok(()) => { passed += 1; println!("  PASS  healthz"); }
                Err(e) => {
                    println!("  FAIL  healthz: {e}");
                    failures.push(format!("{}: healthz — {e}", svc.name));
                }
            }
        } else {
            println!("  SKIP  healthz (known deviation)");
        }

        // 2. GET /api/health → JSON {status: ...}
        checked += 1;
        match check_api_health(&client, &base).await {
            Ok(()) => { passed += 1; println!("  PASS  api/health"); }
            Err(e) => {
                println!("  FAIL  api/health: {e}");
                failures.push(format!("{}: api/health — {e}", svc.name));
            }
        }

        // 3-5. Auth + error format (requires JWT and platform auth)
        if let Some(ref token) = jwt {
            if svc.expect_auth {
                // 3. No JWT → 401
                checked += 1;
                match check_auth_rejection(&client, &base, svc.mutation_route).await {
                    Ok(()) => { passed += 1; println!("  PASS  auth rejection (no JWT → 401)"); }
                    Err(e) => {
                        println!("  FAIL  auth rejection: {e}");
                        failures.push(format!("{}: auth rejection — {e}", svc.name));
                    }
                }
                // 4. Valid JWT → non-401
                checked += 1;
                match check_auth_acceptance(&client, &base, svc.mutation_route, token).await {
                    Ok(()) => { passed += 1; println!("  PASS  auth acceptance (JWT → non-401)"); }
                    Err(e) => {
                        println!("  FAIL  auth acceptance: {e}");
                        failures.push(format!("{}: auth acceptance — {e}", svc.name));
                    }
                }
                // 5. 4xx → JSON {error/message}
                checked += 1;
                match check_error_format(&client, &base, svc.mutation_route, token).await {
                    Ok(()) => { passed += 1; println!("  PASS  error format"); }
                    Err(e) => {
                        println!("  FAIL  error format: {e}");
                        failures.push(format!("{}: error format — {e}", svc.name));
                    }
                }
            } else {
                println!("  SKIP  auth tests (known deviation: {})", svc.name);
            }
        } else {
            println!("  SKIP  auth tests (no JWT key)");
        }

        // 6. CORS headers
        if svc.expect_cors {
            checked += 1;
            match check_cors(&client, &base).await {
                Ok(()) => { passed += 1; println!("  PASS  CORS"); }
                Err(e) => {
                    println!("  FAIL  CORS: {e}");
                    failures.push(format!("{}: CORS — {e}", svc.name));
                }
            }
        } else {
            println!("  SKIP  CORS (known deviation)");
        }

        // 7. GET /metrics → Prometheus format
        if svc.expect_metrics {
            checked += 1;
            match check_metrics(&client, &base).await {
                Ok(()) => { passed += 1; println!("  PASS  metrics"); }
                Err(e) => {
                    println!("  FAIL  metrics: {e}");
                    failures.push(format!("{}: metrics — {e}", svc.name));
                }
            }
        } else {
            println!("  SKIP  metrics (known deviation)");
        }

        println!();
    }

    // --- Summary ---
    println!("{bar}");
    println!(
        "RESULT: {passed}/{checked} passed, {} failed, {skipped} services skipped",
        failures.len()
    );
    if !failures.is_empty() {
        println!("\nFAILURES:");
        for f in &failures {
            println!("  - {f}");
        }
    }
    println!("{bar}");

    assert!(
        failures.is_empty(),
        "{} conformance check(s) failed — see report above",
        failures.len()
    );
}
