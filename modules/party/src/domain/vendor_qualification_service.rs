//! Vendor qualification service — Guard→Mutation→Outbox atomicity.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    build_vendor_qualification_created_envelope, build_vendor_qualification_updated_envelope,
    VendorQualificationPayload, EVENT_TYPE_VENDOR_QUALIFICATION_CREATED,
    EVENT_TYPE_VENDOR_QUALIFICATION_UPDATED,
};
use crate::outbox::enqueue_event_tx;

use super::party::PartyError;
use super::vendor_qualification::{
    CreateVendorQualificationRequest, UpdateVendorQualificationRequest, VendorQualification,
};

// ============================================================================
// Reads
// ============================================================================

/// List vendor qualifications for a party, scoped to app_id.
pub async fn list_vendor_qualifications(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
) -> Result<Vec<VendorQualification>, PartyError> {
    let rows: Vec<VendorQualification> = sqlx::query_as(
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
    .await?;

    Ok(rows)
}

/// Get a single vendor qualification by ID, scoped to app_id.
pub async fn get_vendor_qualification(
    pool: &PgPool,
    app_id: &str,
    qualification_id: Uuid,
) -> Result<Option<VendorQualification>, PartyError> {
    let row: Option<VendorQualification> = sqlx::query_as(
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
    .await?;

    Ok(row)
}

// ============================================================================
// Writes
// ============================================================================

/// Create a vendor qualification. Idempotent via idempotency_key.
/// Emits `party.vendor_qualification.created` via the outbox.
pub async fn create_vendor_qualification(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
    req: &CreateVendorQualificationRequest,
    correlation_id: String,
) -> Result<VendorQualification, PartyError> {
    req.validate()?;

    // Idempotency check
    if let Some(ref idem_key) = req.idempotency_key {
        let existing: Option<VendorQualification> = sqlx::query_as(
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
        .await?;

        if let Some(existing) = existing {
            return Ok(existing);
        }
    }

    let qual_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();

    let mut tx = pool.begin().await?;

    // Guard: party must exist for this app
    guard_party_exists_tx(&mut tx, app_id, party_id).await?;

    // Mutation
    let qual: VendorQualification = sqlx::query_as(
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
    .fetch_one(&mut *tx)
    .await?;

    // Outbox
    let payload = VendorQualificationPayload {
        qualification_id: qual_id,
        party_id,
        app_id: app_id.to_string(),
        qualification_status: qual.qualification_status.clone(),
        certification_ref: qual.certification_ref.clone(),
        expires_at: qual.expires_at,
    };

    let envelope = build_vendor_qualification_created_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_VENDOR_QUALIFICATION_CREATED,
        "vendor_qualification",
        &qual_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(qual)
}

/// Update a vendor qualification. Emits `party.vendor_qualification.updated`.
pub async fn update_vendor_qualification(
    pool: &PgPool,
    app_id: &str,
    qualification_id: Uuid,
    req: &UpdateVendorQualificationRequest,
    correlation_id: String,
) -> Result<VendorQualification, PartyError> {
    req.validate()?;

    let event_id = Uuid::new_v4();
    let now = Utc::now();

    let mut tx = pool.begin().await?;

    // Guard: qualification must exist
    let existing: Option<VendorQualification> = sqlx::query_as(
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
    .fetch_optional(&mut *tx)
    .await?;

    let current = existing.ok_or(PartyError::NotFound(qualification_id))?;

    let new_status = req
        .qualification_status
        .as_deref()
        .map(|s| s.trim().to_string())
        .unwrap_or(current.qualification_status);
    let new_cert_ref = if req.certification_ref.is_some() {
        req.certification_ref.clone()
    } else {
        current.certification_ref
    };
    let new_issued = req.issued_at.or(current.issued_at);
    let new_expires = req.expires_at.or(current.expires_at);
    let new_notes = if req.notes.is_some() {
        req.notes.clone()
    } else {
        current.notes
    };
    let new_metadata = if req.metadata.is_some() {
        req.metadata.clone()
    } else {
        current.metadata
    };

    // Mutation
    let updated: VendorQualification = sqlx::query_as(
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
    .bind(&new_status)
    .bind(&new_cert_ref)
    .bind(new_issued)
    .bind(new_expires)
    .bind(&new_notes)
    .bind(&new_metadata)
    .bind(now)
    .bind(qualification_id)
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await?;

    // Outbox
    let payload = VendorQualificationPayload {
        qualification_id,
        party_id: updated.party_id,
        app_id: app_id.to_string(),
        qualification_status: updated.qualification_status.clone(),
        certification_ref: updated.certification_ref.clone(),
        expires_at: updated.expires_at,
    };

    let envelope = build_vendor_qualification_updated_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_VENDOR_QUALIFICATION_UPDATED,
        "vendor_qualification",
        &qualification_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(updated)
}

// ============================================================================
// Helpers
// ============================================================================

async fn guard_party_exists_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    party_id: Uuid,
) -> Result<(), PartyError> {
    let exists: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM party_parties WHERE id = $1 AND app_id = $2")
            .bind(party_id)
            .bind(app_id)
            .fetch_optional(&mut **tx)
            .await?;

    if exists.is_none() {
        return Err(PartyError::NotFound(party_id));
    }
    Ok(())
}
