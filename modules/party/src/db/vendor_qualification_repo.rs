//! Vendor qualification repository — all SQL for `party_vendor_qualifications`.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::domain::party::PartyError;
use crate::domain::vendor_qualification::{
    CreateVendorQualificationRequest, UpdateVendorQualificationRequest, VendorQualification,
};

// ── Reads ─────────────────────────────────────────────────────────────────────

pub async fn list_vendor_qualifications(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
) -> Result<Vec<VendorQualification>, PartyError> {
    Ok(sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, qualification_status, certification_ref,
               issued_at, expires_at, notes, idempotency_key, metadata,
               created_at, updated_at
        FROM party_vendor_qualifications
        WHERE party_id = $1 AND app_id = $2
        ORDER BY created_at DESC
        "#,
    )
    .bind(party_id)
    .bind(app_id)
    .fetch_all(pool)
    .await?)
}

pub async fn get_vendor_qualification(
    pool: &PgPool,
    app_id: &str,
    qualification_id: Uuid,
) -> Result<Option<VendorQualification>, PartyError> {
    Ok(sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, qualification_status, certification_ref,
               issued_at, expires_at, notes, idempotency_key, metadata,
               created_at, updated_at
        FROM party_vendor_qualifications
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(qualification_id)
    .bind(app_id)
    .fetch_optional(pool)
    .await?)
}

pub async fn find_vendor_qualification_by_idempotency_key(
    pool: &PgPool,
    app_id: &str,
    idem_key: &str,
) -> Result<Option<VendorQualification>, PartyError> {
    Ok(sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, qualification_status, certification_ref,
               issued_at, expires_at, notes, idempotency_key, metadata,
               created_at, updated_at
        FROM party_vendor_qualifications
        WHERE app_id = $1 AND idempotency_key = $2
        "#,
    )
    .bind(app_id)
    .bind(idem_key)
    .fetch_optional(pool)
    .await?)
}

// ── Transaction helpers ───────────────────────────────────────────────────────

pub async fn insert_vendor_qualification_tx(
    tx: &mut Transaction<'_, Postgres>,
    qual_id: Uuid,
    party_id: Uuid,
    app_id: &str,
    req: &CreateVendorQualificationRequest,
    now: DateTime<Utc>,
) -> Result<VendorQualification, PartyError> {
    Ok(sqlx::query_as(
        r#"
        INSERT INTO party_vendor_qualifications (
            id, party_id, app_id, qualification_status, certification_ref,
            issued_at, expires_at, notes, idempotency_key, metadata,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11)
        RETURNING id, party_id, app_id, qualification_status, certification_ref,
                  issued_at, expires_at, notes, idempotency_key, metadata,
                  created_at, updated_at
        "#,
    )
    .bind(qual_id)
    .bind(party_id)
    .bind(app_id)
    .bind(req.qualification_status.trim())
    .bind(&req.certification_ref)
    .bind(req.issued_at)
    .bind(req.expires_at)
    .bind(&req.notes)
    .bind(&req.idempotency_key)
    .bind(&req.metadata)
    .bind(now)
    .fetch_one(&mut **tx)
    .await?)
}

pub async fn fetch_vendor_qualification_for_update_tx(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    qualification_id: Uuid,
) -> Result<Option<VendorQualification>, PartyError> {
    Ok(sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, qualification_status, certification_ref,
               issued_at, expires_at, notes, idempotency_key, metadata,
               created_at, updated_at
        FROM party_vendor_qualifications
        WHERE id = $1 AND app_id = $2
        FOR UPDATE
        "#,
    )
    .bind(qualification_id)
    .bind(app_id)
    .fetch_optional(&mut **tx)
    .await?)
}

pub struct UpdateVendorQualificationData<'a> {
    pub qualification_id: Uuid,
    pub app_id: &'a str,
    pub qualification_status: String,
    pub certification_ref: Option<String>,
    pub issued_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub updated_at: DateTime<Utc>,
}

pub async fn update_vendor_qualification_row_tx(
    tx: &mut Transaction<'_, Postgres>,
    p: &UpdateVendorQualificationData<'_>,
) -> Result<VendorQualification, PartyError> {
    Ok(sqlx::query_as(
        r#"
        UPDATE party_vendor_qualifications
        SET qualification_status = $1, certification_ref = $2, issued_at = $3,
            expires_at = $4, notes = $5, metadata = $6, updated_at = $7
        WHERE id = $8 AND app_id = $9
        RETURNING id, party_id, app_id, qualification_status, certification_ref,
                  issued_at, expires_at, notes, idempotency_key, metadata,
                  created_at, updated_at
        "#,
    )
    .bind(&p.qualification_status)
    .bind(&p.certification_ref)
    .bind(p.issued_at)
    .bind(p.expires_at)
    .bind(&p.notes)
    .bind(&p.metadata)
    .bind(p.updated_at)
    .bind(p.qualification_id)
    .bind(p.app_id)
    .fetch_one(&mut **tx)
    .await?)
}
