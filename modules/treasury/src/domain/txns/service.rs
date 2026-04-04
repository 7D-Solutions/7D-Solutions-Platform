//! Service functions for bank transaction ingestion.

pub use super::repo::{
    default_account_id, insert_bank_txn_tx, is_event_processed, record_processed_event_tx,
};
