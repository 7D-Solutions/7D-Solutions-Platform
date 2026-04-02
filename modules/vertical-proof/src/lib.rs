//! Vertical proof module — proves a vertical can call 5 platform modules
//! using only `ctx.platform_client::<T>()` and the typed client crates.
//!
//! This module exists solely to validate the plug-and-play developer experience.
//! It calls Party, AR, Inventory, Production, and Notifications from a single
//! vertical, subscribes to an AR event, and publishes its own event through
//! the outbox.

use platform_sdk::VerifiedClaims;

pub mod wiring_test;

/// Build service-level claims for test calls that don't originate from
/// an HTTP request. Uses a fixed test tenant ID.
pub fn test_claims() -> VerifiedClaims {
    let tenant_id = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001")
        .expect("valid uuid");
    platform_sdk::PlatformClient::service_claims(tenant_id)
}
