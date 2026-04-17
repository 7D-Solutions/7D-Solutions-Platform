use sqlx::PgPool;
use uuid::Uuid;

use super::models::*;
use super::state_machine;

// ── Complaints ────────────────────────────────────────────────────────────────

pub async fn create_complaint(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    req: &CreateComplaintRequest,
    complaint_number: &str,
) -> Result<Complaint, ComplaintError> {
    if let Some(ref code) = req.category_code {
        ensure_category_active(tx, tenant_id, code).await?;
    }

    let complaint = sqlx::query_as::<_, Complaint>(
        r#"
        INSERT INTO complaints (
            tenant_id, complaint_number, status, party_id, customer_contact_id,
            source, source_ref, severity, category_code, title, description,
            source_entity_type, source_entity_id, assigned_to, due_date, created_by
        ) VALUES (
            $1, $2, 'intake', $3, $4,
            $5, $6, $7, $8, $9, $10,
            $11, $12, $13,
            COALESCE($14, now() + interval '30 days'),
            $15
        )
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(complaint_number)
    .bind(req.party_id)
    .bind(req.customer_contact_id)
    .bind(req.source.as_str())
    .bind(&req.source_ref)
    .bind(req.severity.map(|s| s.as_str()))
    .bind(&req.category_code)
    .bind(&req.title)
    .bind(&req.description)
    .bind(&req.source_entity_type)
    .bind(req.source_entity_id)
    .bind::<Option<String>>(None)
    .bind(req.due_date)
    .bind(&req.created_by)
    .fetch_one(&mut **tx)
    .await?;

    Ok(complaint)
}

pub async fn get_complaint(
    pool: &PgPool,
    tenant_id: &str,
    complaint_id: Uuid,
) -> Result<Option<Complaint>, ComplaintError> {
    let row = sqlx::query_as::<_, Complaint>(
        "SELECT * FROM complaints WHERE id = $1 AND tenant_id = $2",
    )
    .bind(complaint_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn get_complaint_detail(
    pool: &PgPool,
    tenant_id: &str,
    complaint_id: Uuid,
) -> Result<Option<ComplaintDetail>, ComplaintError> {
    let complaint = match get_complaint(pool, tenant_id, complaint_id).await? {
        Some(c) => c,
        None => return Ok(None),
    };
    let activity_log = list_activity_log(pool, tenant_id, complaint_id).await?;
    let resolution = get_resolution(pool, tenant_id, complaint_id).await?;
    Ok(Some(ComplaintDetail { complaint, activity_log, resolution }))
}

pub async fn list_complaints(
    pool: &PgPool,
    tenant_id: &str,
    q: &ListComplaintsQuery,
) -> Result<Vec<Complaint>, ComplaintError> {
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);

    let rows = sqlx::query_as::<_, Complaint>(
        r#"
        SELECT * FROM complaints
        WHERE tenant_id = $1
          AND ($2::TEXT IS NULL OR status = $2)
          AND ($3::TEXT IS NULL OR severity = $3)
          AND ($4::TEXT IS NULL OR category_code = $4)
          AND ($5::UUID IS NULL OR party_id = $5)
          AND ($6::TEXT IS NULL OR assigned_to = $6)
          AND ($7::TEXT IS NULL OR source_entity_type = $7)
          AND ($8::TIMESTAMPTZ IS NULL OR received_at >= $8)
          AND ($9::TIMESTAMPTZ IS NULL OR received_at <= $9)
        ORDER BY received_at DESC
        LIMIT $10 OFFSET $11
        "#,
    )
    .bind(tenant_id)
    .bind(&q.status)
    .bind(&q.severity)
    .bind(&q.category_code)
    .bind(q.party_id)
    .bind(&q.assigned_to)
    .bind(&q.source_entity_type)
    .bind(q.from_date)
    .bind(q.to_date)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

pub async fn update_complaint(
    pool: &PgPool,
    tenant_id: &str,
    complaint_id: Uuid,
    req: &UpdateComplaintRequest,
) -> Result<Complaint, ComplaintError> {
    let complaint = sqlx::query_as::<_, Complaint>(
        r#"
        UPDATE complaints SET
            customer_contact_id = COALESCE($3, customer_contact_id),
            source_ref = COALESCE($4, source_ref),
            title = COALESCE($5, title),
            description = COALESCE($6, description),
            source_entity_type = COALESCE($7, source_entity_type),
            source_entity_id = COALESCE($8, source_entity_id),
            due_date = COALESCE($9, due_date),
            updated_at = now()
        WHERE id = $1 AND tenant_id = $2
        RETURNING *
        "#,
    )
    .bind(complaint_id)
    .bind(tenant_id)
    .bind(req.customer_contact_id)
    .bind(&req.source_ref)
    .bind(&req.title)
    .bind(&req.description)
    .bind(&req.source_entity_type)
    .bind(req.source_entity_id)
    .bind(req.due_date)
    .fetch_optional(pool)
    .await?
    .ok_or(ComplaintError::NotFound(complaint_id))?;

    Ok(complaint)
}

// ── State Transitions ─────────────────────────────────────────────────────────

pub async fn triage_complaint(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    complaint_id: Uuid,
    req: &TriageComplaintRequest,
) -> Result<Complaint, ComplaintError> {
    let current = fetch_for_update(tx, tenant_id, complaint_id).await?;
    let next = state_machine::transition_triage(&current.status)?;

    ensure_category_active(tx, tenant_id, &req.category_code).await?;

    let due_date = req.due_date.unwrap_or_else(|| {
        current.received_at + chrono::Duration::days(30)
    });

    let complaint = sqlx::query_as::<_, Complaint>(
        r#"
        UPDATE complaints SET
            status = $3,
            category_code = $4,
            severity = $5,
            assigned_to = $6,
            assigned_at = now(),
            due_date = $7,
            updated_at = now()
        WHERE id = $1 AND tenant_id = $2
        RETURNING *
        "#,
    )
    .bind(complaint_id)
    .bind(tenant_id)
    .bind(next.as_str())
    .bind(&req.category_code)
    .bind(req.severity.as_str())
    .bind(&req.assigned_to)
    .bind(due_date)
    .fetch_one(&mut **tx)
    .await?;

    Ok(complaint)
}

pub async fn start_investigation(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    complaint_id: Uuid,
    _req: &StartInvestigationRequest,
) -> Result<Complaint, ComplaintError> {
    let current = fetch_for_update(tx, tenant_id, complaint_id).await?;
    let next = state_machine::transition_start_investigation(&current.status)?;

    let complaint = sqlx::query_as::<_, Complaint>(
        r#"
        UPDATE complaints SET status = $3, updated_at = now()
        WHERE id = $1 AND tenant_id = $2
        RETURNING *
        "#,
    )
    .bind(complaint_id)
    .bind(tenant_id)
    .bind(next.as_str())
    .fetch_one(&mut **tx)
    .await?;

    Ok(complaint)
}

pub async fn respond_complaint(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    complaint_id: Uuid,
    _req: &RespondComplaintRequest,
) -> Result<Complaint, ComplaintError> {
    let current = fetch_for_update(tx, tenant_id, complaint_id).await?;
    let next = state_machine::transition_respond(&current.status)?;

    // Guard: requires at least one customer_communication activity entry
    let comm_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM complaint_activity_log
           WHERE complaint_id = $1 AND tenant_id = $2 AND activity_type = 'customer_communication'"#,
    )
    .bind(complaint_id)
    .bind(tenant_id)
    .fetch_one(&mut **tx)
    .await?;

    if comm_count == 0 {
        return Err(ComplaintError::InvalidTransition {
            from: current.status.clone(),
            to: "responded".to_string(),
            reason: "at least one customer_communication activity entry is required before marking as responded".to_string(),
        });
    }

    let complaint = sqlx::query_as::<_, Complaint>(
        r#"
        UPDATE complaints SET
            status = $3,
            responded_at = now(),
            updated_at = now()
        WHERE id = $1 AND tenant_id = $2
        RETURNING *
        "#,
    )
    .bind(complaint_id)
    .bind(tenant_id)
    .bind(next.as_str())
    .fetch_one(&mut **tx)
    .await?;

    Ok(complaint)
}

pub async fn close_complaint(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    complaint_id: Uuid,
    req: &CloseComplaintRequest,
) -> Result<Complaint, ComplaintError> {
    let current = fetch_for_update(tx, tenant_id, complaint_id).await?;
    let next = state_machine::transition_close(&current.status)?;

    // Guard: requires a complaint_resolution record
    let resolution_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM complaint_resolutions WHERE complaint_id = $1 AND tenant_id = $2",
    )
    .bind(complaint_id)
    .bind(tenant_id)
    .fetch_one(&mut **tx)
    .await?;

    if resolution_count == 0 {
        return Err(ComplaintError::InvalidTransition {
            from: current.status.clone(),
            to: "closed".to_string(),
            reason: "a complaint_resolution record is required before closing".to_string(),
        });
    }

    let complaint = sqlx::query_as::<_, Complaint>(
        r#"
        UPDATE complaints SET
            status = $3,
            outcome = $4,
            closed_at = now(),
            updated_at = now()
        WHERE id = $1 AND tenant_id = $2
        RETURNING *
        "#,
    )
    .bind(complaint_id)
    .bind(tenant_id)
    .bind(next.as_str())
    .bind(req.outcome.as_str())
    .fetch_one(&mut **tx)
    .await?;

    Ok(complaint)
}

pub async fn cancel_complaint(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    complaint_id: Uuid,
    req: &CancelComplaintRequest,
) -> Result<Complaint, ComplaintError> {
    let current = fetch_for_update(tx, tenant_id, complaint_id).await?;
    let next = state_machine::transition_cancel(&current.status)?;

    let complaint = sqlx::query_as::<_, Complaint>(
        r#"
        UPDATE complaints SET
            status = $3,
            updated_at = now()
        WHERE id = $1 AND tenant_id = $2
        RETURNING *
        "#,
    )
    .bind(complaint_id)
    .bind(tenant_id)
    .bind(next.as_str())
    .fetch_one(&mut **tx)
    .await?;

    if let Some(reason) = &req.reason {
        let log_req = CreateActivityLogRequest {
            activity_type: ActivityType::Note,
            from_value: Some(current.status.clone()),
            to_value: Some("cancelled".to_string()),
            content: Some(format!("Cancelled: {}", reason)),
            visible_to_customer: None,
            recorded_by: req.cancelled_by.clone(),
        };
        add_activity_log_entry(tx, tenant_id, complaint_id, &log_req).await?;
    }

    Ok(complaint)
}

pub async fn assign_complaint(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    complaint_id: Uuid,
    req: &AssignComplaintRequest,
) -> Result<Complaint, ComplaintError> {
    let current = fetch_for_update(tx, tenant_id, complaint_id).await?;

    if ComplaintStatus::from_str(&current.status)
        .map(|s| s.is_terminal())
        .unwrap_or(false)
    {
        return Err(ComplaintError::Validation(
            "cannot assign a complaint in a terminal state".to_string(),
        ));
    }

    let complaint = sqlx::query_as::<_, Complaint>(
        r#"
        UPDATE complaints SET
            assigned_to = $3,
            assigned_at = now(),
            updated_at = now()
        WHERE id = $1 AND tenant_id = $2
        RETURNING *
        "#,
    )
    .bind(complaint_id)
    .bind(tenant_id)
    .bind(&req.assigned_to)
    .fetch_one(&mut **tx)
    .await?;

    Ok(complaint)
}

// ── Activity Log ──────────────────────────────────────────────────────────────

pub async fn add_activity_log_entry(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    complaint_id: Uuid,
    req: &CreateActivityLogRequest,
) -> Result<ComplaintActivityLog, ComplaintError> {
    let entry = sqlx::query_as::<_, ComplaintActivityLog>(
        r#"
        INSERT INTO complaint_activity_log (
            tenant_id, complaint_id, activity_type, from_value, to_value,
            content, visible_to_customer, recorded_by
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(complaint_id)
    .bind(req.activity_type.as_str())
    .bind(&req.from_value)
    .bind(&req.to_value)
    .bind(&req.content)
    .bind(req.visible_to_customer.unwrap_or(false))
    .bind(&req.recorded_by)
    .fetch_one(&mut **tx)
    .await?;

    Ok(entry)
}

pub async fn list_activity_log(
    pool: &PgPool,
    tenant_id: &str,
    complaint_id: Uuid,
) -> Result<Vec<ComplaintActivityLog>, ComplaintError> {
    let rows = sqlx::query_as::<_, ComplaintActivityLog>(
        "SELECT * FROM complaint_activity_log WHERE complaint_id = $1 AND tenant_id = $2 ORDER BY recorded_at ASC",
    )
    .bind(complaint_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Append-only guard: activity log entries cannot be updated.
pub fn update_activity_log_entry(_id: Uuid) -> Result<(), ComplaintError> {
    Err(ComplaintError::AppendOnly("complaint_activity_log".to_string()))
}

/// Append-only guard: activity log entries cannot be deleted.
pub fn delete_activity_log_entry(_id: Uuid) -> Result<(), ComplaintError> {
    Err(ComplaintError::AppendOnly("complaint_activity_log".to_string()))
}

// ── Resolution ────────────────────────────────────────────────────────────────

pub async fn create_resolution(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    complaint_id: Uuid,
    req: &CreateResolutionRequest,
) -> Result<ComplaintResolution, ComplaintError> {
    // One resolution per complaint — second attempt is a 409
    let existing: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM complaint_resolutions WHERE complaint_id = $1 AND tenant_id = $2",
    )
    .bind(complaint_id)
    .bind(tenant_id)
    .fetch_one(&mut **tx)
    .await?;

    if existing > 0 {
        return Err(ComplaintError::Conflict(
            "a resolution record already exists for this complaint".to_string(),
        ));
    }

    let resolution = sqlx::query_as::<_, ComplaintResolution>(
        r#"
        INSERT INTO complaint_resolutions (
            tenant_id, complaint_id, action_taken, root_cause_summary,
            customer_acceptance, customer_response_at, resolved_by
        ) VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(complaint_id)
    .bind(&req.action_taken)
    .bind(&req.root_cause_summary)
    .bind(req.customer_acceptance.as_str())
    .bind(req.customer_response_at)
    .bind(&req.resolved_by)
    .fetch_one(&mut **tx)
    .await?;

    Ok(resolution)
}

pub async fn get_resolution(
    pool: &PgPool,
    tenant_id: &str,
    complaint_id: Uuid,
) -> Result<Option<ComplaintResolution>, ComplaintError> {
    let row = sqlx::query_as::<_, ComplaintResolution>(
        "SELECT * FROM complaint_resolutions WHERE complaint_id = $1 AND tenant_id = $2 ORDER BY resolved_at DESC LIMIT 1",
    )
    .bind(complaint_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

// ── Category Codes ────────────────────────────────────────────────────────────

pub async fn create_category_code(
    pool: &PgPool,
    tenant_id: &str,
    req: &CreateCategoryCodeRequest,
) -> Result<ComplaintCategoryCode, ComplaintError> {
    let row = sqlx::query_as::<_, ComplaintCategoryCode>(
        r#"
        INSERT INTO complaint_category_codes (tenant_id, category_code, display_label, description, updated_by)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(&req.category_code)
    .bind(&req.display_label)
    .bind(&req.description)
    .bind(&req.created_by)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list_category_codes(
    pool: &PgPool,
    tenant_id: &str,
    include_inactive: bool,
) -> Result<Vec<ComplaintCategoryCode>, ComplaintError> {
    let rows = if include_inactive {
        sqlx::query_as::<_, ComplaintCategoryCode>(
            "SELECT * FROM complaint_category_codes WHERE tenant_id = $1 ORDER BY category_code",
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, ComplaintCategoryCode>(
            "SELECT * FROM complaint_category_codes WHERE tenant_id = $1 AND active = true ORDER BY category_code",
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await?
    };
    Ok(rows)
}

pub async fn update_category_code(
    pool: &PgPool,
    tenant_id: &str,
    category_code: &str,
    req: &UpdateCategoryCodeRequest,
) -> Result<ComplaintCategoryCode, ComplaintError> {
    let row = sqlx::query_as::<_, ComplaintCategoryCode>(
        r#"
        UPDATE complaint_category_codes SET
            display_label = COALESCE($3, display_label),
            description = COALESCE($4, description),
            active = COALESCE($5, active),
            updated_by = $6,
            updated_at = now()
        WHERE tenant_id = $1 AND category_code = $2
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(category_code)
    .bind(&req.display_label)
    .bind(&req.description)
    .bind(req.active)
    .bind(&req.updated_by)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ComplaintError::Validation(format!("category code '{}' not found", category_code)))?;
    Ok(row)
}

// ── Labels ────────────────────────────────────────────────────────────────────

pub async fn upsert_status_label(
    pool: &PgPool,
    tenant_id: &str,
    canonical: &str,
    req: &UpsertLabelRequest,
) -> Result<CcStatusLabel, ComplaintError> {
    let row = sqlx::query_as::<_, CcStatusLabel>(
        r#"
        INSERT INTO cc_status_labels (tenant_id, canonical_status, display_label, description, updated_by)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (tenant_id, canonical_status) DO UPDATE SET
            display_label = EXCLUDED.display_label,
            description = EXCLUDED.description,
            updated_by = EXCLUDED.updated_by,
            updated_at = now()
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(canonical)
    .bind(&req.display_label)
    .bind(&req.description)
    .bind(&req.updated_by)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list_status_labels(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<CcStatusLabel>, ComplaintError> {
    let rows = sqlx::query_as::<_, CcStatusLabel>(
        "SELECT * FROM cc_status_labels WHERE tenant_id = $1 ORDER BY canonical_status",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn upsert_severity_label(
    pool: &PgPool,
    tenant_id: &str,
    canonical: &str,
    req: &UpsertLabelRequest,
) -> Result<CcSeverityLabel, ComplaintError> {
    let row = sqlx::query_as::<_, CcSeverityLabel>(
        r#"
        INSERT INTO cc_severity_labels (tenant_id, canonical_severity, display_label, description, updated_by)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (tenant_id, canonical_severity) DO UPDATE SET
            display_label = EXCLUDED.display_label,
            description = EXCLUDED.description,
            updated_by = EXCLUDED.updated_by,
            updated_at = now()
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(canonical)
    .bind(&req.display_label)
    .bind(&req.description)
    .bind(&req.updated_by)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list_severity_labels(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<CcSeverityLabel>, ComplaintError> {
    let rows = sqlx::query_as::<_, CcSeverityLabel>(
        "SELECT * FROM cc_severity_labels WHERE tenant_id = $1 ORDER BY canonical_severity",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn upsert_source_label(
    pool: &PgPool,
    tenant_id: &str,
    canonical: &str,
    req: &UpsertLabelRequest,
) -> Result<CcSourceLabel, ComplaintError> {
    let row = sqlx::query_as::<_, CcSourceLabel>(
        r#"
        INSERT INTO cc_source_labels (tenant_id, canonical_source, display_label, description, updated_by)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (tenant_id, canonical_source) DO UPDATE SET
            display_label = EXCLUDED.display_label,
            description = EXCLUDED.description,
            updated_by = EXCLUDED.updated_by,
            updated_at = now()
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(canonical)
    .bind(&req.display_label)
    .bind(&req.description)
    .bind(&req.updated_by)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list_source_labels(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<CcSourceLabel>, ComplaintError> {
    let rows = sqlx::query_as::<_, CcSourceLabel>(
        "SELECT * FROM cc_source_labels WHERE tenant_id = $1 ORDER BY canonical_source",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

// ── Numbering ─────────────────────────────────────────────────────────────────

pub async fn next_complaint_number(
    conn: &mut sqlx::PgConnection,
    tenant_id: &str,
) -> Result<String, ComplaintError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM complaints WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(format!("CC-{:05}", count + 1))
}

// ── Internal helpers ──────────────────────────────────────────────────────────

async fn fetch_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    complaint_id: Uuid,
) -> Result<Complaint, ComplaintError> {
    sqlx::query_as::<_, Complaint>(
        "SELECT * FROM complaints WHERE id = $1 AND tenant_id = $2 FOR UPDATE",
    )
    .bind(complaint_id)
    .bind(tenant_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(ComplaintError::NotFound(complaint_id))
}

async fn ensure_category_active(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    category_code: &str,
) -> Result<(), ComplaintError> {
    let active: Option<bool> = sqlx::query_scalar(
        "SELECT active FROM complaint_category_codes WHERE tenant_id = $1 AND category_code = $2",
    )
    .bind(tenant_id)
    .bind(category_code)
    .fetch_optional(&mut **tx)
    .await?;

    match active {
        Some(true) => Ok(()),
        Some(false) => Err(ComplaintError::Validation(format!(
            "category code '{}' is inactive",
            category_code
        ))),
        None => Err(ComplaintError::Validation(format!(
            "category code '{}' not found for this tenant",
            category_code
        ))),
    }
}
