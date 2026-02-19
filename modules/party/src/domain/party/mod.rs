//! Party bounded context — types, service, and domain logic.

pub mod models;
pub mod service;

pub use models::{
    CreateCompanyRequest, CreateIndividualRequest, ExternalRef, Party, PartyCompany,
    PartyError, PartyIndividual, PartyView, SearchQuery, UpdatePartyRequest,
};
