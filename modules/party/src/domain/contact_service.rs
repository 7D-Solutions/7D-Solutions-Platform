pub mod guards;
pub mod mutation;
pub mod query;

pub use mutation::{create_contact, deactivate_contact, set_primary_for_role, update_contact};
pub use query::{get_contact, get_primary_contacts, list_contacts};
