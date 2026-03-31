//! Repository for revrec contract amendments and schedule supersession
//!
//! Amendment functions persist contract modifications and amended schedules
//! that replace prior schedule versions.

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use crate::repos::outbox_repo;
use crate::revrec::{
    ContractModifiedPayload, ScheduleCreatedPayload, EVENT_TYPE_CONTRACT_MODIFIED,
    EVENT_TYPE_SCHEDULE_CREATED, MUTATION_CLASS_DATA_MUTATION,
};

use super::revrec_repo::RevrecRepoError;

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
        return Err(RevrecRepoError::DuplicateModification(
            payload.modification_id,
        ));
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

    let outbox_payload =
        serde_json::to_value(payload).map_err(|e| RevrecRepoError::Serialization(e.to_string()))?;

    // Look up the most recent outbox event for this contract to link supersession.
    let prior_event_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT event_id FROM events_outbox
         WHERE aggregate_type = 'revrec_contract' AND aggregate_id = $1
         ORDER BY occurred_at DESC LIMIT 1",
    )
    .bind(payload.contract_id.to_string())
    .fetch_optional(&mut *tx)
    .await?;

    outbox_repo::insert_outbox_event_with_linkage(
        &mut tx,
        event_id,
        EVENT_TYPE_CONTRACT_MODIFIED,
        "revrec_contract",
        &payload.contract_id.to_string(),
        outbox_payload,
        None,
        prior_event_id,
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
    .map(
        |(modification_id, modification_type, effective_date)| ModificationRow {
            modification_id,
            modification_type,
            effective_date,
        },
    )
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

    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM revrec_schedules WHERE schedule_id = $1 AND tenant_id = $2)",
    )
    .bind(payload.schedule_id)
    .bind(&payload.tenant_id)
    .fetch_one(&mut *tx)
    .await?;

    if exists {
        tx.rollback().await?;
        return Err(RevrecRepoError::DuplicateSchedule(payload.schedule_id));
    }

    let current_max: Option<i32> =
        sqlx::query_scalar("SELECT MAX(version) FROM revrec_schedules WHERE obligation_id = $1")
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

    let outbox_payload =
        serde_json::to_value(payload).map_err(|e| RevrecRepoError::Serialization(e.to_string()))?;

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
