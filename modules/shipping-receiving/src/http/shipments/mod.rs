//! HTTP handlers for shipment CRUD and lifecycle endpoints.
//!
//! All endpoints extract tenant_id from VerifiedClaims — never from JSON input.
//! Write endpoints accept an optional Idempotency-Key header.

mod create_label;
mod handlers;
pub mod types;

pub use create_label::create_label;
pub use handlers::*;
pub use types::ShipmentLineRow;
