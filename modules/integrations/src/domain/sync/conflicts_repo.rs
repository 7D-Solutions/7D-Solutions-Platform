use sqlx::PgPool;
use uuid::Uuid;

use super::conflicts::{
    ConflictClass, ConflictError, ConflictRow, ConflictStatus, CreateConflictRequest,
    ResolveConflictRequest, MAX_VALUE_BYTES,
};

// ── Guards ────────────────────────────────────────────────────────────────────

fn validate_create(req: &CreateConflictRequest) -> Result<(), ConflictError> {
    // creation/edit require both value snapshots
    if req.conflict_class != ConflictClass::Deletion {
        if req.internal_value.is_none() || req.external_value.is_none() {
            return Err(ConflictError::MissingValues);
        }
    }
    // 256 KB cap per value blob
    if let Some(v) = &req.internal_value {
        if v.to_string().len() > MAX_VALUE_BYTES {
            return Err(ConflictError::ValueTooLarge);
        }
    }
    if let Some(v) = &req.external_value {
        if v.to_string().len() > MAX_VALUE_BYTES {
            return Err(ConflictError::ValueTooLarge);
        }
    }
    Ok(())
}

// ── Write operations ──────────────────────────────────────────────────────────

/// Insert a new conflict row. Validates class/value invariants before touching
/// the database so guard failures never produce partial writes.
pub async fn create_conflict(
    pool: &PgPool,
    req: &CreateConflictRequest,
) -> Result<ConflictRow, ConflictError> {
    validate_create(req)?;

    sqlx::query_as::<_, ConflictRow>(
        r#"
        INSERT INTO integrations_sync_conflicts (
            app_id, provider, entity_type, entity_id,
            conflict_class, detected_by,
            internal_value, external_value
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        RETURNING
            id, app_id, provider, entity_type, entity_id,
            conflict_class, status, detected_by, detected_at,
            internal_value, external_value, internal_id,
            resolved_by, resolved_at, resolution_note,
            created_at, updated_at
        "#,
    )
    .bind(&req.app_id)
    .bind(&req.provider)
    .bind(&req.entity_type)
    .bind(&req.entity_id)
    .bind(req.conflict_class.as_str())
    .bind(&req.detected_by)
    .bind(&req.internal_value)
    .bind(&req.external_value)
    .fetch_one(pool)
    .await
    .map_err(ConflictError::Database)
}

/// Transition a pending conflict to `resolved`.  Enforces that internal_id is
/// provided and that the row is currently `pending`.
pub async fn resolve_conflict(
    pool: &PgPool,
    app_id: &str,
    conflict_id: Uuid,
    req: &ResolveConflictRequest,
) -> Result<ConflictRow, ConflictError> {
    let current = get_conflict(pool, app_id, conflict_id)
        .await?
        .ok_or(ConflictError::NotFound(conflict_id))?;

    let current_status = ConflictStatus::from_str(&current.status)
        .unwrap_or(ConflictStatus::Pending);

    if current_status != ConflictStatus::Pending {
        return Err(ConflictError::InvalidTransition(
            current.status.clone(),
            "resolved".to_string(),
        ));
    }

    sqlx::query_as::<_, ConflictRow>(
        r#"
        UPDATE integrations_sync_conflicts
        SET status          = 'resolved',
            internal_id     = $3,
            resolved_by     = $4,
            resolved_at     = NOW(),
            resolution_note = $5,
            updated_at      = NOW()
        WHERE id = $1 AND app_id = $2 AND status = 'pending'
        RETURNING
            id, app_id, provider, entity_type, entity_id,
            conflict_class, status, detected_by, detected_at,
            internal_value, external_value, internal_id,
            resolved_by, resolved_at, resolution_note,
            created_at, updated_at
        "#,
    )
    .bind(conflict_id)
    .bind(app_id)
    .bind(&req.internal_id)
    .bind(&req.resolved_by)
    .bind(&req.resolution_note)
    .fetch_optional(pool)
    .await
    .map_err(ConflictError::Database)?
    .ok_or(ConflictError::NotFound(conflict_id))
}

/// Transition a pending conflict to `ignored` or `unresolvable`.
pub async fn close_conflict(
    pool: &PgPool,
    app_id: &str,
    conflict_id: Uuid,
    new_status: ConflictStatus,
    closed_by: &str,
    note: Option<&str>,
) -> Result<ConflictRow, ConflictError> {
    // Only these two terminal statuses are valid here; resolved goes via resolve_conflict
    if !matches!(new_status, ConflictStatus::Ignored | ConflictStatus::Unresolvable) {
        return Err(ConflictError::InvalidTransition(
            "pending".to_string(),
            new_status.as_str().to_string(),
        ));
    }

    let current = get_conflict(pool, app_id, conflict_id)
        .await?
        .ok_or(ConflictError::NotFound(conflict_id))?;

    let current_status = ConflictStatus::from_str(&current.status)
        .unwrap_or(ConflictStatus::Pending);

    if current_status != ConflictStatus::Pending {
        return Err(ConflictError::InvalidTransition(
            current.status.clone(),
            new_status.as_str().to_string(),
        ));
    }

    sqlx::query_as::<_, ConflictRow>(
        r#"
        UPDATE integrations_sync_conflicts
        SET status          = $3,
            resolved_by     = $4,
            resolved_at     = NOW(),
            resolution_note = $5,
            updated_at      = NOW()
        WHERE id = $1 AND app_id = $2 AND status = 'pending'
        RETURNING
            id, app_id, provider, entity_type, entity_id,
            conflict_class, status, detected_by, detected_at,
            internal_value, external_value, internal_id,
            resolved_by, resolved_at, resolution_note,
            created_at, updated_at
        "#,
    )
    .bind(conflict_id)
    .bind(app_id)
    .bind(new_status.as_str())
    .bind(closed_by)
    .bind(note)
    .fetch_optional(pool)
    .await
    .map_err(ConflictError::Database)?
    .ok_or(ConflictError::NotFound(conflict_id))
}

// ── Read operations ───────────────────────────────────────────────────────────

pub async fn get_conflict(
    pool: &PgPool,
    app_id: &str,
    conflict_id: Uuid,
) -> Result<Option<ConflictRow>, ConflictError> {
    sqlx::query_as::<_, ConflictRow>(
        r#"
        SELECT
            id, app_id, provider, entity_type, entity_id,
            conflict_class, status, detected_by, detected_at,
            internal_value, external_value, internal_id,
            resolved_by, resolved_at, resolution_note,
            created_at, updated_at
        FROM integrations_sync_conflicts
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(conflict_id)
    .bind(app_id)
    .fetch_optional(pool)
    .await
    .map_err(ConflictError::Database)
}

/// List pending conflicts for a given app+provider+entity_type, newest first.
/// Uses the partial index on `status = 'pending'` for fast reads.
pub async fn list_pending(
    pool: &PgPool,
    app_id: &str,
    provider: &str,
    entity_type: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<ConflictRow>, ConflictError> {
    sqlx::query_as::<_, ConflictRow>(
        r#"
        SELECT
            id, app_id, provider, entity_type, entity_id,
            conflict_class, status, detected_by, detected_at,
            internal_value, external_value, internal_id,
            resolved_by, resolved_at, resolution_note,
            created_at, updated_at
        FROM integrations_sync_conflicts
        WHERE app_id = $1 AND provider = $2 AND entity_type = $3
          AND status = 'pending'
        ORDER BY created_at DESC
        LIMIT $4 OFFSET $5
        "#,
    )
    .bind(app_id)
    .bind(provider)
    .bind(entity_type)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .map_err(ConflictError::Database)
}

/// List conflicts with full filter support and total-count pagination.
///
/// Returns `(rows, total_count)`. All filter fields are optional.
pub async fn list_conflicts_paged(
    pool: &PgPool,
    app_id: &str,
    provider: Option<&str>,
    entity_type: Option<&str>,
    status_filter: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<ConflictRow>, i64), ConflictError> {
    const COLS: &str = r#"
        id, app_id, provider, entity_type, entity_id,
        conflict_class, status, detected_by, detected_at,
        internal_value, external_value, internal_id,
        resolved_by, resolved_at, resolution_note,
        created_at, updated_at
    "#;

    let rows = sqlx::query_as::<_, ConflictRow>(&format!(
        r#"
        SELECT {COLS}
        FROM integrations_sync_conflicts
        WHERE app_id = $1
          AND ($2::text IS NULL OR provider    = $2)
          AND ($3::text IS NULL OR entity_type = $3)
          AND ($4::text IS NULL OR status      = $4)
        ORDER BY created_at DESC
        LIMIT $5 OFFSET $6
        "#
    ))
    .bind(app_id)
    .bind(provider)
    .bind(entity_type)
    .bind(status_filter)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .map_err(ConflictError::Database)?;

    let total: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)
        FROM integrations_sync_conflicts
        WHERE app_id = $1
          AND ($2::text IS NULL OR provider    = $2)
          AND ($3::text IS NULL OR entity_type = $3)
          AND ($4::text IS NULL OR status      = $4)
        "#,
    )
    .bind(app_id)
    .bind(provider)
    .bind(entity_type)
    .bind(status_filter)
    .fetch_one(pool)
    .await
    .map_err(ConflictError::Database)?;

    Ok((rows, total.0))
}

/// List all conflicts for an app_id, filtered by optional status, paged.
pub async fn list_conflicts(
    pool: &PgPool,
    app_id: &str,
    status_filter: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<ConflictRow>, ConflictError> {
    sqlx::query_as::<_, ConflictRow>(
        r#"
        SELECT
            id, app_id, provider, entity_type, entity_id,
            conflict_class, status, detected_by, detected_at,
            internal_value, external_value, internal_id,
            resolved_by, resolved_at, resolution_note,
            created_at, updated_at
        FROM integrations_sync_conflicts
        WHERE app_id = $1
          AND ($2::text IS NULL OR status = $2)
        ORDER BY created_at DESC
        LIMIT $3 OFFSET $4
        "#,
    )
    .bind(app_id)
    .bind(status_filter)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .map_err(ConflictError::Database)
}
