//! Vendor qualification service — Guard→Mutation→Outbox atomicity.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::{
    party_repo, vendor_qualification_repo, vendor_qualification_repo::UpdateVendorQualificationData,
};
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
    vendor_qualification_repo::list_vendor_qualifications(pool, app_id, party_id).await
}

/// Get a single vendor qualification by ID, scoped to app_id.
pub async fn get_vendor_qualification(
    pool: &PgPool,
    app_id: &str,
    qualification_id: Uuid,
) -> Result<Option<VendorQualification>, PartyError> {
    vendor_qualification_repo::get_vendor_qualification(pool, app_id, qualification_id).await
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
        if let Some(existing) =
            vendor_qualification_repo::find_vendor_qualification_by_idempotency_key(
                pool, app_id, idem_key,
            )
            .await?
        {
            return Ok(existing);
        }
    }

    let qual_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();

    let mut tx = pool.begin().await?;

    party_repo::guard_party_exists_tx(&mut tx, app_id, party_id).await?;

    let qual = vendor_qualification_repo::insert_vendor_qualification_tx(
        &mut tx, qual_id, party_id, app_id, req, now,
    )
    .await?;

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

    let current = vendor_qualification_repo::fetch_vendor_qualification_for_update_tx(
        &mut tx,
        app_id,
        qualification_id,
    )
    .await?
    .ok_or(PartyError::NotFound(qualification_id))?;

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

    let updated = vendor_qualification_repo::update_vendor_qualification_row_tx(
        &mut tx,
        &UpdateVendorQualificationData {
            qualification_id,
            app_id,
            qualification_status: new_status,
            certification_ref: new_cert_ref,
            issued_at: new_issued,
            expires_at: new_expires,
            notes: new_notes,
            metadata: new_metadata,
            updated_at: now,
        },
    )
    .await?;

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
