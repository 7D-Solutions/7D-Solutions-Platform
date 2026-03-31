//! Work order service — Guard→Mutation→Outbox for all WO lifecycle operations.
//!
//! Split:
//! - `core`: types, CRUD, and queries
//! - `transitions`: status-change logic with guard enforcement and GL cost payload

mod core;
mod transitions;

pub use self::core::{
    CreateWorkOrderRequest, ListWorkOrdersQuery, TransitionRequest, WoError, WorkOrder,
    WorkOrderRepo,
};
