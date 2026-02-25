pub mod types;
pub mod state_machine;
pub mod guards;
pub mod service;

pub use types::{Direction, InboundStatus, OutboundStatus, LineQty};
pub use state_machine::{
    validate_inbound, validate_outbound,
    inbound_transitions, outbound_transitions,
    TransitionError,
};
pub use guards::{
    run_inbound_guards, run_outbound_guards,
    GuardError, InboundGuardContext, OutboundGuardContext,
};
pub use service::{Shipment, ShipmentError, ShipmentService, TransitionRequest};
