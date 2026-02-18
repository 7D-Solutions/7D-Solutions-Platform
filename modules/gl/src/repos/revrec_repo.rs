//! Repository for revrec contract persistence
//!
//! All writes are atomic with the outbox (transactional outbox pattern).
//! Contract creation persists contract + obligations + outbox event in a single transaction.

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use crate::repos::outbox_repo;
use crate::revrec::{
    ContractCreatedPayload, ContractModifiedPayload, ScheduleCreatedPayload,
    EVENT_TYPE_CONTRACT_CREATED, EVENT_TYPE_CONTRACT_MODIFIED, EVENT_TYPE_SCHEDULE_CREATED,
    MUTATION_CLASS_DATA_MUTATION,
};

/// Errors from revrec repository operations
#[derive(Debug, thiserror::Error)]
pub enum RevrecRepoError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Contract already exists: {0}")]
    DuplicateContract(Uuid),

    #[error("Allocation sum mismatch: obligations sum to {sum}, expected {expected}")]
    AllocationMismatch { sum: i64, expected: i64 },

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Schedule already exists: {0}")]
    DuplicateSchedule(Uuid),

    #[error("Obligation not found: {0}")]
    ObligationNotFound(Uuid),

    #[error("Schedule lines sum {sum} does not match total {expected}")]
    ScheduleSumMismatch { sum: i64, expected: i64 },

    #[error("Modification already exists: {0}")]
    DuplicateModification(Uuid),

    #[error("Contract not found: {0}")]
    ContractNotFound(Uuid),
}

/// Check if a contract already exists (idempotency check)
pub async fn contract_exists(pool: &PgPool, contract_id: Uuid) -> Result<bool, RevrecRepoError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM revrec_contracts WHERE contract_id = $1)",
    )
    .bind(contract_id)
    .fetch_one(pool)
    .await?;
    Ok(exists)
}

/// Create a revenue contract with obligations, atomically with outbox event.
///
/// This function:
/// 1. Validates allocation sum invariant
/// 2. Inserts contract row
/// 3. Inserts all obligation rows
/// 4. Inserts revrec.contract_created into outbox
/// 5. Commits atomically
///
/// Idempotency: if contract_id already exists, returns DuplicateContract error.
pub async fn create_contract(
    pool: &PgPool,
    event_id: Uuid,
    payload: &ContractCreatedPayload,
) -> Result<Uuid, RevrecRepoError> {
    // Validate allocation invariant before starting transaction
    let allocation_sum: i64 = payload
        .performance_obligations
        .iter()
        .map(|o| o.allocated_amount_minor)
        .sum();
    if allocation_sum != payload.total_transaction_price_minor {
        return Err(RevrecRepoError::AllocationMismatch {
            sum: allocation_sum,
            expected: payload.total_transaction_price_minor,
        });
    }

    let mut tx = pool.begin().await?;

    // Idempotency: check if contract already exists within transaction (serializable)
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM revrec_contracts WHERE contract_id = $1)",
    )
    .bind(payload.contract_id)
    .fetch_one(&mut *tx)
    .await?;

    if exists {
        tx.rollback().await?;
        return Err(RevrecRepoError::DuplicateContract(payload.contract_id));
    }

    // Parse dates
    let contract_start = NaiveDate::parse_from_str(&payload.contract_start, "%Y-%m-%d")
        .map_err(|e| RevrecRepoError::Serialization(format!("Invalid contract_start: {}", e)))?;
    let contract_end = payload
        .contract_end
        .as_ref()
        .map(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d"))
        .transpose()
        .map_err(|e| RevrecRepoError::Serialization(format!("Invalid contract_end: {}", e)))?;

    // Insert contract
    sqlx::query(
        r#"
        INSERT INTO revrec_contracts (
            contract_id, tenant_id, customer_id, contract_name,
            contract_start, contract_end, total_transaction_price_minor,
            currency, external_contract_ref, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10)
        "#,
    )
    .bind(payload.contract_id)
    .bind(&payload.tenant_id)
    .bind(&payload.customer_id)
    .bind(&payload.contract_name)
    .bind(contract_start)
    .bind(contract_end)
    .bind(payload.total_transaction_price_minor)
    .bind(&payload.currency)
    .bind(&payload.external_contract_ref)
    .bind(payload.created_at)
    .execute(&mut *tx)
    .await?;

    // Insert obligations
    for obligation in &payload.performance_obligations {
        let satisfaction_start =
            NaiveDate::parse_from_str(&obligation.satisfaction_start, "%Y-%m-%d").map_err(|e| {
                RevrecRepoError::Serialization(format!("Invalid satisfaction_start: {}", e))
            })?;
        let satisfaction_end = obligation
            .satisfaction_end
            .as_ref()
            .map(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d"))
            .transpose()
            .map_err(|e| {
                RevrecRepoError::Serialization(format!("Invalid satisfaction_end: {}", e))
            })?;

        let pattern_json = serde_json::to_value(&obligation.recognition_pattern)
            .map_err(|e| RevrecRepoError::Serialization(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO revrec_obligations (
                obligation_id, contract_id, tenant_id, name, description,
                allocated_amount_minor, recognition_pattern,
                satisfaction_start, satisfaction_end, created_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NOW())
            "#,
        )
        .bind(obligation.obligation_id)
        .bind(payload.contract_id)
        .bind(&payload.tenant_id)
        .bind(&obligation.name)
        .bind(&obligation.description)
        .bind(obligation.allocated_amount_minor)
        .bind(pattern_json)
        .bind(satisfaction_start)
        .bind(satisfaction_end)
        .execute(&mut *tx)
        .await?;
    }

    // Insert outbox event atomically
    let outbox_payload = serde_json::to_value(payload)
        .map_err(|e| RevrecRepoError::Serialization(e.to_string()))?;

    outbox_repo::insert_outbox_event(
        &mut tx,
        event_id,
        EVENT_TYPE_CONTRACT_CREATED,
        "revrec_contract",
        &payload.contract_id.to_string(),
        outbox_payload,
        MUTATION_CLASS_DATA_MUTATION,
    )
    .await?;

    tx.commit().await?;

    tracing::info!(
        contract_id = %payload.contract_id,
        tenant_id = %payload.tenant_id,
        obligations = payload.performance_obligations.len(),
        total_price = payload.total_transaction_price_minor,
        "Revrec contract created atomically with outbox"
    );

    Ok(payload.contract_id)
}

/// Fetch a contract by ID
pub async fn get_contract(
    pool: &PgPool,
    contract_id: Uuid,
) -> Result<Option<ContractRow>, RevrecRepoError> {
    let row = sqlx::query_as::<_, ContractRow>(
        "SELECT contract_id, tenant_id, customer_id, contract_name,
                contract_start, contract_end, total_transaction_price_minor,
                currency, external_contract_ref, status, created_at
         FROM revrec_contracts
         WHERE contract_id = $1",
    )
    .bind(contract_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Fetch obligations for a contract
pub async fn get_obligations(
    pool: &PgPool,
    contract_id: Uuid,
) -> Result<Vec<ObligationRow>, RevrecRepoError> {
    let rows = sqlx::query_as::<_, ObligationRow>(
        "SELECT obligation_id, contract_id, tenant_id, name, description,
                allocated_amount_minor, recognition_pattern,
                satisfaction_start, satisfaction_end, status, created_at
         FROM revrec_obligations
         WHERE contract_id = $1
         ORDER BY created_at",
    )
    .bind(contract_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[derive(Debug, sqlx::FromRow)]
pub struct ContractRow {
    pub contract_id: Uuid,
    pub tenant_id: String,
    pub customer_id: String,
    pub contract_name: String,
    pub contract_start: NaiveDate,
    pub contract_end: Option<NaiveDate>,
    pub total_transaction_price_minor: i64,
    pub currency: String,
    pub external_contract_ref: Option<String>,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct ObligationRow {
    pub obligation_id: Uuid,
    pub contract_id: Uuid,
    pub tenant_id: String,
    pub name: String,
    pub description: String,
    pub allocated_amount_minor: i64,
    pub recognition_pattern: serde_json::Value,
    pub satisfaction_start: NaiveDate,
    pub satisfaction_end: Option<NaiveDate>,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

// ============================================================================
// Schedule Persistence
// ============================================================================

/// Create a recognition schedule with lines, atomically with outbox event.
///
/// This function:
/// 1. Validates schedule lines sum == total_to_recognize_minor
/// 2. Determines version (max existing version + 1 for this obligation)
/// 3. Inserts schedule row with version and optional previous_schedule_id
/// 4. Inserts all schedule line rows
/// 5. Inserts revrec.schedule_created into outbox
/// 6. Commits atomically
///
/// Idempotency: if schedule_id already exists, returns DuplicateSchedule error.
pub async fn create_schedule(
    pool: &PgPool,
    event_id: Uuid,
    payload: &ScheduleCreatedPayload,
) -> Result<Uuid, RevrecRepoError> {
    // Validate lines sum invariant
    let lines_sum: i64 = payload
        .lines
        .iter()
        .map(|l| l.amount_to_recognize_minor)
        .sum();
    if lines_sum != payload.total_to_recognize_minor {
        return Err(RevrecRepoError::ScheduleSumMismatch {
            sum: lines_sum,
            expected: payload.total_to_recognize_minor,
        });
    }

    let mut tx = pool.begin().await?;

    // Idempotency check
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM revrec_schedules WHERE schedule_id = $1)",
    )
    .bind(payload.schedule_id)
    .fetch_one(&mut *tx)
    .await?;

    if exists {
        tx.rollback().await?;
        return Err(RevrecRepoError::DuplicateSchedule(payload.schedule_id));
    }

    // Determine version: max existing + 1
    let current_max: Option<i32> = sqlx::query_scalar(
        "SELECT MAX(version) FROM revrec_schedules WHERE obligation_id = $1",
    )
    .bind(payload.obligation_id)
    .fetch_one(&mut *tx)
    .await?;

    let version = current_max.unwrap_or(0) + 1;

    // Find previous schedule_id (the one with the current max version)
    let previous_schedule_id: Option<Uuid> = if version > 1 {
        sqlx::query_scalar(
            "SELECT schedule_id FROM revrec_schedules
             WHERE obligation_id = $1 AND version = $2",
        )
        .bind(payload.obligation_id)
        .bind(version - 1)
        .fetch_optional(&mut *tx)
        .await?
    } else {
        None
    };

    // Insert schedule
    sqlx::query(
        r#"
        INSERT INTO revrec_schedules (
            schedule_id, contract_id, obligation_id, tenant_id,
            total_to_recognize_minor, currency,
            first_period, last_period,
            version, previous_schedule_id,
            created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        "#,
    )
    .bind(payload.schedule_id)
    .bind(payload.contract_id)
    .bind(payload.obligation_id)
    .bind(&payload.tenant_id)
    .bind(payload.total_to_recognize_minor)
    .bind(&payload.currency)
    .bind(&payload.first_period)
    .bind(&payload.last_period)
    .bind(version)
    .bind(previous_schedule_id)
    .bind(payload.created_at)
    .execute(&mut *tx)
    .await?;

    // Insert schedule lines
    for line in &payload.lines {
        sqlx::query(
            r#"
            INSERT INTO revrec_schedule_lines (
                schedule_id, period, amount_to_recognize_minor,
                deferred_revenue_account, recognized_revenue_account
            )
            VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(payload.schedule_id)
        .bind(&line.period)
        .bind(line.amount_to_recognize_minor)
        .bind(&line.deferred_revenue_account)
        .bind(&line.recognized_revenue_account)
        .execute(&mut *tx)
        .await?;
    }

    // Insert outbox event atomically
    let outbox_payload = serde_json::to_value(payload)
        .map_err(|e| RevrecRepoError::Serialization(e.to_string()))?;

    outbox_repo::insert_outbox_event(
        &mut tx,
        event_id,
        EVENT_TYPE_SCHEDULE_CREATED,
        "revrec_schedule",
        &payload.schedule_id.to_string(),
        outbox_payload,
        MUTATION_CLASS_DATA_MUTATION,
    )
    .await?;

    tx.commit().await?;

    tracing::info!(
        schedule_id = %payload.schedule_id,
        obligation_id = %payload.obligation_id,
        version = version,
        lines = payload.lines.len(),
        total = payload.total_to_recognize_minor,
        "Revrec schedule created atomically with outbox (v{})",
        version
    );

    Ok(payload.schedule_id)
}

/// Fetch a schedule by ID
pub async fn get_schedule(
    pool: &PgPool,
    schedule_id: Uuid,
) -> Result<Option<ScheduleRow>, RevrecRepoError> {
    let row = sqlx::query_as::<_, ScheduleRow>(
        "SELECT schedule_id, contract_id, obligation_id, tenant_id,
                total_to_recognize_minor, currency, first_period, last_period,
                version, previous_schedule_id, created_at
         FROM revrec_schedules
         WHERE schedule_id = $1",
    )
    .bind(schedule_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Fetch schedule lines for a schedule
pub async fn get_schedule_lines(
    pool: &PgPool,
    schedule_id: Uuid,
) -> Result<Vec<ScheduleLineRow>, RevrecRepoError> {
    let rows = sqlx::query_as::<_, ScheduleLineRow>(
        "SELECT id, schedule_id, period, amount_to_recognize_minor,
                deferred_revenue_account, recognized_revenue_account,
                recognized, recognized_at
         FROM revrec_schedule_lines
         WHERE schedule_id = $1
         ORDER BY period",
    )
    .bind(schedule_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Get the latest schedule version for an obligation
pub async fn get_latest_schedule_for_obligation(
    pool: &PgPool,
    obligation_id: Uuid,
) -> Result<Option<ScheduleRow>, RevrecRepoError> {
    let row = sqlx::query_as::<_, ScheduleRow>(
        "SELECT schedule_id, contract_id, obligation_id, tenant_id,
                total_to_recognize_minor, currency, first_period, last_period,
                version, previous_schedule_id, created_at
         FROM revrec_schedules
         WHERE obligation_id = $1
         ORDER BY version DESC
         LIMIT 1",
    )
    .bind(obligation_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

#[derive(Debug, sqlx::FromRow)]
pub struct ScheduleRow {
    pub schedule_id: Uuid,
    pub contract_id: Uuid,
    pub obligation_id: Uuid,
    pub tenant_id: String,
    pub total_to_recognize_minor: i64,
    pub currency: String,
    pub first_period: String,
    pub last_period: String,
    pub version: i32,
    pub previous_schedule_id: Option<Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct ScheduleLineRow {
    pub id: i64,
    pub schedule_id: Uuid,
    pub period: String,
    pub amount_to_recognize_minor: i64,
    pub deferred_revenue_account: String,
    pub recognized_revenue_account: String,
    pub recognized: bool,
    pub recognized_at: Option<chrono::DateTime<chrono::Utc>>,
}

// ============================================================================
// Recognition Run Support
// ============================================================================

/// A due schedule line enriched with contract/obligation context for recognition posting.
#[derive(Debug, sqlx::FromRow)]
pub struct DueScheduleLine {
    pub line_id: i64,
    pub schedule_id: Uuid,
    pub contract_id: Uuid,
    pub obligation_id: Uuid,
    pub tenant_id: String,
    pub period: String,
    pub amount_to_recognize_minor: i64,
    pub currency: String,
    pub deferred_revenue_account: String,
    pub recognized_revenue_account: String,
}

/// Find unrecognized schedule lines due for a given period.
///
/// Only returns lines from the **latest** schedule version for each obligation,
/// preventing double-recognition when schedules are re-versioned.
///
/// The query:
/// 1. Finds the max version per obligation_id
/// 2. Joins to schedule_lines WHERE recognized = false AND period = target
/// 3. Returns enriched rows with contract/obligation context
pub async fn find_due_lines_for_period(
    pool: &PgPool,
    tenant_id: &str,
    period: &str,
) -> Result<Vec<DueScheduleLine>, RevrecRepoError> {
    let rows = sqlx::query_as::<_, DueScheduleLine>(
        r#"
        SELECT
            sl.id AS line_id,
            s.schedule_id,
            s.contract_id,
            s.obligation_id,
            s.tenant_id,
            sl.period,
            sl.amount_to_recognize_minor,
            s.currency,
            sl.deferred_revenue_account,
            sl.recognized_revenue_account
        FROM revrec_schedule_lines sl
        JOIN revrec_schedules s ON sl.schedule_id = s.schedule_id
        JOIN (
            SELECT obligation_id, MAX(version) AS max_version
            FROM revrec_schedules
            WHERE tenant_id = $1
            GROUP BY obligation_id
        ) latest ON s.obligation_id = latest.obligation_id AND s.version = latest.max_version
        WHERE s.tenant_id = $1
          AND sl.period = $2
          AND sl.recognized = false
        ORDER BY sl.id
        "#,
    )
    .bind(tenant_id)
    .bind(period)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Mark a schedule line as recognized within an existing transaction.
///
/// Sets `recognized = true` and `recognized_at = NOW()`.
/// Returns the number of rows affected (0 if already recognized — idempotent).
pub async fn mark_line_recognized(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    line_id: i64,
) -> Result<u64, RevrecRepoError> {
    let result = sqlx::query(
        r#"
        UPDATE revrec_schedule_lines
        SET recognized = true, recognized_at = NOW()
        WHERE id = $1 AND recognized = false
        "#,
    )
    .bind(line_id)
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected())
}

/// Get cumulative recognized amount for a schedule up to and including a period.
pub async fn get_cumulative_recognized(
    pool: &PgPool,
    schedule_id: Uuid,
    up_to_period: &str,
) -> Result<i64, RevrecRepoError> {
    let sum: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT SUM(amount_to_recognize_minor)
        FROM revrec_schedule_lines
        WHERE schedule_id = $1
          AND recognized = true
          AND period <= $2
        "#,
    )
    .bind(schedule_id)
    .bind(up_to_period)
    .fetch_one(pool)
    .await?;
    Ok(sum.unwrap_or(0))
}

// ============================================================================
// Amendment functions (Phase 24a — bd-1qi)
// ============================================================================

/// A single row from revrec_contract_modifications.
pub struct ModificationRow {
    pub modification_id: Uuid,
    pub modification_type: String,
    pub effective_date: NaiveDate,
}

/// Persist a contract modification record and emit a contract_modified outbox event.
///
/// Idempotency: modification_id is the PRIMARY KEY; duplicate calls return
/// `RevrecRepoError::DuplicateModification`.
pub async fn create_amendment(
    pool: &PgPool,
    event_id: Uuid,
    payload: &ContractModifiedPayload,
) -> Result<(), RevrecRepoError> {
    let mod_type = serde_json::to_value(&payload.modification_type)
        .map_err(|e| RevrecRepoError::Serialization(e.to_string()))?
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    let mut tx = pool.begin().await?;

    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM revrec_contract_modifications WHERE modification_id = $1)",
    )
    .bind(payload.modification_id)
    .fetch_one(&mut *tx)
    .await?;

    if exists {
        tx.rollback().await?;
        return Err(RevrecRepoError::DuplicateModification(payload.modification_id));
    }

    let effective_date = NaiveDate::parse_from_str(&payload.effective_date, "%Y-%m-%d")
        .map_err(|e| RevrecRepoError::Serialization(e.to_string()))?;

    sqlx::query(
        r#"
        INSERT INTO revrec_contract_modifications (
            modification_id, contract_id, tenant_id, modification_type,
            effective_date, new_transaction_price_minor, reason,
            requires_cumulative_catchup
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(payload.modification_id)
    .bind(payload.contract_id)
    .bind(&payload.tenant_id)
    .bind(&mod_type)
    .bind(effective_date)
    .bind(payload.new_transaction_price_minor)
    .bind(&payload.reason)
    .bind(payload.requires_cumulative_catchup)
    .execute(&mut *tx)
    .await?;

    let outbox_payload = serde_json::to_value(payload)
        .map_err(|e| RevrecRepoError::Serialization(e.to_string()))?;

    outbox_repo::insert_outbox_event(
        &mut tx,
        event_id,
        EVENT_TYPE_CONTRACT_MODIFIED,
        "revrec_contract",
        &payload.contract_id.to_string(),
        outbox_payload,
        MUTATION_CLASS_DATA_MUTATION,
    )
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Return all modifications for a contract, ordered by effective_date ascending.
pub async fn get_modifications_for_contract(
    pool: &PgPool,
    contract_id: Uuid,
) -> Result<Vec<ModificationRow>, RevrecRepoError> {
    let rows = sqlx::query_as::<_, (Uuid, String, NaiveDate)>(
        r#"
        SELECT modification_id, modification_type, effective_date
        FROM revrec_contract_modifications
        WHERE contract_id = $1
        ORDER BY effective_date ASC
        "#,
    )
    .bind(contract_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(modification_id, modification_type, effective_date)| ModificationRow {
        modification_id,
        modification_type,
        effective_date,
    })
    .collect();

    Ok(rows)
}

/// Return the outbox event_id for the revrec.schedule_created event of a schedule.
///
/// Returns `None` if no outbox event was found (e.g., migrated data).
pub async fn find_schedule_outbox_event_id(
    pool: &PgPool,
    schedule_id: Uuid,
) -> Result<Option<Uuid>, RevrecRepoError> {
    let event_id: Option<Uuid> = sqlx::query_scalar(
        r#"
        SELECT event_id FROM events_outbox
        WHERE event_type = $1 AND aggregate_id = $2
        LIMIT 1
        "#,
    )
    .bind(EVENT_TYPE_SCHEDULE_CREATED)
    .bind(schedule_id.to_string())
    .fetch_optional(pool)
    .await?;

    Ok(event_id)
}

/// Like `create_schedule` but sets `supersedes_event_id` in the outbox event.
///
/// Use for amended schedules that replace a prior schedule version.
pub async fn create_schedule_with_supersession(
    pool: &PgPool,
    event_id: Uuid,
    payload: &ScheduleCreatedPayload,
    supersedes_event_id: Option<Uuid>,
) -> Result<Uuid, RevrecRepoError> {
    let lines_sum: i64 = payload.lines.iter().map(|l| l.amount_to_recognize_minor).sum();
    if lines_sum != payload.total_to_recognize_minor {
        return Err(RevrecRepoError::ScheduleSumMismatch {
            sum: lines_sum,
            expected: payload.total_to_recognize_minor,
        });
    }

    let mut tx = pool.begin().await?;

    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM revrec_schedules WHERE schedule_id = $1)",
    )
    .bind(payload.schedule_id)
    .fetch_one(&mut *tx)
    .await?;

    if exists {
        tx.rollback().await?;
        return Err(RevrecRepoError::DuplicateSchedule(payload.schedule_id));
    }

    let current_max: Option<i32> = sqlx::query_scalar(
        "SELECT MAX(version) FROM revrec_schedules WHERE obligation_id = $1",
    )
    .bind(payload.obligation_id)
    .fetch_one(&mut *tx)
    .await?;
    let version = current_max.unwrap_or(0) + 1;

    let previous_schedule_id: Option<Uuid> = if version > 1 {
        sqlx::query_scalar(
            "SELECT schedule_id FROM revrec_schedules WHERE obligation_id = $1 AND version = $2",
        )
        .bind(payload.obligation_id)
        .bind(version - 1)
        .fetch_optional(&mut *tx)
        .await?
    } else {
        None
    };

    sqlx::query(
        r#"
        INSERT INTO revrec_schedules (
            schedule_id, contract_id, obligation_id, tenant_id,
            total_to_recognize_minor, currency, first_period, last_period,
            version, previous_schedule_id, created_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        "#,
    )
    .bind(payload.schedule_id)
    .bind(payload.contract_id)
    .bind(payload.obligation_id)
    .bind(&payload.tenant_id)
    .bind(payload.total_to_recognize_minor)
    .bind(&payload.currency)
    .bind(&payload.first_period)
    .bind(&payload.last_period)
    .bind(version)
    .bind(previous_schedule_id)
    .bind(payload.created_at)
    .execute(&mut *tx)
    .await?;

    for line in &payload.lines {
        sqlx::query(
            r#"
            INSERT INTO revrec_schedule_lines (
                schedule_id, period, amount_to_recognize_minor,
                deferred_revenue_account, recognized_revenue_account
            ) VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(payload.schedule_id)
        .bind(&line.period)
        .bind(line.amount_to_recognize_minor)
        .bind(&line.deferred_revenue_account)
        .bind(&line.recognized_revenue_account)
        .execute(&mut *tx)
        .await?;
    }

    let outbox_payload = serde_json::to_value(payload)
        .map_err(|e| RevrecRepoError::Serialization(e.to_string()))?;

    outbox_repo::insert_outbox_event_with_linkage(
        &mut tx,
        event_id,
        EVENT_TYPE_SCHEDULE_CREATED,
        "revrec_schedule",
        &payload.schedule_id.to_string(),
        outbox_payload,
        None,
        supersedes_event_id,
        MUTATION_CLASS_DATA_MUTATION,
    )
    .await?;

    tx.commit().await?;
    Ok(payload.schedule_id)
}
