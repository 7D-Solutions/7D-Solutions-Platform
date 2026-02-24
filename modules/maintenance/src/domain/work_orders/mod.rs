pub mod types;
pub mod state_machine;
pub mod guards;
pub mod labor;
pub mod parts;
pub mod service;

pub use types::*;
pub use state_machine::validate_transition;
pub use guards::{run_guards, validate_close_fields, validate_completion_fields, GuardError, TransitionContext};
pub use state_machine::{allowed_transitions, TransitionError};
pub use service::{
    CreateWorkOrderRequest, ListWorkOrdersQuery, TransitionRequest, WoError, WorkOrder,
    WorkOrderRepo,
};
pub use parts::{AddPartRequest, WoPart, WoPartError, WoPartsRepo};
pub use labor::{AddLaborRequest, WoLabor, WoLaborError, WoLaborRepo};
