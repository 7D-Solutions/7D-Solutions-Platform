//! AR domain layer — business logic and data access separated from HTTP handlers.

pub mod charges;
pub mod customers;
pub mod disputes;
pub mod events;
pub mod health;
pub mod invoices;
pub mod payment_methods;
pub mod refunds;
pub mod subscriptions;
pub mod tax_config;
pub mod usage;
pub mod webhooks;
