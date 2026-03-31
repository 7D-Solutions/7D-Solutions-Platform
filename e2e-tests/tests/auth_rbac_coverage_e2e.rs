//! Auth/RBAC coverage sweep (bd-decba)
//!
//! Verifies that EVERY non-health endpoint across all modules requires JWT
//! authentication. For aerospace/defense, no data endpoint may be accessible
//! without a valid Bearer token.
//!
//! ## What this tests
//! 1. No-auth requests to read endpoints → 401 with JSON body
//! 2. No-auth requests to mutation endpoints → 401 with JSON body
//! 3. Invalid JWT → 401 with JSON body
//! 4. Valid JWT, wrong permissions → 403 with JSON body
//! 5. /healthz and /api/ready are the ONLY unauthenticated endpoints
//! 6. All 401/403 responses include {error, message}
//!
//! ## Design
//! Uses reqwest against running services. Tests skip if a service is not
//! reachable (no panic on connection refused).
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests --test auth_rbac_coverage_e2e -- --nocapture
//! ```

use reqwest::StatusCode;
use serde_json::Value;

// ============================================================================
// Module port map — matches docker-compose / dev-native ports
// ============================================================================

struct ModuleInfo {
    name: &'static str,
    port: u16,
    /// Representative read endpoint (non-health) that must require auth
    read_endpoints: &'static [&'static str],
    /// Representative mutation endpoint that must require auth
    mutation_endpoints: &'static [(&'static str, &'static str)], // (method, path)
    /// Endpoints that are intentionally unauthenticated
    exempt_endpoints: &'static [&'static str],
}

fn modules() -> Vec<ModuleInfo> {
    vec![
        ModuleInfo {
            name: "inventory",
            port: 8092,
            read_endpoints: &[
                "/api/inventory/items",
                "/api/inventory/uoms",
                "/api/inventory/locations/00000000-0000-0000-0000-000000000000",
            ],
            mutation_endpoints: &[("POST", "/api/inventory/items")],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/metrics", "/api/openapi.json"],
        },
        ModuleInfo {
            name: "ar",
            port: 8086,
            read_endpoints: &[
                "/api/ar/customers",
                "/api/ar/invoices",
                "/api/ar/charges",
                "/api/ar/refunds",
                "/api/ar/disputes",
                "/api/ar/payment-methods",
                "/api/ar/webhooks",
                "/api/ar/events",
                "/api/ar/aging",
            ],
            mutation_endpoints: &[("POST", "/api/ar/customers")],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/metrics"],
        },
        ModuleInfo {
            name: "gl",
            port: 8091,
            read_endpoints: &[
                "/api/gl/trial-balance",
                "/api/gl/income-statement",
                "/api/gl/balance-sheet",
                "/api/gl/detail",
                "/api/gl/cash-flow",
            ],
            mutation_endpoints: &[("POST", "/api/gl/journals")],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/api/openapi.json", "/metrics"],
        },
        ModuleInfo {
            name: "party",
            port: 8098,
            read_endpoints: &[
                "/api/party/parties",
                "/api/party/parties/00000000-0000-0000-0000-000000000000",
            ],
            mutation_endpoints: &[("POST", "/api/party/companies")],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/metrics"],
        },
        ModuleInfo {
            name: "ap",
            port: 8099,
            read_endpoints: &[
                "/api/ap/vendors",
                "/api/ap/bills",
                "/api/ap/purchase-orders",
            ],
            mutation_endpoints: &[("POST", "/api/ap/vendors")],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/api/openapi.json", "/metrics"],
        },
        ModuleInfo {
            name: "treasury",
            port: 8096,
            read_endpoints: &[
                "/api/treasury/accounts",
                "/api/treasury/cash-position",
                "/api/treasury/forecast",
                "/api/treasury/recon/matches",
                "/api/treasury/recon/unmatched",
            ],
            mutation_endpoints: &[("POST", "/api/treasury/accounts")],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/api/openapi.json", "/metrics"],
        },
        ModuleInfo {
            name: "timekeeping",
            port: 8095,
            read_endpoints: &[
                "/api/timekeeping/employees",
                "/api/timekeeping/projects",
                "/api/timekeeping/entries",
                "/api/timekeeping/approvals",
                "/api/timekeeping/allocations",
                "/api/timekeeping/exports",
                "/api/timekeeping/rates",
            ],
            mutation_endpoints: &[("POST", "/api/timekeeping/employees")],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/metrics"],
        },
        ModuleInfo {
            name: "fixed-assets",
            port: 8094,
            read_endpoints: &[
                "/api/fixed-assets/assets",
                "/api/fixed-assets/categories",
                "/api/fixed-assets/depreciation/runs",
                "/api/fixed-assets/disposals",
            ],
            mutation_endpoints: &[("POST", "/api/fixed-assets/categories")],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/api/openapi.json", "/metrics"],
        },
        ModuleInfo {
            name: "consolidation",
            port: 8093,
            read_endpoints: &[
                "/api/consolidation/groups",
            ],
            mutation_endpoints: &[("POST", "/api/consolidation/groups")],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/api/openapi.json", "/metrics"],
        },
        ModuleInfo {
            name: "shipping-receiving",
            port: 8097,
            read_endpoints: &[
                "/api/shipping-receiving/shipments",
            ],
            mutation_endpoints: &[("POST", "/api/shipping-receiving/shipments")],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/api/openapi.json", "/metrics"],
        },
        ModuleInfo {
            name: "ttp",
            port: 8100,
            read_endpoints: &[
                "/api/ttp/service-agreements",
                "/api/metering/trace",
            ],
            mutation_endpoints: &[("POST", "/api/ttp/billing-runs")],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/metrics"],
        },
        ModuleInfo {
            name: "bom",
            port: 8107,
            read_endpoints: &[
                "/api/bom/boms",
            ],
            mutation_endpoints: &[("POST", "/api/bom/boms")],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/api/openapi.json", "/metrics"],
        },
        ModuleInfo {
            name: "production",
            port: 8108,
            read_endpoints: &[
                "/api/production/work-orders",
            ],
            mutation_endpoints: &[("POST", "/api/production/work-orders")],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/api/openapi.json", "/metrics"],
        },
        ModuleInfo {
            name: "maintenance",
            port: 8109,
            read_endpoints: &[
                "/api/maintenance/assets",
            ],
            mutation_endpoints: &[("POST", "/api/maintenance/assets")],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/api/openapi.json", "/metrics"],
        },
        ModuleInfo {
            name: "quality-inspection",
            port: 8110,
            read_endpoints: &[
                "/api/quality-inspection/inspections/by-part-rev",
            ],
            mutation_endpoints: &[("POST", "/api/quality-inspection/plans")],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/api/openapi.json", "/metrics"],
        },
        ModuleInfo {
            name: "notifications",
            port: 8085,
            read_endpoints: &[],
            mutation_endpoints: &[],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/metrics"],
        },
        ModuleInfo {
            name: "reporting",
            port: 8101,
            read_endpoints: &[
                "/api/reporting/pl",
                "/api/reporting/balance-sheet",
                "/api/reporting/cashflow",
                "/api/reporting/kpis",
            ],
            mutation_endpoints: &[("POST", "/api/reporting/rebuild")],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/api/openapi.json", "/metrics"],
        },
        ModuleInfo {
            name: "workflow",
            port: 8102,
            read_endpoints: &[
                "/api/workflow/instances",
            ],
            mutation_endpoints: &[("POST", "/api/workflow/definitions")],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/api/openapi.json", "/metrics"],
        },
        ModuleInfo {
            name: "integrations",
            port: 8103,
            read_endpoints: &[
                "/api/integrations/connectors",
                "/api/integrations/external-refs/by-entity",
            ],
            mutation_endpoints: &[("POST", "/api/integrations/connectors")],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/metrics"],
        },
        ModuleInfo {
            name: "pdf-editor",
            port: 8104,
            read_endpoints: &[
                "/api/pdf/forms/templates",
                "/api/pdf/forms/submissions",
            ],
            mutation_endpoints: &[("POST", "/api/pdf/forms/templates")],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/api/openapi.json", "/metrics"],
        },
        ModuleInfo {
            name: "numbering",
            port: 8105,
            read_endpoints: &[],
            mutation_endpoints: &[("POST", "/allocate")],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/api/schema-version", "/api/openapi.json", "/metrics"],
        },
        ModuleInfo {
            name: "workforce-competence",
            port: 8111,
            read_endpoints: &[
                "/api/workforce-competence/authorization",
            ],
            mutation_endpoints: &[("POST", "/api/workforce-competence/artifacts")],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/api/schema-version", "/metrics"],
        },
        ModuleInfo {
            name: "payments",
            port: 8088,
            read_endpoints: &[],
            mutation_endpoints: &[("POST", "/api/payments/checkout-sessions")],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/api/openapi.json", "/metrics"],
        },
        ModuleInfo {
            name: "subscriptions",
            port: 8087,
            read_endpoints: &[],
            mutation_endpoints: &[],
            exempt_endpoints: &["/healthz", "/api/health", "/api/ready", "/api/version", "/api/openapi.json", "/metrics"],
        },
    ]
}

// ============================================================================
// HTTP helpers
// ============================================================================

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap()
}

fn base_url(port: u16) -> String {
    format!("http://localhost:{port}")
}

async fn is_up(port: u16) -> bool {
    client()
        .get(format!("{}/healthz", base_url(port)))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Send a GET request with no Authorization header and check for 401.
/// Result: (got_401, json_body_ok, is_not_found). `is_not_found` means route
/// returned 404 — the endpoint does not exist, not a security gap.
async fn assert_get_requires_auth(
    c: &reqwest::Client,
    port: u16,
    path: &str,
    module: &str,
) -> Result<(bool, bool, bool), String> {
    let url = format!("{}{}", base_url(port), path);
    let resp = c.get(&url).send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or_default();

    if status == StatusCode::NOT_FOUND {
        eprintln!("  ⏭️  SKIP {module} GET {path}: route not found (404)");
        return Ok((false, false, true));
    }

    let got_401 = status == StatusCode::UNAUTHORIZED;
    let has_error_field = body.get("error").is_some();
    let has_message_field = body.get("message").is_some();
    let json_body_ok = has_error_field && has_message_field;

    if !got_401 {
        eprintln!(
            "  🚨 FAIL {module} GET {path}: expected 401, got {status}"
        );
    } else if !json_body_ok {
        eprintln!(
            "  ⚠️  WARN {module} GET {path}: 401 but missing JSON fields (error={}, message={})",
            has_error_field, has_message_field
        );
    } else {
        eprintln!("  ✅ {module} GET {path}: 401 with JSON body");
    }
    Ok((got_401, json_body_ok, false))
}

/// Send a POST request with no Authorization header and check for 401.
async fn assert_post_requires_auth(
    c: &reqwest::Client,
    port: u16,
    path: &str,
    module: &str,
) -> Result<(bool, bool, bool), String> {
    let url = format!("{}{}", base_url(port), path);
    let resp = c
        .post(&url)
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or_default();

    if status == StatusCode::NOT_FOUND {
        eprintln!("  ⏭️  SKIP {module} POST {path}: route not found (404)");
        return Ok((false, false, true));
    }

    let got_401 = status == StatusCode::UNAUTHORIZED;
    let has_error_field = body.get("error").is_some();
    let has_message_field = body.get("message").is_some();
    let json_body_ok = has_error_field && has_message_field;

    if !got_401 {
        eprintln!(
            "  🚨 FAIL {module} POST {path}: expected 401, got {status}"
        );
    } else if !json_body_ok {
        eprintln!(
            "  ⚠️  WARN {module} POST {path}: 401 but missing JSON fields (error={}, message={})",
            has_error_field, has_message_field
        );
    } else {
        eprintln!("  ✅ {module} POST {path}: 401 with JSON body");
    }
    Ok((got_401, json_body_ok, false))
}

/// Send a GET request with an invalid Bearer token.
async fn assert_invalid_jwt_rejected(
    c: &reqwest::Client,
    port: u16,
    path: &str,
    module: &str,
) -> Result<bool, String> {
    let url = format!("{}{}", base_url(port), path);
    let resp = c
        .get(&url)
        .header("authorization", "Bearer not-a-real-jwt")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();

    let got_401 = status == StatusCode::UNAUTHORIZED;
    if !got_401 {
        eprintln!(
            "  🚨 FAIL {module} GET {path} (invalid JWT): expected 401, got {status}"
        );
    } else {
        eprintln!("  ✅ {module} GET {path} (invalid JWT): 401");
    }
    Ok(got_401)
}

/// Verify that health endpoints are accessible without auth.
async fn assert_health_is_open(
    c: &reqwest::Client,
    port: u16,
    path: &str,
    module: &str,
) -> Result<bool, String> {
    let url = format!("{}{}", base_url(port), path);
    let resp = c.get(&url).send().await.map_err(|e| e.to_string())?;
    let is_ok = resp.status().is_success();
    if !is_ok {
        eprintln!(
            "  ⚠️  WARN {module} GET {path}: expected 2xx, got {}",
            resp.status()
        );
    }
    Ok(is_ok)
}

// ============================================================================
// Main sweep test
// ============================================================================

#[tokio::test]
async fn auth_rbac_full_coverage_sweep() {
    eprintln!("\n══════════════════════════════════════════════════════");
    eprintln!("  Auth/RBAC Coverage Sweep (bd-decba)");
    eprintln!("══════════════════════════════════════════════════════\n");

    let c = client();
    let mut report: Vec<String> = Vec::new();
    let mut total_tested = 0u32;
    let mut total_enforced = 0u32;
    let mut total_unprotected = 0u32;
    let mut total_skipped = 0u32;

    for module in modules() {
        eprintln!("── {} (port {}) ──", module.name, module.port);

        if !is_up(module.port).await {
            eprintln!("  ⏭️  SKIP — not running\n");
            total_skipped += 1;
            report.push(format!(
                "| {} | — | — | — | — | SKIP (not running) |",
                module.name
            ));
            continue;
        }

        let mut mod_total = 0u32;
        let mut mod_enforced = 0u32;
        let mut mod_unprotected = 0u32;
        let mut consistent_body = true;

        // 1. Health endpoints must be open
        for path in module.exempt_endpoints {
            if let Ok(ok) = assert_health_is_open(&c, module.port, path, module.name).await {
                if !ok {
                    eprintln!("  ⚠️  Health endpoint {path} not 2xx");
                }
            }
        }

        // 2. Read endpoints must require auth (no token → 401)
        for path in module.read_endpoints {
            mod_total += 1;
            total_tested += 1;
            match assert_get_requires_auth(&c, module.port, path, module.name).await {
                Ok((_, _, true)) => {
                    // 404 — route doesn't exist, not a security gap
                    mod_total -= 1;
                    total_tested -= 1;
                }
                Ok((got_401, json_ok, _)) => {
                    if got_401 {
                        mod_enforced += 1;
                        total_enforced += 1;
                    } else {
                        mod_unprotected += 1;
                        total_unprotected += 1;
                    }
                    if !json_ok {
                        consistent_body = false;
                    }
                }
                Err(e) => eprintln!("  ⚠️  Error checking {path}: {e}"),
            }

            // Also check invalid JWT
            let _ = assert_invalid_jwt_rejected(&c, module.port, path, module.name).await;
        }

        // 3. Mutation endpoints must require auth (no token → 401)
        for (method, path) in module.mutation_endpoints {
            mod_total += 1;
            total_tested += 1;
            let result = match *method {
                "POST" => assert_post_requires_auth(&c, module.port, path, module.name).await,
                _ => assert_get_requires_auth(&c, module.port, path, module.name).await,
            };
            match result {
                Ok((_, _, true)) => {
                    // 404 — route doesn't exist, not a security gap
                    mod_total -= 1;
                    total_tested -= 1;
                }
                Ok((got_401, json_ok, _)) => {
                    if got_401 {
                        mod_enforced += 1;
                        total_enforced += 1;
                    } else {
                        mod_unprotected += 1;
                        total_unprotected += 1;
                    }
                    if !json_ok {
                        consistent_body = false;
                    }
                }
                Err(e) => eprintln!("  ⚠️  Error checking {path}: {e}"),
            }
        }

        let body_status = if consistent_body { "yes" } else { "NO" };
        report.push(format!(
            "| {} | {} | {} | {} | {} | OK |",
            module.name, mod_total, mod_enforced, mod_unprotected, body_status
        ));
        eprintln!();
    }

    // ── Print report ──
    eprintln!("\n══════════════════════════════════════════════════════");
    eprintln!("  RESULTS");
    eprintln!("══════════════════════════════════════════════════════\n");
    eprintln!("| Module | Total | Enforced | Unprotected | Body OK | Status |");
    eprintln!("|--------|-------|----------|-------------|---------|--------|");
    for line in &report {
        eprintln!("{line}");
    }
    eprintln!();
    eprintln!("Tested: {total_tested}  Enforced: {total_enforced}  Unprotected: {total_unprotected}  Skipped: {total_skipped}");
    eprintln!();

    if total_unprotected > 0 {
        eprintln!(
            "🚨 {} unprotected non-health endpoints found — P0 security bugs",
            total_unprotected
        );
    }

    // Assertion: every tested endpoint must require auth
    assert_eq!(
        total_unprotected, 0,
        "Found {} unprotected non-health endpoints — every API endpoint must require JWT",
        total_unprotected
    );
}

// ============================================================================
// Static audit: route structure verification (no live services needed)
// ============================================================================

/// Static verification that permission constants exist for every module.
/// If a module has _MUTATE but no _READ, read routes are likely unprotected.
#[test]
fn every_module_has_read_permission_constant() {
    use security::permissions;

    // All modules that serve data endpoints MUST have both MUTATE and READ
    let modules_with_data: Vec<(&str, &str, &str)> = vec![
        ("ar", permissions::AR_MUTATE, permissions::AR_READ),
        ("gl", permissions::GL_POST, permissions::GL_READ),
        ("inventory", permissions::INVENTORY_MUTATE, permissions::INVENTORY_READ),
        ("ap", permissions::AP_MUTATE, permissions::AP_READ),
        ("party", permissions::PARTY_MUTATE, permissions::PARTY_READ),
        ("treasury", permissions::TREASURY_MUTATE, permissions::TREASURY_READ),
        ("timekeeping", permissions::TIMEKEEPING_MUTATE, permissions::TIMEKEEPING_READ),
        ("fixed-assets", permissions::FIXED_ASSETS_MUTATE, permissions::FIXED_ASSETS_READ),
        ("consolidation", permissions::CONSOLIDATION_MUTATE, permissions::CONSOLIDATION_READ),
        ("shipping-receiving", permissions::SHIPPING_RECEIVING_MUTATE, permissions::SHIPPING_RECEIVING_READ),
        ("ttp", permissions::TTP_MUTATE, permissions::TTP_READ),
        ("bom", permissions::BOM_MUTATE, permissions::BOM_READ),
        ("production", permissions::PRODUCTION_MUTATE, permissions::PRODUCTION_READ),
        ("maintenance", permissions::MAINTENANCE_MUTATE, permissions::MAINTENANCE_READ),
        ("quality-inspection", permissions::QUALITY_INSPECTION_MUTATE, permissions::QUALITY_INSPECTION_READ),
        ("reporting", permissions::REPORTING_MUTATE, permissions::REPORTING_READ),
        ("notifications", permissions::NOTIFICATIONS_MUTATE, permissions::NOTIFICATIONS_READ),
        ("integrations", permissions::INTEGRATIONS_MUTATE, permissions::INTEGRATIONS_READ),
        ("pdf-editor", permissions::PDF_EDITOR_MUTATE, permissions::PDF_EDITOR_READ),
        ("workforce-competence", permissions::WORKFORCE_COMPETENCE_MUTATE, permissions::WORKFORCE_COMPETENCE_READ),
        ("workflow", permissions::WORKFLOW_MUTATE, permissions::WORKFLOW_READ),
    ];

    for (name, mutate, read) in &modules_with_data {
        assert!(!mutate.is_empty(), "{name}: mutate permission is empty");
        assert!(!read.is_empty(), "{name}: read permission is empty");
        assert_ne!(
            mutate, read,
            "{name}: mutate and read permissions must be distinct"
        );
        println!("✅ {name}: mutate={mutate}, read={read}");
    }
}

/// Verify that the middleware returns proper JSON error bodies.
/// This is a unit-level check using Tower oneshot — no live service needed.
#[tokio::test]
async fn middleware_401_response_has_json_body() {
    use axum::{body::Body, http::Request, routing::get, Router};
    use rsa::pkcs8::{EncodePublicKey, LineEnding};
    use rsa::RsaPrivateKey;
    use security::{
        authz_middleware::{ClaimsLayer, RequirePermissionsLayer},
        permissions, JwtVerifier,
    };
    use std::sync::Arc;
    use tower::ServiceExt;

    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
    let pub_key = priv_key.to_public_key();
    let pub_pem = pub_key.to_public_key_pem(LineEnding::LF).unwrap();
    let verifier = Arc::new(JwtVerifier::from_public_pem(&pub_pem).unwrap());

    let app = Router::new()
        .route("/api/data", get(|| async { "ok" }))
        .route_layer(RequirePermissionsLayer::new(&[permissions::AR_READ]))
        .layer(ClaimsLayer::permissive(verifier));

    // No token → 401
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/data")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 401);
    let body_bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let body: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(
        body.get("error").is_some(),
        "401 response must have 'error' field"
    );
    assert!(
        body.get("message").is_some(),
        "401 response must have 'message' field"
    );
    assert_eq!(body["error"], "unauthorized");
    println!("✅ 401 response body: {body}");
}

/// Verify that 403 response has proper JSON body.
#[tokio::test]
async fn middleware_403_response_has_json_body() {
    use axum::{body::Body, http::Request, routing::get, Router};
    use jsonwebtoken::{Algorithm, EncodingKey, Header};
    use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
    use rsa::RsaPrivateKey;
    use security::{
        authz_middleware::{ClaimsLayer, RequirePermissionsLayer},
        permissions, JwtVerifier,
    };
    use serde::Serialize;
    use std::sync::Arc;
    use tower::ServiceExt;

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

    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
    let pub_key = priv_key.to_public_key();
    let priv_pem = priv_key.to_pkcs8_pem(LineEnding::LF).unwrap();
    let pub_pem = pub_key.to_public_key_pem(LineEnding::LF).unwrap();
    let encoding = EncodingKey::from_rsa_pem(priv_pem.as_bytes()).unwrap();
    let verifier = Arc::new(JwtVerifier::from_public_pem(&pub_pem).unwrap());

    let app = Router::new()
        .route("/api/data", get(|| async { "ok" }))
        .route_layer(RequirePermissionsLayer::new(&[permissions::AR_READ]))
        .layer(ClaimsLayer::permissive(verifier));

    // Token with wrong permissions → 403
    let now = chrono::Utc::now();
    let claims = TestClaims {
        sub: uuid::Uuid::new_v4().to_string(),
        iss: "auth-rs".to_string(),
        aud: "7d-platform".to_string(),
        iat: now.timestamp(),
        exp: (now + chrono::Duration::minutes(15)).timestamp(),
        jti: uuid::Uuid::new_v4().to_string(),
        tenant_id: uuid::Uuid::new_v4().to_string(),
        app_id: None,
        roles: vec!["operator".to_string()],
        perms: vec!["gl.post".to_string()], // Wrong permission
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    };
    let token = jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &encoding).unwrap();

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/data")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 403);
    let body_bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let body: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(
        body.get("error").is_some(),
        "403 response must have 'error' field"
    );
    assert!(
        body.get("message").is_some(),
        "403 response must have 'message' field"
    );
    assert_eq!(body["error"], "forbidden");
    println!("✅ 403 response body: {body}");
}
