pub mod guards;
pub mod labor;
pub mod parts;
pub mod service;
pub mod state_machine;
pub mod types;

pub use guards::{
    run_guards, validate_close_fields, validate_completion_fields, GuardError, TransitionContext,
};
pub use labor::{AddLaborRequest, WoLabor, WoLaborError, WoLaborRepo};
pub use parts::{AddPartRequest, WoPart, WoPartError, WoPartsRepo};
pub use service::{
    CreateWorkOrderRequest, ListWorkOrdersQuery, TransitionRequest, WoError, WorkOrder,
    WorkOrderRepo,
};
pub use state_machine::validate_transition;
pub use state_machine::{allowed_transitions, TransitionError};
pub use types::*;
