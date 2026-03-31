use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// The type of competence artifact in the registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ArtifactType {
    Certification,
    Training,
    Qualification,
}

impl std::fmt::Display for ArtifactType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArtifactType::Certification => write!(f, "certification"),
            ArtifactType::Training => write!(f, "training"),
            ArtifactType::Qualification => write!(f, "qualification"),
        }
    }
}

impl std::str::FromStr for ArtifactType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "certification" => Ok(ArtifactType::Certification),
            "training" => Ok(ArtifactType::Training),
            "qualification" => Ok(ArtifactType::Qualification),
            _ => Err(format!("unknown artifact_type: '{s}'")),
        }
    }
}

/// A registered competence artifact (the "what").
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompetenceArtifact {
    pub id: Uuid,
    pub tenant_id: String,
    pub artifact_type: ArtifactType,
    pub name: String,
    pub code: String,
    pub description: Option<String>,
    /// How many days this competence remains valid after award. None = never expires.
    pub valid_duration_days: Option<i32>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request to register a new competence artifact.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RegisterArtifactRequest {
    pub tenant_id: String,
    pub artifact_type: ArtifactType,
    pub name: String,
    pub code: String,
    pub description: Option<String>,
    pub valid_duration_days: Option<i32>,
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

/// An operator's competence assignment (the "who has what").
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OperatorCompetence {
    pub id: Uuid,
    pub tenant_id: String,
    pub operator_id: Uuid,
    pub artifact_id: Uuid,
    pub awarded_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub evidence_ref: Option<String>,
    pub awarded_by: Option<String>,
    pub is_revoked: bool,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Request to assign a competence to an operator.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AssignCompetenceRequest {
    pub tenant_id: String,
    pub operator_id: Uuid,
    pub artifact_id: Uuid,
    pub awarded_at: DateTime<Utc>,
    /// Explicit expiry. If absent, computed from artifact's valid_duration_days.
    pub expires_at: Option<DateTime<Utc>>,
    pub evidence_ref: Option<String>,
    pub awarded_by: Option<String>,
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

/// Query: is this operator authorized for a given capability at a given time?
#[derive(Debug, Serialize, Deserialize)]
pub struct AuthorizationQuery {
    pub tenant_id: String,
    pub operator_id: Uuid,
    pub artifact_code: String,
    pub at_time: DateTime<Utc>,
}

/// Result of an authorization check.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AuthorizationResult {
    pub authorized: bool,
    pub operator_id: Uuid,
    pub artifact_code: String,
    pub at_time: DateTime<Utc>,
    /// The matching assignment, if authorized.
    pub assignment_id: Option<Uuid>,
    pub expires_at: Option<DateTime<Utc>>,
}
