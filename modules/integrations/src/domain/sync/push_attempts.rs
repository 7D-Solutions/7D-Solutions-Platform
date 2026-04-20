use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use super::conflicts::{ConflictClass, ConflictRow, MAX_VALUE_BYTES};

// ── List filter ───────────────────────────────────────────────────────────────

/// Structured filter for `list_attempts`. All fields optional.
#[derive(Debug, Default)]
pub struct ListAttemptsFilter<'a> {
    pub provider: Option<&'a str>,
    pub entity_type: Option<&'a str>,
    pub status: Option<&'a str>,
    pub request_fingerprint: Option<&'a str>,
    pub started_after: Option<DateTime<Utc>>,
    pub started_before: Option<DateTime<Utc>>,
}

// ── Domain model ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum PushStatus {
    Accepted,
    Inflight,
    Succeeded,
    Failed,
    UnknownFailure,
    /// Pre-call: authority version advanced before dispatch; no write was sent.
    Superseded,
    /// Post-call: write completed but authority changed while inflight;
    /// reconciliation has been run and either auto-closed or opened a conflict.
    CompletedUnderStaleAuthority,
}

impl PushStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PushStatus::Accepted => "accepted",
            PushStatus::Inflight => "inflight",
            PushStatus::Succeeded => "succeeded",
            PushStatus::Failed => "failed",
            PushStatus::UnknownFailure => "unknown_failure",
            PushStatus::Superseded => "superseded",
            PushStatus::CompletedUnderStaleAuthority => "completed_under_stale_authority",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "accepted" => Some(PushStatus::Accepted),
            "inflight" => Some(PushStatus::Inflight),
            "succeeded" => Some(PushStatus::Succeeded),
            "failed" => Some(PushStatus::Failed),
            "unknown_failure" => Some(PushStatus::UnknownFailure),
            "superseded" => Some(PushStatus::Superseded),
            "completed_under_stale_authority" => Some(PushStatus::CompletedUnderStaleAuthority),
            _ => None,
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
pub struct PushAttemptRow {
    pub id: Uuid,
    pub app_id: String,
    pub provider: String,
    pub entity_type: String,
    pub entity_id: String,
    pub operation: String,
    pub authority_version: i64,
    pub request_fingerprint: String,
    pub status: String,
    pub error_message: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ── Repository ────────────────────────────────────────────────────────────────

const SELECT_COLS: &str = r#"
    id, app_id, provider, entity_type, entity_id, operation,
    authority_version, request_fingerprint, status, error_message,
    started_at, completed_at, created_at, updated_at
"#;

/// Record a new push intent in 'accepted' state.
pub async fn insert_attempt(
    pool: &PgPool,
    app_id: &str,
    provider: &str,
    entity_type: &str,
    entity_id: &str,
    operation: &str,
    authority_version: i64,
    request_fingerprint: &str,
) -> Result<PushAttemptRow, sqlx::Error> {
    sqlx::query_as::<_, PushAttemptRow>(&format!(
        r#"
        INSERT INTO integrations_sync_push_attempts
            (app_id, provider, entity_type, entity_id, operation,
             authority_version, request_fingerprint)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING {SELECT_COLS}
        "#
    ))
    .bind(app_id)
    .bind(provider)
    .bind(entity_type)
    .bind(entity_id)
    .bind(operation)
    .bind(authority_version)
    .bind(request_fingerprint)
    .fetch_one(pool)
    .await
}

/// Transition an 'accepted' attempt to 'inflight'. No-ops if not in 'accepted'.
pub async fn transition_to_inflight(
    pool: &PgPool,
    attempt_id: Uuid,
) -> Result<Option<PushAttemptRow>, sqlx::Error> {
    sqlx::query_as::<_, PushAttemptRow>(&format!(
        r#"
        UPDATE integrations_sync_push_attempts
        SET status = 'inflight', updated_at = NOW()
        WHERE id = $1 AND status = 'accepted'
        RETURNING {SELECT_COLS}
        "#
    ))
    .bind(attempt_id)
    .fetch_optional(pool)
    .await
}

/// Transition an 'inflight' attempt to a terminal status.
/// `new_status` must be one of: 'succeeded', 'failed', 'unknown_failure'.
pub async fn complete_attempt(
    pool: &PgPool,
    attempt_id: Uuid,
    new_status: &str,
    error_message: Option<&str>,
) -> Result<Option<PushAttemptRow>, sqlx::Error> {
    sqlx::query_as::<_, PushAttemptRow>(&format!(
        r#"
        UPDATE integrations_sync_push_attempts
        SET status        = $2,
            error_message = $3,
            completed_at  = NOW(),
            updated_at    = NOW()
        WHERE id = $1 AND status = 'inflight'
        RETURNING {SELECT_COLS}
        "#
    ))
    .bind(attempt_id)
    .bind(new_status)
    .bind(error_message)
    .fetch_optional(pool)
    .await
}

/// Fetch a single attempt by ID.
pub async fn get_attempt(
    pool: &PgPool,
    attempt_id: Uuid,
) -> Result<Option<PushAttemptRow>, sqlx::Error> {
    sqlx::query_as::<_, PushAttemptRow>(&format!(
        r#"
        SELECT {SELECT_COLS}
        FROM integrations_sync_push_attempts
        WHERE id = $1
        "#
    ))
    .bind(attempt_id)
    .fetch_optional(pool)
    .await
}

/// Return inflight attempts whose `started_at` is older than `stale_threshold`.
/// Used by the watchdog worker to detect orphaned pushes.
pub async fn list_stale_inflight(
    pool: &PgPool,
    stale_threshold: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<PushAttemptRow>, sqlx::Error> {
    sqlx::query_as::<_, PushAttemptRow>(&format!(
        r#"
        SELECT {SELECT_COLS}
        FROM integrations_sync_push_attempts
        WHERE status = 'inflight' AND started_at < $1
        ORDER BY started_at ASC
        LIMIT $2
        "#
    ))
    .bind(stale_threshold)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Transition all inflight rows older than `stale_threshold` to `failed`
/// with error_message = 'inflight_timeout'. Returns the number of rows transitioned.
pub async fn timeout_stale_inflight(
    pool: &PgPool,
    stale_threshold: DateTime<Utc>,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE integrations_sync_push_attempts
        SET status        = 'failed',
            error_message = 'inflight_timeout',
            completed_at  = NOW(),
            updated_at    = NOW()
        WHERE status = 'inflight' AND started_at < $1
        "#,
    )
    .bind(stale_threshold)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

/// List attempts for a tenant with optional structured filters and pagination.
///
/// Returns `(rows, total_count)`. Filters are ANDed. All filter fields are
/// optional; omitted fields match all rows for that column.
pub async fn list_attempts(
    pool: &PgPool,
    app_id: &str,
    filter: &ListAttemptsFilter<'_>,
    page: i64,
    page_size: i64,
) -> Result<(Vec<PushAttemptRow>, i64), sqlx::Error> {
    let offset = (page - 1).max(0) * page_size;

    // Build WHERE clause dynamically by tracking bind parameter index.
    // $1 is always app_id; additional predicates append $2, $3, ...
    let mut predicates = vec!["app_id = $1".to_string()];
    let mut idx: usize = 2;

    if filter.provider.is_some() {
        predicates.push(format!("provider = ${idx}"));
        idx += 1;
    }
    if filter.entity_type.is_some() {
        predicates.push(format!("entity_type = ${idx}"));
        idx += 1;
    }
    if filter.status.is_some() {
        predicates.push(format!("status = ${idx}"));
        idx += 1;
    }
    if filter.request_fingerprint.is_some() {
        predicates.push(format!("request_fingerprint = ${idx}"));
        idx += 1;
    }
    if filter.started_after.is_some() {
        predicates.push(format!("started_at >= ${idx}"));
        idx += 1;
    }
    if filter.started_before.is_some() {
        predicates.push(format!("started_at <= ${idx}"));
        idx += 1;
    }

    let where_clause = predicates.join(" AND ");
    let limit_idx = idx;
    let offset_idx = idx + 1;

    let data_sql = format!(
        "SELECT {SELECT_COLS} FROM integrations_sync_push_attempts \
         WHERE {where_clause} ORDER BY started_at DESC LIMIT ${limit_idx} OFFSET ${offset_idx}"
    );
    let count_sql = format!(
        "SELECT COUNT(*) FROM integrations_sync_push_attempts WHERE {where_clause}"
    );

    macro_rules! bind_filters {
        ($q:expr) => {{
            let mut q = $q.bind(app_id);
            if let Some(v) = filter.provider { q = q.bind(v); }
            if let Some(v) = filter.entity_type { q = q.bind(v); }
            if let Some(v) = filter.status { q = q.bind(v); }
            if let Some(v) = filter.request_fingerprint { q = q.bind(v); }
            if let Some(v) = filter.started_after { q = q.bind(v); }
            if let Some(v) = filter.started_before { q = q.bind(v); }
            q
        }};
    }

    let rows = bind_filters!(sqlx::query_as::<_, PushAttemptRow>(&data_sql))
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool)
        .await?;

    let total: (i64,) = bind_filters!(sqlx::query_as::<_, (i64,)>(&count_sql))
        .fetch_one(pool)
        .await?;

    Ok((rows, total.0))
}

// ── Pre-call authority version check ─────────────────────────────────────────

/// Outcome of `pre_call_version_check`.
#[derive(Debug)]
pub enum PreCallOutcome {
    /// Authority advanced since the attempt was accepted; the attempt is now
    /// `superseded` and no external call should be dispatched.
    Superseded(PushAttemptRow),
    /// Versions match; caller should proceed to `transition_to_inflight`.
    ReadyForInflight,
}

/// Check whether authority changed since the attempt was accepted.
///
/// If `current_authority_version` differs from `attempt.authority_version`,
/// the attempt is transitioned to `superseded` atomically.  The external call
/// must NOT be dispatched in that case.  Returns `ReadyForInflight` when
/// versions match and the caller should proceed to `transition_to_inflight`.
pub async fn pre_call_version_check(
    pool: &PgPool,
    attempt_id: Uuid,
    current_authority_version: i64,
) -> Result<PreCallOutcome, sqlx::Error> {
    let row = match get_attempt(pool, attempt_id).await? {
        Some(r) => r,
        None => return Ok(PreCallOutcome::ReadyForInflight),
    };

    if row.authority_version == current_authority_version {
        return Ok(PreCallOutcome::ReadyForInflight);
    }

    // Authority has advanced: supersede the attempt so dispatch is prevented.
    let superseded = sqlx::query_as::<_, PushAttemptRow>(&format!(
        r#"
        UPDATE integrations_sync_push_attempts
        SET status       = 'superseded',
            completed_at = NOW(),
            updated_at   = NOW()
        WHERE id = $1 AND status = 'accepted'
        RETURNING {SELECT_COLS}
        "#
    ))
    .bind(attempt_id)
    .fetch_optional(pool)
    .await?;

    match superseded {
        Some(r) => Ok(PreCallOutcome::Superseded(r)),
        // Attempt already left 'accepted' (concurrent transition); caller re-checks state.
        None => Ok(PreCallOutcome::ReadyForInflight),
    }
}

// ── Post-call stale-authority reconciliation ──────────────────────────────────

/// Outcome of `post_call_reconcile`.
#[derive(Debug)]
pub enum ReconcileOutcome {
    /// Values were JSON-equal; no conflict row was created.
    AutoClosed,
    /// Values differed; a conflict row was created and is returned.
    ConflictOpened(ConflictRow),
}

#[derive(Debug, thiserror::Error)]
pub enum ReconcileError {
    #[error("conflict value blob exceeds 256 KB limit")]
    ValueTooLarge,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// Atomically record that a push completed under stale authority and reconcile.
///
/// 1. Transitions the attempt from `inflight` → `completed_under_stale_authority`.
/// 2. Compares `internal_value` (current platform state) with `external_value`
///    (current provider state) using JSON equality.
/// 3. Equivalent → returns `AutoClosed`; no conflict row is created.
/// 4. Divergent → inserts a conflict row inside the same transaction and
///    returns `ConflictOpened`.
///
/// Steps 1 and 4 are committed atomically: a divergent reconciliation never
/// leaves the attempt `inflight` or a conflict row orphaned.
pub async fn post_call_reconcile(
    pool: &PgPool,
    attempt_id: Uuid,
    app_id: &str,
    provider: &str,
    entity_type: &str,
    entity_id: &str,
    internal_value: Option<Value>,
    external_value: Option<Value>,
) -> Result<ReconcileOutcome, ReconcileError> {
    // Guard: validate blob sizes before touching the database.
    if let Some(v) = &internal_value {
        if v.to_string().len() > MAX_VALUE_BYTES {
            return Err(ReconcileError::ValueTooLarge);
        }
    }
    if let Some(v) = &external_value {
        if v.to_string().len() > MAX_VALUE_BYTES {
            return Err(ReconcileError::ValueTooLarge);
        }
    }

    let mut tx = pool.begin().await?;

    // 1. Transition inflight → completed_under_stale_authority.
    sqlx::query(&format!(
        r#"
        UPDATE integrations_sync_push_attempts
        SET status       = 'completed_under_stale_authority',
            completed_at = NOW(),
            updated_at   = NOW()
        WHERE id = $1 AND status = 'inflight'
        "#
    ))
    .bind(attempt_id)
    .execute(&mut *tx)
    .await?;

    // 2. Compare values.
    if internal_value == external_value {
        tx.commit().await?;
        return Ok(ReconcileOutcome::AutoClosed);
    }

    // 3. Divergent: determine conflict class and insert conflict row atomically.
    let conflict_class = match (&internal_value, &external_value) {
        (Some(_), Some(_)) => ConflictClass::Edit,
        _ => ConflictClass::Deletion,
    };

    let conflict = sqlx::query_as::<_, ConflictRow>(
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
    .bind(app_id)
    .bind(provider)
    .bind(entity_type)
    .bind(entity_id)
    .bind(conflict_class.as_str())
    .bind(format!("push_attempt:{attempt_id}"))
    .bind(&internal_value)
    .bind(&external_value)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(ReconcileOutcome::ConflictOpened(conflict))
}

// ── Watchdog ──────────────────────────────────────────────────────────────────

const WATCHDOG_INTERVAL_SECS: u64 = 60;
const INFLIGHT_TIMEOUT_MINUTES: i64 = 10;

/// Background task that periodically times out stale inflight push attempts.
/// Runs every 60 seconds; transitions rows with `started_at < NOW() - 10min` to `failed`.
pub async fn run_watchdog_task(pool: PgPool) {
    tracing::info!(
        interval_secs = WATCHDOG_INTERVAL_SECS,
        timeout_minutes = INFLIGHT_TIMEOUT_MINUTES,
        "Integrations: push-attempt watchdog started"
    );

    let mut interval = tokio::time::interval(Duration::from_secs(WATCHDOG_INTERVAL_SECS));
    loop {
        interval.tick().await;

        let threshold =
            Utc::now() - chrono::Duration::minutes(INFLIGHT_TIMEOUT_MINUTES);
        match timeout_stale_inflight(&pool, threshold).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(
                count = n,
                "Integrations: watchdog timed out stale inflight push attempts"
            ),
            Err(e) => tracing::error!(
                error = %e,
                "Integrations: push-attempt watchdog error"
            ),
        }
    }
}
