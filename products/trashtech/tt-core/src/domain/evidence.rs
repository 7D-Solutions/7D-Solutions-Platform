use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EvidenceRecord {
    pub id: Uuid,
    pub pickup_job_id: Uuid,
    pub evidence_type: String,
    pub payload: serde_json::Value,
    pub recorded_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateEvidenceRecord {
    pub pickup_job_id: Uuid,
    pub evidence_type: EvidenceType,
    pub payload: serde_json::Value,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceType {
    RfidScan,
    CameraTimestamp,
    DriverNote,
}

impl EvidenceType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RfidScan => "rfid_scan",
            Self::CameraTimestamp => "camera_timestamp",
            Self::DriverNote => "driver_note",
        }
    }
}

impl std::fmt::Display for EvidenceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

pub struct EvidenceRepo;

impl EvidenceRepo {
    pub async fn create(
        pool: &PgPool,
        input: &CreateEvidenceRecord,
    ) -> Result<EvidenceRecord, sqlx::Error> {
        sqlx::query_as::<_, EvidenceRecord>(
            r#"INSERT INTO evidence_records
                (pickup_job_id, evidence_type, payload, recorded_at)
               VALUES ($1, $2, $3, $4)
               RETURNING *"#,
        )
        .bind(input.pickup_job_id)
        .bind(input.evidence_type.as_str())
        .bind(&input.payload)
        .bind(input.recorded_at)
        .fetch_one(pool)
        .await
    }

    pub async fn get_by_id(
        pool: &PgPool,
        id: Uuid,
    ) -> Result<Option<EvidenceRecord>, sqlx::Error> {
        sqlx::query_as::<_, EvidenceRecord>("SELECT * FROM evidence_records WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn list_for_job(
        pool: &PgPool,
        pickup_job_id: Uuid,
    ) -> Result<Vec<EvidenceRecord>, sqlx::Error> {
        sqlx::query_as::<_, EvidenceRecord>(
            "SELECT * FROM evidence_records WHERE pickup_job_id = $1 ORDER BY recorded_at",
        )
        .bind(pickup_job_id)
        .fetch_all(pool)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_type_as_str() {
        assert_eq!(EvidenceType::RfidScan.as_str(), "rfid_scan");
        assert_eq!(EvidenceType::CameraTimestamp.as_str(), "camera_timestamp");
        assert_eq!(EvidenceType::DriverNote.as_str(), "driver_note");
    }

    #[test]
    fn evidence_type_display() {
        assert_eq!(format!("{}", EvidenceType::RfidScan), "rfid_scan");
    }

    #[test]
    fn create_evidence_serializes() {
        let input = CreateEvidenceRecord {
            pickup_job_id: Uuid::new_v4(),
            evidence_type: EvidenceType::RfidScan,
            payload: serde_json::json!({"tag_id": "ABC123"}),
            recorded_at: Utc::now(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("rfid_scan"));
        assert!(json.contains("tag_id"));
    }

    #[test]
    fn evidence_type_serde_roundtrip() {
        let original = EvidenceType::CameraTimestamp;
        let json = serde_json::to_string(&original).unwrap();
        let parsed: EvidenceType = serde_json::from_str(&json).unwrap();
        assert_eq!(original, parsed);
    }
}
