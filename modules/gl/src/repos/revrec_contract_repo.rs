//! Repository for revrec contract persistence
//!
//! All writes are atomic with the outbox (transactional outbox pattern).
//! Contract creation persists contract + obligations + outbox event in a single transaction.

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use crate::repos::outbox_repo;
use crate::revrec::{
    ContractCreatedPayload, EVENT_TYPE_CONTRACT_CREATED, MUTATION_CLASS_DATA_MUTATION,
};

use super::revrec_repo::RevrecRepoError;

/// Check if a contract already exists (idempotency check)
pub async fn contract_exists(
    pool: &PgPool,
    tenant_id: &str,
    contract_id: Uuid,
) -> Result<bool, RevrecRepoError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM revrec_contracts WHERE contract_id = $1 AND tenant_id = $2)",
    )
    .bind(contract_id)
    .bind(tenant_id)
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
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM revrec_contracts WHERE contract_id = $1)")
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
    let outbox_payload =
        serde_json::to_value(payload).map_err(|e| RevrecRepoError::Serialization(e.to_string()))?;

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
