//! Consolidated trial balance engine.
//!
//! Deterministic pipeline: fetch snapshots → verify hashes → COA map → FX translate → eliminate → cache.

pub mod compute;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("GL client error: {0}")]
    GlClient(#[from] platform_sdk::ClientError),

    #[error("Period not closed for entity {0}")]
    PeriodNotClosed(String),

    #[error("Hash verification failed for entity {entity}: stored={stored}, live={live}")]
    HashMismatch {
        entity: String,
        stored: String,
        live: String,
    },

    #[error("Missing COA mapping for entity {entity}, account {account}")]
    MissingCoaMapping { entity: String, account: String },

    #[error("Missing FX policy for entity {0}")]
    MissingFxPolicy(String),

    #[error("FX rate not found: {from_currency}/{to_currency} for entity {entity}")]
    FxRateNotFound {
        entity: String,
        from_currency: String,
        to_currency: String,
    },

    #[error("Config error: {0}")]
    Config(#[from] super::config::ConfigError),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// A single row in the consolidated trial balance (pre-cache).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConsolidatedTbRow {
    pub account_code: String,
    pub account_name: String,
    pub currency: String,
    pub debit_minor: i64,
    pub credit_minor: i64,
    pub net_minor: i64,
}

/// Result of a consolidation run.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConsolidationResult {
    pub group_id: Uuid,
    pub as_of: NaiveDate,
    pub reporting_currency: String,
    pub rows: Vec<ConsolidatedTbRow>,
    pub input_hash: String,
    pub entity_hashes: Vec<EntityHashEntry>,
}

/// Tracks the close_hash used for each entity so we can verify determinism.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EntityHashEntry {
    pub entity_tenant_id: String,
    pub close_hash: String,
}

/// Compute a deterministic input_hash from sorted entity close hashes.
///
/// SHA-256(group_id | as_of | entity1_tenant:hash1 | entity2_tenant:hash2 | …)
/// Entities are sorted by tenant_id for determinism.
pub fn compute_input_hash(
    group_id: Uuid,
    as_of: NaiveDate,
    entity_hashes: &mut [EntityHashEntry],
) -> String {
    entity_hashes.sort_by(|a, b| a.entity_tenant_id.cmp(&b.entity_tenant_id));

    let mut hasher = Sha256::new();
    hasher.update(group_id.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(as_of.to_string().as_bytes());
    for eh in entity_hashes.iter() {
        hasher.update(b"|");
        hasher.update(eh.entity_tenant_id.as_bytes());
        hasher.update(b":");
        hasher.update(eh.close_hash.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}
