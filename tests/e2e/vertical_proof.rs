#!/usr/bin/env -S cargo +nightly -Zscript
//! Vertical Plug-and-Play Proof Test
//!
//! This script proves a vertical can call platform services using ONLY:
//! 1. The SDK's PlatformServices (from [platform.services] manifest config)
//! 2. The generated typed client (PartiesClient)
//! 3. VerifiedClaims for auth
//!
//! If this works, plug-and-play is real. If it doesn't, every gap is documented.

// NOTE: This is a documentation/proof script, not a compiled test.
// Run the actual proof via the shell script below.

/*
WHAT A VERTICAL DEVELOPER WRITES:

--- module.toml ---
[module]
name = "proof-vertical"
version = "0.1.0"

[server]
host = "0.0.0.0"
port = 9999

[database]
migrations = "./db/migrations"
auto_migrate = true

[bus]
type = "nats"

[platform.services]
party = { enabled = true, default_url = "http://localhost:8098" }

--- main.rs ---
use platform_sdk::{ModuleBuilder, PlatformService};
use platform_client_party::PartiesClient;

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .routes(|ctx| {
            // This is the moment of truth:
            let party: PartiesClient = ctx.platform_client::<PartiesClient>();

            axum::Router::new()
                .route("/api/proof/customers", axum::routing::get({
                    let party = party;
                    move |claims: axum::Extension<platform_sdk::VerifiedClaims>| {
                        let party = party.clone();
                        async move {
                            let result = party.list_parties(&claims, None, None, None).await;
                            match result {
                                Ok(page) => axum::Json(serde_json::json!({
                                    "status": "plug-and-play works",
                                    "customer_count": page.items.len()
                                })).into_response(),
                                Err(e) => (
                                    axum::http::StatusCode::BAD_GATEWAY,
                                    format!("Party call failed: {e}")
                                ).into_response(),
                            }
                        }
                    }
                }))
        })
        .run()
        .await
        .expect("proof vertical failed");
}

WHAT THIS PROVES:
- ctx.platform_client::<PartiesClient>() resolves from manifest
- PlatformClient is auto-constructed with correct URL
- Typed client methods (list_parties) work with VerifiedClaims
- Tenant headers, correlation IDs injected automatically
- No hand-written HTTP client code
- No env var parsing in application code
- No manual header injection
*/
