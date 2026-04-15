//! Party bounded context — types, service, and domain logic.

pub mod create;
pub mod models;
pub mod query;
pub mod service;
pub mod update;
pub mod validation;

pub use models::{
    CreateCompanyRequest, CreateIndividualRequest, ExternalRef, Party, PartyCompany, PartyError,
    PartyIndividual, PartyView, SearchQuery, UpdatePartyRequest,
};
