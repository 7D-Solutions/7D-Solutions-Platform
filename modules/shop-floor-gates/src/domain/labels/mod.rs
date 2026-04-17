pub mod repo;
pub mod service;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct StatusLabel {
    pub id: Uuid,
    pub tenant_id: String,
    pub status_key: String,
    pub display_name: String,
    pub color_hex: Option<String>,
    pub sort_order: i32,
}

#[derive(Debug, Deserialize)]
pub struct UpsertLabelRequest {
    pub status_key: String,
    pub display_name: String,
    pub color_hex: Option<String>,
    pub sort_order: Option<i32>,
}

// Known label tables in this module
pub const LABEL_TABLES: &[&str] = &[
    "hold_type_labels",
    "hold_scope_labels",
    "hold_release_authority_labels",
    "hold_status_labels",
    "handoff_initiation_labels",
    "handoff_status_labels",
    "verification_status_labels",
    "signoff_role_labels",
];
