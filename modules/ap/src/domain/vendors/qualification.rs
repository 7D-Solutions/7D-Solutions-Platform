//! Vendor qualification service — Guard → Mutation → Outbox writes.
//!
//! change_qualification: transitions qualification_status + emits audit event
//! mark_preferred / unmark_preferred: toggles preferred_vendor flag
//! get_qualification_history: returns vendor_qualification_events ordered by time

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    build_vendor_qualified_envelope, build_vendor_disqualified_envelope,
    build_vendor_qualification_changed_envelope,
    VendorQualifiedPayload, VendorDisqualifiedPayload, VendorQualificationChangedPayload,
    EVENT_TYPE_VENDOR_QUALIFIED, EVENT_TYPE_VENDOR_DISQUALIFIED,
    EVENT_TYPE_VENDOR_QUALIFICATION_CHANGED,
};
use crate::outbox::enqueue_event_tx;

use super::{ChangeQualificationRequest, QualificationStatus, Vendor, VendorError, VendorQualificationEvent};

// ============================================================================
// Reads
// ============================================================================

pub async fn get_qualification_history(
    pool: &PgPool,
    tenant_id: &str,
    vendor_id: Uuid,
) -> Result<Vec<VendorQualificationEvent>, VendorError> {
    let rows = sqlx::query_as::<_, VendorQualificationEvent>(
        r#"
        SELECT id, tenant_id, vendor_id, from_status, to_status, reason, changed_by, changed_at
        FROM vendor_qualification_events
        WHERE vendor_id = $1 AND tenant_id = $2
        ORDER BY changed_at DESC
        "#,
    )
    .bind(vendor_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

// ============================================================================
// Writes
// ============================================================================

/// Change vendor qualification status.
/// Emits ap.vendor_qualified (→ qualified), ap.vendor_disqualified (→ disqualified),
/// or ap.vendor_qualification_changed (all other transitions).
pub async fn change_qualification(
    pool: &PgPool,
    tenant_id: &str,
    vendor_id: Uuid,
    req: &ChangeQualificationRequest,
    correlation_id: String,
) -> Result<Vendor, VendorError> {
    let new_status = req.status.as_str();
    let now = Utc::now();
    let event_id = Uuid::new_v4();
    let audit_id = Uuid::new_v4();

    let mut tx = pool.begin().await?;

    // Lock vendor row and get current status
    let vendor: Option<Vendor> = sqlx::query_as(
        r#"
        SELECT vendor_id, tenant_id, name, tax_id, currency,
               payment_terms_days, payment_method, remittance_email,
               is_active, party_id, created_at, updated_at,
               qualification_status, qualification_notes, qualified_by, qualified_at, preferred_vendor
        FROM vendors
        WHERE vendor_id = $1 AND tenant_id = $2
        FOR UPDATE
        "#,
    )
    .bind(vendor_id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?;

    let vendor = vendor.ok_or(VendorError::NotFound(vendor_id))?;
    let from_status = vendor.qualification_status.clone();

    // Set qualified_by / qualified_at only when transitioning into qualified
    let (new_qualified_by, new_qualified_at) = if req.status == QualificationStatus::Qualified {
        (Some(req.changed_by.clone()), Some(now))
    } else {
        (vendor.qualified_by.clone(), vendor.qualified_at)
    };

    // Mutation: update vendors row
    let updated: Vendor = sqlx::query_as(
        r#"
        UPDATE vendors
        SET qualification_status = $1,
            qualification_notes  = $2,
            qualified_by         = $3,
            qualified_at         = $4,
            updated_at           = $5
        WHERE vendor_id = $6 AND tenant_id = $7
        RETURNING
            vendor_id, tenant_id, name, tax_id, currency,
            payment_terms_days, payment_method, remittance_email,
            is_active, party_id, created_at, updated_at,
            qualification_status, qualification_notes, qualified_by, qualified_at, preferred_vendor
        "#,
    )
    .bind(new_status)
    .bind(&req.notes)
    .bind(&new_qualified_by)
    .bind(new_qualified_at)
    .bind(now)
    .bind(vendor_id)
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await?;

    // Mutation: audit row
    sqlx::query(
        r#"
        INSERT INTO vendor_qualification_events
            (id, tenant_id, vendor_id, from_status, to_status, reason, changed_by, changed_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(audit_id)
    .bind(tenant_id)
    .bind(vendor_id)
    .bind(&from_status)
    .bind(new_status)
    .bind(&req.notes)
    .bind(&req.changed_by)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    // Outbox: pick event type based on new status
    match req.status {
        QualificationStatus::Qualified => {
            let payload = VendorQualifiedPayload {
                vendor_id,
                tenant_id: tenant_id.to_string(),
                from_status: from_status.clone(),
                notes: req.notes.clone(),
                qualified_by: req.changed_by.clone(),
                qualified_at: now,
            };
            let envelope = build_vendor_qualified_envelope(
                event_id, tenant_id.to_string(), correlation_id, None, payload,
            );
            enqueue_event_tx(&mut tx, event_id, EVENT_TYPE_VENDOR_QUALIFIED, "vendor", &vendor_id.to_string(), &envelope).await?;
        }
        QualificationStatus::Disqualified => {
            let payload = VendorDisqualifiedPayload {
                vendor_id,
                tenant_id: tenant_id.to_string(),
                from_status: from_status.clone(),
                reason: req.notes.clone(),
                disqualified_by: req.changed_by.clone(),
                disqualified_at: now,
            };
            let envelope = build_vendor_disqualified_envelope(
                event_id, tenant_id.to_string(), correlation_id, None, payload,
            );
            enqueue_event_tx(&mut tx, event_id, EVENT_TYPE_VENDOR_DISQUALIFIED, "vendor", &vendor_id.to_string(), &envelope).await?;
        }
        _ => {
            let payload = VendorQualificationChangedPayload {
                vendor_id,
                tenant_id: tenant_id.to_string(),
                from_status: from_status.clone(),
                to_status: new_status.to_string(),
                notes: req.notes.clone(),
                changed_by: req.changed_by.clone(),
                changed_at: now,
            };
            let envelope = build_vendor_qualification_changed_envelope(
                event_id, tenant_id.to_string(), correlation_id, None, payload,
            );
            enqueue_event_tx(&mut tx, event_id, EVENT_TYPE_VENDOR_QUALIFICATION_CHANGED, "vendor", &vendor_id.to_string(), &envelope).await?;
        }
    }

    tx.commit().await?;
    Ok(updated)
}

/// Mark a vendor as preferred (sets preferred_vendor = TRUE).
pub async fn mark_preferred(
    pool: &PgPool,
    tenant_id: &str,
    vendor_id: Uuid,
    changed_by: &str,
) -> Result<Vendor, VendorError> {
    let now = Utc::now();

    let updated: Option<Vendor> = sqlx::query_as(
        r#"
        UPDATE vendors
        SET preferred_vendor = TRUE, updated_at = $1
        WHERE vendor_id = $2 AND tenant_id = $3
        RETURNING
            vendor_id, tenant_id, name, tax_id, currency,
            payment_terms_days, payment_method, remittance_email,
            is_active, party_id, created_at, updated_at,
            qualification_status, qualification_notes, qualified_by, qualified_at, preferred_vendor
        "#,
    )
    .bind(now)
    .bind(vendor_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    let vendor = updated.ok_or(VendorError::NotFound(vendor_id))?;

    tracing::info!(
        vendor_id = %vendor_id,
        changed_by = %changed_by,
        "vendor marked as preferred"
    );

    Ok(vendor)
}

/// Unmark a vendor as preferred (sets preferred_vendor = FALSE).
pub async fn unmark_preferred(
    pool: &PgPool,
    tenant_id: &str,
    vendor_id: Uuid,
    changed_by: &str,
) -> Result<Vendor, VendorError> {
    let now = Utc::now();

    let updated: Option<Vendor> = sqlx::query_as(
        r#"
        UPDATE vendors
        SET preferred_vendor = FALSE, updated_at = $1
        WHERE vendor_id = $2 AND tenant_id = $3
        RETURNING
            vendor_id, tenant_id, name, tax_id, currency,
            payment_terms_days, payment_method, remittance_email,
            is_active, party_id, created_at, updated_at,
            qualification_status, qualification_notes, qualified_by, qualified_at, preferred_vendor
        "#,
    )
    .bind(now)
    .bind(vendor_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    let vendor = updated.ok_or(VendorError::NotFound(vendor_id))?;

    tracing::info!(
        vendor_id = %vendor_id,
        changed_by = %changed_by,
        "vendor unmarked as preferred"
    );

    Ok(vendor)
}
