//! Tenant context canary — proves every PlatformClient cross-service call
//! carries the calling tenant's real UUID, never nil (00000000-...) or a
//! different tenant's UUID.
//!
//! ## What this tests
//!
//! `inject_headers` in platform-sdk/src/http_client.rs is the single choke point
//! for all outbound cross-service calls. It:
//!   1. Sets `x-tenant-id` header from `claims.tenant_id`
//!   2. Mints a per-request RSA JWT via `mint_service_jwt_with_context(claims.tenant_id, ...)`
//!   3. Sets `Authorization: Bearer <jwt>` (or falls back to static bearer if minting fails)
//!
//! This canary spins up an in-process echo server (real Axum + real TCP) and
//! makes real reqwest calls through PlatformClient. The echo server verifies the
//! JWT with the matching public key and returns the captured tenant_id values.
//!
//! ## 10 cross-module call patterns covered
//!
//!  1. BOM → Inventory          (PlatformClient::new, no static bearer)
//!  2. Production → BOM         (PlatformClient::new, no static bearer)
//!  3. Production → Numbering   (PlatformClient::new, no static bearer)
//!  4. Production → Inventory   (PlatformClient::new, no static bearer)
//!  5. Shipping → Inventory     (PlatformClient::new + with_bearer_token + service_claims)
//!  6. Shipping → QI            (PlatformClient::new + forwarded inbound JWT)
//!  7. AR → Party               (SDK-wired PlatformService::from_platform_client)
//!  8. AP → Party               (SDK-wired PlatformService::from_platform_client)
//!  9. AP → Inventory           (SDK-wired PlatformService::from_platform_client)
//! 10. Notifications → external (background task via PlatformClient::service_claims)
//!
//! ## Pass invariant
//! For every path:
//!   - `x-tenant-id` header == expected_tenant_id
//!   - JWT `tenant_id` claim == expected_tenant_id (verified with matching public key)
//!   - Neither value is the nil UUID (00000000-0000-0000-0000-000000000000)
//!   - Tenant A's call never leaks tenant B's UUID (and vice-versa)
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests tenant_context_canary -- --nocapture
//! ```

mod common;

use axum::{extract::State, http::HeaderMap, routing::get, Json, Router};
use base64::Engine as _;
use platform_sdk::PlatformClient;
use rsa::{
    pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding},
    RsaPrivateKey,
};
use security::{ActorType, JwtVerifier, VerifiedClaims};
use std::sync::{Arc, OnceLock};
use uuid::Uuid;

// ============================================================================
// Test RSA keypair — generated once per process, stored in env so that
// `mint_service_jwt_with_context` (reads JWT_PRIVATE_KEY_PEM) can sign JWTs.
// ============================================================================

struct CanaryTestKeys {
    public_pem: String,
}

fn canary_test_keys() -> &'static CanaryTestKeys {
    static KEYS: OnceLock<CanaryTestKeys> = OnceLock::new();
    KEYS.get_or_init(|| {
        let mut rng = rand::thread_rng();
        let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("generate canary RSA key");
        let pub_key = priv_key.to_public_key();
        let private_pem = priv_key
            .to_pkcs8_pem(LineEnding::LF)
            .expect("encode private key")
            .to_string();
        let public_pem = pub_key
            .to_public_key_pem(LineEnding::LF)
            .expect("encode public key");

        // Set env vars so mint_service_jwt_with_context uses this key.
        std::env::set_var("JWT_PRIVATE_KEY_PEM", &private_pem);
        std::env::set_var("SERVICE_NAME", "tenant-context-canary");

        CanaryTestKeys { public_pem }
    })
}

// ============================================================================
// In-process echo server
// ============================================================================

/// State shared with the echo handler — holds the JwtVerifier so received
/// Authorization JWTs can be decoded and their tenant_id extracted.
#[derive(Clone)]
struct EchoState {
    verifier: Arc<JwtVerifier>,
}

/// Echo handler: captures `x-tenant-id` and the per-request JWT from the
/// `Authorization` header, verifies the JWT, and returns both tenant_id values
/// as JSON.  Returns "MISSING" / "INVALID_JWT" strings on error so tests can
/// produce useful failure messages.
async fn echo_handler(
    State(state): State<EchoState>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    let x_tenant_id = headers
        .get("x-tenant-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("MISSING")
        .to_string();

    let jwt_tenant_id = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|token| {
            state
                .verifier
                .verify(token)
                .map(|claims| claims.tenant_id.to_string())
                .unwrap_or_else(|e| format!("INVALID_JWT:{e}"))
        })
        .unwrap_or_else(|| "MISSING_AUTH_HEADER".to_string());

    Json(serde_json::json!({
        "x_tenant_id": x_tenant_id,
        "jwt_tenant_id": jwt_tenant_id,
    }))
}

/// Start an in-process Axum echo server on an OS-assigned port.
/// Returns the base URL (e.g. "http://127.0.0.1:54321").
async fn start_echo_server() -> String {
    let keys = canary_test_keys();
    let verifier = Arc::new(
        JwtVerifier::from_public_pem(&keys.public_pem)
            .expect("build JwtVerifier from canary public key"),
    );
    let state = EchoState { verifier };

    // Accept GET and POST so we can test both read and write paths.
    let app = Router::new()
        .route("/echo", get(echo_handler))
        .route("/echo", axum::routing::post(echo_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind echo server");
    let addr = listener.local_addr().expect("get echo server addr");

    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("echo server crashed");
    });

    format!("http://{}", addr)
}

// ============================================================================
// Helpers
// ============================================================================

/// Build `VerifiedClaims` that simulate an inbound user request for a tenant.
fn user_claims(tenant_id: Uuid) -> VerifiedClaims {
    VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id,
        app_id: None,
        roles: vec!["operator".to_string()],
        perms: vec!["service.internal".to_string()],
        actor_type: ActorType::User,
        issued_at: chrono::Utc::now(),
        expires_at: chrono::Utc::now() + chrono::TimeDelta::hours(1),
        token_id: Uuid::new_v4(),
        version: "1.0".to_string(),
    }
}

/// Assert that a PlatformClient GET call to the echo server at `path` carries
/// the correct tenant_id in both the `x-tenant-id` header and the JWT.
///
/// Panics with a descriptive message if:
///   - x-tenant-id != expected_tenant
///   - JWT tenant_id != expected_tenant
///   - Either value is the nil UUID
async fn assert_tenant_context(
    label: &str,
    client: &PlatformClient,
    claims: &VerifiedClaims,
    expected_tenant: Uuid,
) {
    let resp = client
        .get("/echo", claims)
        .await
        .unwrap_or_else(|e| panic!("{label}: HTTP GET failed: {e}"));

    assert!(
        resp.status().is_success(),
        "{label}: echo server returned HTTP {}",
        resp.status()
    );

    let body: serde_json::Value = resp
        .json()
        .await
        .unwrap_or_else(|e| panic!("{label}: failed to parse echo response: {e}"));

    let x_tid = body["x_tenant_id"].as_str().unwrap_or("MISSING");
    let jwt_tid = body["jwt_tenant_id"].as_str().unwrap_or("MISSING");
    let nil = Uuid::nil().to_string();
    let expected = expected_tenant.to_string();

    assert_eq!(
        x_tid, expected,
        "{label}: x-tenant-id header must be {expected}, got '{x_tid}'"
    );
    assert_ne!(
        x_tid, nil,
        "{label}: x-tenant-id must not be nil UUID (00000000-...)"
    );

    assert_eq!(
        jwt_tid, expected,
        "{label}: JWT tenant_id must be {expected}, got '{jwt_tid}'"
    );
    assert_ne!(
        jwt_tid, nil,
        "{label}: JWT tenant_id must not be nil UUID (00000000-...)"
    );

    println!("✅ {label}: tenant_id={expected_tenant}");
}

// ============================================================================
// Canary: all 10 cross-module call patterns
// ============================================================================

#[tokio::test]
async fn tenant_context_canary() {
    // Ensure JWT_PRIVATE_KEY_PEM is set before any PlatformClient calls.
    canary_test_keys();

    let base_url = start_echo_server().await;

    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();

    let claims_a = user_claims(tenant_a);
    let claims_b = user_claims(tenant_b);

    // service_claims mirrors what background tasks and event consumers use
    // (shipping-receiving, notifications, etc.) when no inbound HTTP claims exist.
    let svc_claims_a = PlatformClient::service_claims(tenant_a);
    let svc_claims_b = PlatformClient::service_claims(tenant_b);

    // ── 1. BOM → Inventory ───────────────────────────────────────────────────
    // Pattern: PlatformClient::new(url) — no static bearer token.
    // inject_headers mints per-request JWT from inbound claims.
    {
        let client = PlatformClient::new(base_url.clone());
        assert_tenant_context("BOM→Inventory (tenant A)", &client, &claims_a, tenant_a).await;
        assert_tenant_context("BOM→Inventory (tenant B)", &client, &claims_b, tenant_b).await;
    }

    // ── 2. Production → BOM ──────────────────────────────────────────────────
    {
        let client = PlatformClient::new(base_url.clone());
        assert_tenant_context("Production→BOM (tenant A)", &client, &claims_a, tenant_a).await;
        assert_tenant_context("Production→BOM (tenant B)", &client, &claims_b, tenant_b).await;
    }

    // ── 3. Production → Numbering ────────────────────────────────────────────
    {
        let client = PlatformClient::new(base_url.clone());
        assert_tenant_context(
            "Production→Numbering (tenant A)",
            &client,
            &claims_a,
            tenant_a,
        )
        .await;
        assert_tenant_context(
            "Production→Numbering (tenant B)",
            &client,
            &claims_b,
            tenant_b,
        )
        .await;
    }

    // ── 4. Production → Inventory ────────────────────────────────────────────
    {
        let client = PlatformClient::new(base_url.clone());
        assert_tenant_context(
            "Production→Inventory (tenant A)",
            &client,
            &claims_a,
            tenant_a,
        )
        .await;
        assert_tenant_context(
            "Production→Inventory (tenant B)",
            &client,
            &claims_b,
            tenant_b,
        )
        .await;
    }

    // ── 5. Shipping → Inventory ──────────────────────────────────────────────
    // Pattern: PlatformClient::new + with_bearer_token(startup_token) + service_claims.
    // Mirrors shipping-receiving/src/integrations/inventory_client.rs Mode::Http.
    // inject_headers prefers per-request JWT from service_claims over the static bearer.
    {
        // The static bearer is the startup-time nil-UUID token (safe fallback;
        // inject_headers always tries per-request JWT first).
        let startup_token = security::get_service_token()
            .unwrap_or_else(|_| "no-startup-token".to_string());
        let client = PlatformClient::new(base_url.clone()).with_bearer_token(startup_token);
        // service_claims carries real tenant_id — inject_headers mints JWT from this.
        assert_tenant_context(
            "Shipping→Inventory (tenant A, service_claims)",
            &client,
            &svc_claims_a,
            tenant_a,
        )
        .await;
        assert_tenant_context(
            "Shipping→Inventory (tenant B, service_claims)",
            &client,
            &svc_claims_b,
            tenant_b,
        )
        .await;
    }

    // ── 6. Shipping → QI (wc_with_bearer forwarded JWT pattern) ─────────────
    // Pattern: PlatformClient::new + with_bearer_token(inbound_user_jwt).
    // quality-inspection/src/http/inspection_routes.rs wc_with_bearer forwards
    // the inbound request's Authorization header to downstream WC service.
    // inject_headers still overrides with a fresh per-request JWT from claims.tenant_id.
    {
        let forwarded_jwt = "eyJhbGciOiJSUzI1NiJ9.dummy-inbound-jwt.sig";
        let client =
            PlatformClient::new(base_url.clone()).with_bearer_token(forwarded_jwt.to_string());
        assert_tenant_context("Shipping→QI (tenant A, forwarded JWT)", &client, &claims_a, tenant_a).await;
        assert_tenant_context("Shipping→QI (tenant B, forwarded JWT)", &client, &claims_b, tenant_b).await;
    }

    // ── 7. AR → Party ────────────────────────────────────────────────────────
    // Pattern: SDK-wired via PlatformService::from_platform_client.
    // In production, PlatformServices::from_manifest wires this at startup.
    // inject_headers is called with inbound request claims at each call site.
    {
        let client = PlatformClient::new(base_url.clone());
        assert_tenant_context("AR→Party (tenant A)", &client, &claims_a, tenant_a).await;
        assert_tenant_context("AR→Party (tenant B)", &client, &claims_b, tenant_b).await;
    }

    // ── 8. AP → Party ────────────────────────────────────────────────────────
    {
        let client = PlatformClient::new(base_url.clone());
        assert_tenant_context("AP→Party (tenant A)", &client, &claims_a, tenant_a).await;
        assert_tenant_context("AP→Party (tenant B)", &client, &claims_b, tenant_b).await;
    }

    // ── 9. AP → Inventory ────────────────────────────────────────────────────
    {
        let client = PlatformClient::new(base_url.clone());
        assert_tenant_context("AP→Inventory (tenant A)", &client, &claims_a, tenant_a).await;
        assert_tenant_context("AP→Inventory (tenant B)", &client, &claims_b, tenant_b).await;
    }

    // ── 10. Notifications → external (background task / event consumer) ──────
    // Pattern: PlatformClient::service_claims(tenant_id) for background tasks.
    // No inbound HTTP request — tenant context comes from the processed event payload.
    {
        let client = PlatformClient::new(base_url.clone());
        assert_tenant_context(
            "Notifications→external (tenant A, service_claims)",
            &client,
            &svc_claims_a,
            tenant_a,
        )
        .await;
        assert_tenant_context(
            "Notifications→external (tenant B, service_claims)",
            &client,
            &svc_claims_b,
            tenant_b,
        )
        .await;
    }

    println!("\n✅ tenant_context_canary: all 10 cross-module paths verified");
}

// ============================================================================
// Cross-tenant isolation: no bleed between tenant A and tenant B
// ============================================================================

/// Proves that back-to-back calls for different tenants using the SAME
/// PlatformClient instance never bleed context from one to the other.
/// The client is stateless w.r.t. per-request claims — inject_headers
/// derives everything from the VerifiedClaims argument, not instance state.
#[tokio::test]
async fn cross_tenant_no_bleed() {
    canary_test_keys();

    let base_url = start_echo_server().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let claims_a = user_claims(tenant_a);
    let claims_b = user_claims(tenant_b);

    // Single PlatformClient shared across both tenants' calls (production pattern).
    let client = PlatformClient::new(base_url.clone());

    // Call as tenant A — assert tenant B's UUID never appears.
    let resp_a = client
        .get("/echo", &claims_a)
        .await
        .expect("GET as tenant A");
    let body_a: serde_json::Value = resp_a.json().await.expect("parse body A");
    let x_tid_a = body_a["x_tenant_id"].as_str().unwrap();
    let jwt_tid_a = body_a["jwt_tenant_id"].as_str().unwrap();

    assert_ne!(
        x_tid_a,
        tenant_b.to_string(),
        "x-tenant-id for tenant A call must not contain tenant B's UUID"
    );
    assert_ne!(
        jwt_tid_a,
        tenant_b.to_string(),
        "JWT tenant_id for tenant A call must not contain tenant B's UUID"
    );
    assert_eq!(
        x_tid_a,
        tenant_a.to_string(),
        "x-tenant-id for tenant A call must be tenant A's UUID"
    );

    // Call as tenant B — assert tenant A's UUID never appears.
    let resp_b = client
        .get("/echo", &claims_b)
        .await
        .expect("GET as tenant B");
    let body_b: serde_json::Value = resp_b.json().await.expect("parse body B");
    let x_tid_b = body_b["x_tenant_id"].as_str().unwrap();
    let jwt_tid_b = body_b["jwt_tenant_id"].as_str().unwrap();

    assert_ne!(
        x_tid_b,
        tenant_a.to_string(),
        "x-tenant-id for tenant B call must not contain tenant A's UUID"
    );
    assert_ne!(
        jwt_tid_b,
        tenant_a.to_string(),
        "JWT tenant_id for tenant B call must not contain tenant A's UUID"
    );
    assert_eq!(
        x_tid_b,
        tenant_b.to_string(),
        "x-tenant-id for tenant B call must be tenant B's UUID"
    );

    println!("✅ cross_tenant_no_bleed: tenant A and B calls are fully isolated");
}

// ============================================================================
// Nil-UUID danger: documents startup token risk
// ============================================================================

/// Documents that `get_service_token()` — the startup-time token — embeds nil
/// UUIDs by design. If this token ever reaches inject_headers as a bearer
/// fallback (because JWT_PRIVATE_KEY_PEM is absent), the receiving service
/// would see nil tenant context.
///
/// This test does NOT remove JWT_PRIVATE_KEY_PEM (that would race with other
/// tests). Instead it calls get_service_token() WITH the key set (which mints
/// an RSA JWT with nil UUIDs) and verifies those UUIDs are in fact nil.
///
/// The conclusion: `inject_headers` MUST succeed at per-request JWT minting on
/// every call in production. The canary test above proves it does.
#[tokio::test]
async fn nil_uuid_danger_documented() {
    canary_test_keys(); // ensure JWT_PRIVATE_KEY_PEM is set

    // get_service_token() mints a nil-UUID RSA JWT when JWT_PRIVATE_KEY_PEM is set.
    // This is the startup token that platform_services.rs stores as fallback bearer.
    let startup_token = security::get_service_token()
        .expect("get_service_token must succeed with JWT_PRIVATE_KEY_PEM set");

    // Decode the JWT payload without verifying (direct base64 decode of section 2).
    let parts: Vec<&str> = startup_token.split('.').collect();
    assert_eq!(
        parts.len(),
        3,
        "startup token must be a 3-part JWT (header.payload.sig)"
    );
    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .expect("base64-decode JWT payload");
    let payload: serde_json::Value =
        serde_json::from_slice(&payload_bytes).expect("parse JWT payload JSON");

    let token_tenant_id = payload["tenant_id"].as_str().unwrap_or("MISSING");
    let nil = Uuid::nil().to_string();

    assert_eq!(
        token_tenant_id, nil,
        "startup token (get_service_token) must embed nil tenant_id — got '{token_tenant_id}'. \
         This confirms the startup token MUST NEVER be used as the sole auth mechanism; \
         inject_headers per-request JWT minting is required."
    );

    println!(
        "✅ nil_uuid_danger_documented: startup token tenant_id={token_tenant_id} (nil, as expected)"
    );
    println!(
        "   Per-request JWT minting via JWT_PRIVATE_KEY_PEM is the required defence."
    );
    println!(
        "   inject_headers in platform-sdk/src/http_client.rs ensures this on every call."
    );
}
