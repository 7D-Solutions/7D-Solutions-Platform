pub mod repo;
pub mod service;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct OperationStartVerification {
    pub id: Uuid,
    pub tenant_id: String,
    pub work_order_id: Uuid,
    pub operation_id: Uuid,
    pub status: String,
    pub drawing_verified: bool,
    pub material_verified: bool,
    pub instruction_verified: bool,
    pub operator_id: Uuid,
    pub operator_confirmed_at: Option<DateTime<Utc>>,
    pub verifier_id: Option<Uuid>,
    pub verified_at: Option<DateTime<Utc>>,
    pub skipped_by: Option<Uuid>,
    pub skipped_at: Option<DateTime<Utc>>,
    pub skip_reason: Option<String>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateVerificationRequest {
    pub work_order_id: Uuid,
    pub operation_id: Uuid,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OperatorConfirmRequest {
    pub drawing_verified: bool,
    pub material_verified: bool,
    pub instruction_verified: bool,
}

#[derive(Debug, Deserialize)]
pub struct VerifyRequest {
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SkipVerificationRequest {
    pub skip_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListVerificationsQuery {
    pub status: Option<String>,
    pub work_order_id: Option<Uuid>,
    pub operation_id: Option<Uuid>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}
