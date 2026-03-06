use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Eco {
    pub id: Uuid,
    pub tenant_id: String,
    pub eco_number: String,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub created_by: String,
    pub approved_by: Option<String>,
    pub approved_at: Option<DateTime<Utc>>,
    pub applied_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct EcoAuditEntry {
    pub id: i64,
    pub eco_id: Uuid,
    pub tenant_id: String,
    pub action: String,
    pub actor: String,
    pub detail: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct EcoBomRevision {
    pub id: Uuid,
    pub eco_id: Uuid,
    pub tenant_id: String,
    pub bom_id: Uuid,
    pub before_revision_id: Uuid,
    pub after_revision_id: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct EcoDocRevision {
    pub id: Uuid,
    pub eco_id: Uuid,
    pub tenant_id: String,
    pub doc_id: Uuid,
    pub doc_revision_id: Uuid,
    pub created_at: DateTime<Utc>,
}

// Request types

#[derive(Debug, Deserialize)]
pub struct CreateEcoRequest {
    /// If omitted, auto-allocated from the Numbering service.
    #[serde(default)]
    pub eco_number: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub created_by: String,
}

#[derive(Debug, Deserialize)]
pub struct LinkBomRevisionRequest {
    pub bom_id: Uuid,
    pub before_revision_id: Uuid,
    pub after_revision_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct LinkDocRevisionRequest {
    pub doc_id: Uuid,
    pub doc_revision_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct EcoActionRequest {
    pub actor: String,
    pub comment: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ApplyEcoRequest {
    pub actor: String,
    pub effective_from: DateTime<Utc>,
    pub effective_to: Option<DateTime<Utc>>,
}
