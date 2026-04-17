//! Customer-Complaints event contracts v1.
//!
//! Events produced:
//!   customer_complaints.complaint.received, .triaged, .status_changed, .assigned,
//!   .customer_communicated, .resolved, .closed, .overdue

pub mod envelope;
pub mod produced;

pub const CC_SCHEMA_VERSION: &str = "1.0.0";
pub const SOURCE_MODULE: &str = "customer-complaints";

pub const MUTATION_DATA: &str = "DATA_MUTATION";
pub const MUTATION_LIFECYCLE: &str = "LIFECYCLE";

pub use produced::*;
pub use envelope::create_cc_envelope;
