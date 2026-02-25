//! HTTP handlers for shipment CRUD and lifecycle endpoints.
//!
//! All endpoints extract tenant_id from VerifiedClaims — never from JSON input.
//! Write endpoints accept an optional Idempotency-Key header.

mod handlers;
mod types;

pub use handlers::*;
pub use types::ShipmentLineRow;
