//! Outside-Processing event contracts v1
//!
//! Events produced:
//!   outside_processing.order_created, .order_issued, .order_closed, .order_cancelled
//!   outside_processing.shipment_requested, .shipped, .returned
//!   outside_processing.review_completed, .re_identification_recorded
//!
//! All events carry EventEnvelope with source_module="outside-processing".

pub mod envelope;
pub mod produced;

pub const OP_SCHEMA_VERSION: &str = "1.0.0";
pub const SOURCE_MODULE: &str = "outside-processing";

pub const MUTATION_DATA: &str = "DATA_MUTATION";
pub const MUTATION_LIFECYCLE: &str = "LIFECYCLE";
pub const MUTATION_REVERSAL: &str = "REVERSAL";

pub use produced::*;
pub use envelope::create_op_envelope;
