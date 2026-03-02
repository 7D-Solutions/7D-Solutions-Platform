pub mod guards;
pub mod service;
pub mod state_machine;
pub mod types;

pub use guards::{
    run_inbound_guards, run_outbound_guards, GuardError, InboundGuardContext, OutboundGuardContext,
};
pub use service::{Shipment, ShipmentError, ShipmentService, TransitionRequest};
pub use state_machine::{
    inbound_transitions, outbound_transitions, validate_inbound, validate_outbound, TransitionError,
};
pub use types::{Direction, InboundStatus, LineQty, OutboundStatus};
