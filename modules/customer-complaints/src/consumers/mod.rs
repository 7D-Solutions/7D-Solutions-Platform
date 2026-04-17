//! Event consumers for customer-complaints.
//!
//! - party_deactivated: flags open complaints when a party is deactivated
//! - order_shipped: logs order shipment context on linked complaints
//! - shipment_received: logs inbound shipment context on linked complaints

pub mod order_shipped;
pub mod party_deactivated;
pub mod shipment_received;
